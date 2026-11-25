use crate::{
    constants::BUFFER,
    error::{CassiniError, Result, ResultContext, TileId},
    grid::{Compression, Grid, GridExtent, Sample},
    tile::Tile,
};
use log::warn;
use std::path::PathBuf;

/// Writes and returns `<name>_with_buffer.tif` in the tile's render directory:
/// the mosaic of [`mosaic_with_buffer`].
pub fn create_tif_with_buffer<T: Sample>(
    tile: &Tile,
    neighbor_tiles: &[PathBuf],
    tif_filename_without_extension: &str,
    resolution: f64,
) -> Result<Grid<T>> {
    let raster_with_buffer = mosaic_with_buffer(
        tile,
        neighbor_tiles,
        tif_filename_without_extension,
        resolution,
    )?;
    let raster_with_buffer_path = tile.render_dir_path.join(format!(
        "{}_with_buffer.tif",
        tif_filename_without_extension
    ));
    raster_with_buffer
        .write_geotiff(&raster_with_buffer_path, Compression::None)
        .context(format!(
            "could not create `{}` for tile {}",
            raster_with_buffer_path.display(),
            TileId::from(tile)
        ))?;
    Ok(raster_with_buffer)
}

/// The `<name>.tif` rasters of the tile and its neighbors, mosaicked and
/// cropped to the tile extended by `BUFFER` metres, at `resolution`, as `T`.
///
/// This does what `gdalbuildvrt` then `gdal_translate -projwin … -tr … -ot …`
/// did: cells are sampled with nearest neighbor at the output cell centres,
/// a source's nodata cells are transparent, a later source wins where two
/// have data, and cells no source covers are nodata. A missing neighbor
/// raster is skipped with a warning, as `gdalbuildvrt` skips it.
pub fn mosaic_with_buffer<T: Sample>(
    tile: &Tile,
    neighbor_tiles: &[PathBuf],
    tif_filename_without_extension: &str,
    resolution: f64,
) -> Result<Grid<T>> {
    mosaic(
        tile,
        neighbor_tiles,
        tif_filename_without_extension,
        resolution,
    )
    .context(format!(
        "could not mosaic the `{}.tif` rasters for tile {}",
        tif_filename_without_extension,
        TileId::from(tile)
    ))
}

fn mosaic<T: Sample>(
    tile: &Tile,
    neighbor_tiles: &[PathBuf],
    tif_filename_without_extension: &str,
    resolution: f64,
) -> Result<Grid<T>> {
    let tif_filename = format!("{}.tif", tif_filename_without_extension);

    let tile_raster_path = tile.lidar_dir_path.join(&tif_filename);
    let tile_raster = Grid::<T>::read_geotiff(&tile_raster_path)?;
    let nodata = tile_raster
        .nodata
        .ok_or_else(|| CassiniError::InvalidArtifact {
            path: tile_raster_path.clone(),
            message: "expected a nodata value".to_owned(),
        })?;
    let mut raster_with_buffer =
        paste_with_buffer(&TileId::from(tile), &tile_raster, nodata, resolution);
    drop(tile_raster);

    for neighbor_tile in neighbor_tiles {
        let neighbor_raster_path = neighbor_tile.join(&tif_filename);
        if !neighbor_raster_path.exists() {
            warn!(
                "Tile {}. Neighbor raster `{}` does not exist, skipping it",
                TileId::from(tile),
                neighbor_raster_path.display()
            );
            continue;
        }
        let neighbor_raster = Grid::<T>::read_geotiff(&neighbor_raster_path)?;
        paste_nearest(&neighbor_raster, &mut raster_with_buffer);
    }

    Ok(raster_with_buffer)
}

/// The grid of [`mosaic_with_buffer`] for `tile` at `resolution`: the tile
/// and `BUFFER` metres around it.
pub fn buffered_extent(tile: &TileId, resolution: f64) -> GridExtent {
    let buffer = BUFFER as i64;
    let size = |min: i64, max: i64| ((max - min + 2 * buffer) as f64 / resolution).round() as usize;
    GridExtent {
        min_x: (tile.min_x - buffer) as f64,
        max_y: (tile.max_y + buffer) as f64,
        resolution,
        width: size(tile.min_x, tile.max_x),
        height: size(tile.min_y, tile.max_y),
    }
}

