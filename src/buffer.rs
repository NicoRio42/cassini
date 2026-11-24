use crate::{
    error::{CassiniError, Result},
    grid::{Compression, Grid, Sample},
    tile::Tile,
};
use std::path::{Path, PathBuf};

/// Writes `<name>_with_buffer.tif` in the tile's render directory: the
/// `<name>.tif` rasters of the tile and its neighbors, mosaicked and cropped
/// to the tile extended by `buffer` metres, at `resolution`, as `T`.
///
/// This does what `gdalbuildvrt` then `gdal_translate -projwin … -tr … -ot …`
/// did: cells are sampled with nearest neighbor at the output cell centres,
/// a source's nodata cells are transparent, a later source wins where two
/// have data, and cells no source covers are nodata.
pub fn create_tif_with_buffer<T: Sample>(
    tile: &Tile,
    neighbor_tiles: &[PathBuf],
    buffer: i64,
    tif_filename_without_extension: &str,
    resolution: f64,
) -> Result<()> {
    let raster_with_buffer_path = tile.render_dir_path.join(format!(
        "{}_with_buffer.tif",
        tif_filename_without_extension
    ));

    let tile_raster_path = tile
        .lidar_dir_path
        .join(format!("{}.tif", tif_filename_without_extension));
    let mut rasters_paths = vec![tile_raster_path];
    rasters_paths.extend(neighbor_tiles.iter().map(|neighbor_tile| {
        neighbor_tile.join(format!("{}.tif", tif_filename_without_extension))
    }));

    let mut raster_with_buffer: Option<Grid<T>> = None;
    for raster_path in &rasters_paths {
        let source = Grid::<T>::read_geotiff(raster_path)?;
        let output = match &mut raster_with_buffer {
            Some(output) => output,
            None => raster_with_buffer.insert(empty_buffered_grid(
                tile,
                buffer,
                resolution,
                &source,
                raster_path,
            )?),
        };
        paste_nearest(&source, output);
    }

    raster_with_buffer
        .expect("the tile raster is always read")
        .write_geotiff(&raster_with_buffer_path, Compression::None)
}

/// A grid of the buffered tile extent at `resolution`, filled with the nodata
/// value of the tile raster `source`.
fn empty_buffered_grid<T: Sample>(
    tile: &Tile,
    buffer: i64,
    resolution: f64,
    source: &Grid<T>,
    source_path: &Path,
) -> Result<Grid<T>> {
    let nodata = source.nodata.ok_or_else(|| CassiniError::InvalidArtifact {
        path: source_path.to_path_buf(),
        message: "expected a nodata value".to_owned(),
    })?;
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
    // The source cell index along one axis for each output cell, if inside.
    let source_indices =
        |output_count: usize, output_center: &dyn Fn(usize) -> f64, source_count: usize| {
            (0..output_count)
                .map(|index| {
                    let source_index = (output_center(index) / source.resolution).floor();
                    (0.0..source_count as f64)
                        .contains(&source_index)
                        .then_some(source_index as usize)
                })
                .collect::<Vec<_>>()
        };
    let source_cols = source_indices(
        output.width,
        &|col| output.cell_center(col, 0).0 - source.min_x,
        source.width,
    );
    let source_rows = source_indices(
        output.height,
        &|row| source.max_y - output.cell_center(0, row).1,
        source.height,
    );

    for (row, source_row) in source_rows.iter().enumerate() {
        let Some(source_row) = *source_row else {
            continue;
        };
        for (col, source_col) in source_cols.iter().enumerate() {
            let Some(source_col) = *source_col else {
                continue;
            };
            let value = source.get(source_col, source_row);
            if source.has_data(value) {
                output.set(col, row, value);
            }
        }
    }
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
}
