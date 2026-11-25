use crate::{
    constants::BUFFER,
    error::{CassiniError, Result, ResultContext, TileId},
    grid::{Compression, Grid, Sample},
    tile::Tile,
};
use log::warn;
use std::path::{Path, PathBuf};

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
    let mut raster_with_buffer =
        empty_buffered_grid(tile, resolution, &tile_raster, &tile_raster_path)?;
    paste_nearest(&tile_raster, &mut raster_with_buffer);
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

/// A grid of the buffered tile extent at `resolution`, filled with the nodata
/// value of the tile raster `source`.
fn empty_buffered_grid<T: Sample>(
    tile: &Tile,
    resolution: f64,
    source: &Grid<T>,
    source_path: &Path,
) -> Result<Grid<T>> {
    let nodata = source.nodata.ok_or_else(|| CassiniError::InvalidArtifact {
        path: source_path.to_path_buf(),
        message: "expected a nodata value".to_owned(),
    })?;
    let buffer = BUFFER as i64;
    let size = |min: i64, max: i64| ((max - min + 2 * buffer) as f64 / resolution).round() as usize;
    Ok(Grid::new(
        (tile.min_x - buffer) as f64,
        (tile.max_y + buffer) as f64,
        resolution,
        size(tile.min_x, tile.max_x),
        size(tile.min_y, tile.max_y),
        Some(nodata),
        nodata,
    ))
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