/// The tile raster `raster` alone, on the grid of [`mosaic_with_buffer`]
/// with `nodata` as the nodata value: the mosaic of a tile without
/// neighbours, from a raster in memory.
pub fn paste_with_buffer<T: Sample>(
    tile: &TileId,
    raster: &Grid<T>,
    nodata: T,
    resolution: f64,
) -> Grid<T> {
    let extent = buffered_extent(tile, resolution);
    let mut raster_with_buffer = Grid::new(
        extent.min_x,
        extent.max_y,
        extent.resolution,
        extent.width,
        extent.height,
        Some(nodata),
        nodata,
    );
    paste_nearest(raster, &mut raster_with_buffer);
    raster_with_buffer
}

/// Copies the data cells of `source` to the `output` cells whose centre they
/// contain.
fn paste_nearest<T: Sample>(source: &Grid<T>, output: &mut Grid<T>) {
    let cols = overlapping_indices(output.width, source.width, |col| {
        (output.cell_center(col, 0).0 - source.min_x) / source.resolution
    });
    let rows = overlapping_indices(output.height, source.height, |row| {
        (source.max_y - output.cell_center(0, row).1) / source.resolution
    });

    for &(row, source_row) in &rows {
        for &(col, source_col) in &cols {
            let value = source.get(source_col, source_row);
            if source.has_data(value) {
                output.set(col, row, value);
            }
        }
    }
}

/// The (output, source) index pairs along one axis, for the `output_count`
/// output cells whose centre is inside one of the `source_count` source cells.
/// `source_offset` is the distance from the source edge to the centre of an
/// output cell, in source cells.
fn overlapping_indices(
    output_count: usize,
    source_count: usize,
    source_offset: impl Fn(usize) -> f64,
) -> Vec<(usize, usize)> {
    (0..output_count)
        .filter_map(|index| {
            let source_index = source_offset(index).floor();
            (0.0..source_count as f64)
                .contains(&source_index)
                .then_some((index, source_index as usize))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2 x 2 grid of 1 m cells with its north-west corner at (`min_x`,
    /// `max_y`).
    fn source(min_x: f64, max_y: f64, values: [f32; 4]) -> Grid<f32> {
        Grid {
            min_x,
            max_y,
            resolution: 1.0,
            width: 2,
            height: 2,
            nodata: Some(-9999.0),
            values: values.to_vec(),
        }
    }

    #[test]
    fn upsamples_with_nearest_neighbor() {
        let mut output = Grid::new(0.0, 2.0, 0.5, 4, 4, Some(-9999.0), -9999.0f32);
        paste_nearest(&source(0.0, 2.0, [1.0, 2.0, 3.0, 4.0]), &mut output);
        #[rustfmt::skip]
        assert_eq!(output.values, [
            1.0, 1.0, 2.0, 2.0,
            1.0, 1.0, 2.0, 2.0,
            3.0, 3.0, 4.0, 4.0,
            3.0, 3.0, 4.0, 4.0,
        ]);
    }

    #[test]
    fn copies_cells_of_the_same_resolution_one_to_one() {
        let mut source = source(0.0, 1.0, [1.0, 2.0, 3.0, 4.0]);
        source.resolution = 0.5;
        let mut output = Grid::new(-0.5, 1.5, 0.5, 4, 4, Some(-9999.0), -9999.0f32);
        paste_nearest(&source, &mut output);
        let n = -9999.0;
        #[rustfmt::skip]
        assert_eq!(output.values, [
            n, n, n, n,
            n, 1.0, 2.0, n,
            n, 3.0, 4.0, n,
            n, n, n, n,
        ]);
    }

    #[test]
    fn takes_the_last_column_and_row_from_the_neighbors_of_half_cell_offset_sources() {
        // A 2 m tile of 0.5 m cells, and its DEM and those of its east, south
        // and south-east neighbours as IGN's are: 0.25 m west and north of
        // their tiles, so each output cell centre is on a source cell edge.
        let offset_source = |min_x: f64, max_y: f64, first: f32| Grid {
            min_x,
            max_y,
            resolution: 0.5,
            width: 4,
            height: 4,
            nodata: Some(-9999.0),
            values: (0..16).map(|i| first + i as f32).collect(),
        };
        let mut output = Grid::new(0.0, 2.0, 0.5, 4, 4, Some(-9999.0), -9999.0f32);
        for source in [
            offset_source(-0.25, 2.25, 0.0),
            offset_source(1.75, 2.25, 100.0),
            offset_source(-0.25, 0.25, 200.0),
            offset_source(1.75, 0.25, 300.0),
        ] {
            paste_nearest(&source, &mut output);
        }
        // The tie goes to the source cell east and south of the centre: the
        // tile's DEM fills all but the last column and row, shifted by one
        // cell, and the neighbours' first column or row the rest.
        #[rustfmt::skip]
        assert_eq!(output.values, [
            5.0, 6.0, 7.0, 104.0,
            9.0, 10.0, 11.0, 108.0,
            13.0, 14.0, 15.0, 112.0,
            201.0, 202.0, 203.0, 300.0,
        ]);
    }

    #[test]
    fn composites_sources_with_transparent_nodata() {
        let mut output = Grid::new(-1.0, 3.0, 1.0, 4, 4, Some(-9999.0), -9999.0f32);
        paste_nearest(&source(0.0, 2.0, [1.0, -9999.0, 3.0, 4.0]), &mut output);
        // Fills the first source's north-east cell, which has no data, and
        // leaves its south-east cell.
        paste_nearest(&source(1.0, 2.0, [5.0, 6.0, -9999.0, 8.0]), &mut output);
        // Overwrites the first source's south-west cell.
        paste_nearest(
            &source(0.0, 1.0, [7.0, -9999.0, -9999.0, -9999.0]),
            &mut output,
        );
        #[rustfmt::skip]
        assert_eq!(output.values, [
            -9999.0, -9999.0, -9999.0, -9999.0,
            -9999.0, 1.0, 5.0, 6.0,
            -9999.0, 7.0, 4.0, 8.0,
            -9999.0, -9999.0, -9999.0, -9999.0,
        ]);
    }

    #[test]
    fn mosaics_the_tile_with_its_neighbors_skipping_missing_ones() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("cassini-buffer-{unique}"));
        let tile_dir = root.join("tile");
        let east_dir = root.join("east");
        // A neighbor whose LiDAR step left no raster.
        let west_dir = root.join("west");
        for dir in [&tile_dir, &east_dir, &west_dir] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let raster =
            |min_x: f64, value: f32| Grid::new(min_x, 1000.0, 100.0, 10, 10, Some(-9999.0), value);
        raster(0.0, 1.0)
            .write_geotiff(&tile_dir.join("dem.tif"), Compression::None)
            .unwrap();
        raster(1000.0, 2.0)
            .write_geotiff(&east_dir.join("dem.tif"), Compression::None)
            .unwrap();
        let tile = Tile {
            lidar_dir_path: tile_dir.clone(),
            render_dir_path: tile_dir.clone(),
            min_x: 0,
            min_y: 0,
            max_x: 1000,
            max_y: 1000,
        };

        let mosaic =
            create_tif_with_buffer::<f32>(&tile, &[west_dir, east_dir], "dem", 100.0).unwrap();
        let written = Grid::<f32>::read_geotiff(&tile_dir.join("dem_with_buffer.tif")).unwrap();
        std::fs::remove_dir_all(&root).unwrap();

        assert_eq!(written, mosaic);
        assert_eq!((mosaic.width, mosaic.height), (14, 14));
        assert_eq!((mosaic.min_x, mosaic.max_y), (-200.0, 1200.0));
        // The row through the tile: west buffer, tile, east neighbor.
        #[rustfmt::skip]
        assert_eq!(&mosaic.values[5 * 14..6 * 14], [
            -9999.0, -9999.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 2.0, 2.0,
        ]);
        // The north buffer has no data.
        assert!(mosaic.values[..2 * 14]
            .iter()
            .all(|&value| value == -9999.0));
    }
}
