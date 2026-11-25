use crate::config::Config;
use crate::constants::INCH;
use crate::contours::generate_contours_with_pullautin_algorithme;
use crate::error::{CassiniError, Result, ResultContext, TileId};
use crate::files::{remove_artifacts, remove_shapefile_artifacts};
use crate::grid::Grid;
use crate::map_features::MapFeature;
use crate::ocad_export::{SPILLED_OBJECTS_FILE_NAME, TILE_OCD_FILE_NAME};
use crate::ocd::OcdTemplate;
use crate::vectors::{
    get_map_features_from_osm_file, get_map_features_from_osm_shapefiles,
    render_map_with_osm_vector_shapes,
};
use crate::vegetation_areas::get_vegetation_features;
use crate::world_file::create_world_file;
use crate::{
    cliff_features::get_cliff_features,
    cliffs::render_cliffs,
    dem::{create_contours_and_slopes, read_dem_with_buffer},
    tile::Tile,
    vegetation::{render_vegetation, UndergrowthMode},
};
use log::info;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

/// Options of the render step, shared by the `process`, `render` and `batch`
/// commands.
#[derive(Clone)]
pub struct RenderOptions {
    pub skip_vector: bool,
    pub skip_520: bool,
    pub undergrowth_mode: UndergrowthMode,
    /// Shapefiles to use instead of the tile's OpenStreetMap data.
    pub shapefiles_dir: Option<PathBuf>,
    /// Also export the map to OCAD.
    pub ocad: Option<OcadOptions>,
}

#[derive(Clone)]
pub struct OcadOptions {
    pub template: Arc<OcdTemplate>,
    /// Douglas–Peucker tolerance in metres for the vegetation areas and the
    /// cliffs, 0 to keep every traced vertex.
    pub simplify_tolerance: f64,
}

/// Renders the tile to `full-map.png` from its filled DEM with buffer
/// (`dem_with_buffer.tif`, see [`crate::dem::DEM_WITH_BUFFER_FILE_NAME`]),
/// vegetation density rasters and vector data. Returns the map features to
/// export when the OCAD export is requested.
pub fn render_tile_map(
    tile: Tile,
    neighbor_tiles: Vec<PathBuf>,
    options: &RenderOptions,
    config: &Config,
) -> Result<Option<Vec<MapFeature>>> {
    let width = tile.max_x - tile.min_x;
    let height = tile.max_y - tile.min_y;
    if width <= 0 || height <= 0 {
        return Err(CassiniError::InvalidInput {
            message: format!("tile {} has a non-positive extent", TileId::from(&tile)),
        });
    }

    let image_width = (width as f32 * config.dpi_resolution / INCH) as u32;
    let image_height = (height as f32 * config.dpi_resolution / INCH) as u32;

    // Nothing of a previous render in the same directory may be left, not
    // even an optional layer that this run does not write.
    remove_render_step_files(&tile.render_dir_path)?;
    let undergrowth_path = tile.render_dir_path.join("undergrowth.png");

    let vegetation_rasters = render_vegetation(
        &tile,
        &neighbor_tiles,
        image_width,
        image_height,
        config,
        &options.undergrowth_mode,
    )?;
    let vegetation_features = match &options.ocad {
        Some(ocad) => get_vegetation_features(
            &tile,
            &vegetation_rasters,
            config,
            &options.undergrowth_mode,
            ocad.simplify_tolerance,
        )?,
        None => vec![],
    };
    drop(vegetation_rasters);

    let dem_with_buffer = read_dem_with_buffer(&tile)?;
    let raw_contours = create_contours_and_slopes(&tile, &dem_with_buffer)?;
    let slopes = Grid::<f32>::read_geotiff(&tile.render_dir_path.join("slopes.tif"))?;
    render_cliffs(&tile, &slopes, image_width, image_height, config)?;
    let cliff_features = match &options.ocad {
        Some(ocad) => get_cliff_features(
            &tile,
            &slopes,
            &dem_with_buffer,
            config,
            ocad.simplify_tolerance,
        )?,
        None => vec![],
    };
    drop(slopes);
    drop(dem_with_buffer);
    let contour_features = generate_contours_with_pullautin_algorithme(
        &tile,
        raw_contours,
        image_width,
        image_height,
        config,
    )?;

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Rendering map to PNG",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y
    );

    let start = Instant::now();

    let cliffs_path = tile.render_dir_path.join("cliffs.png");
    let vegetation_path = tile.render_dir_path.join("vegetation.png");
    let contours_path = tile.render_dir_path.join("contours.png");

    let osm_features = if let Some(shapefiles_dir) = &options.shapefiles_dir {
        get_map_features_from_osm_shapefiles(&tile, shapefiles_dir, options.skip_520)?
    } else if !options.skip_vector {
        let osm_path = tile
            .render_dir_path
            .join(format!("{:0>7}_{:0>7}.osm", tile.min_x, tile.max_y));
        get_map_features_from_osm_file(&tile, &osm_path, options.skip_520)?
    } else {
        vec![]
    };

    render_map_with_osm_vector_shapes(
        &tile,
        image_width,
        image_height,
        config,
        &vegetation_path,
        &undergrowth_path,
        &contours_path,
        &cliffs_path,
        &osm_features,
    )?;

    let resolution = INCH / (config.dpi_resolution);
    let world_file_path = tile.render_dir_path.join("full-map.pgw");

    create_world_file(
        tile.min_x as f32,
        tile.max_y as f32,
        resolution,
        &world_file_path,
    )
    .context(format!("could not create `{}`", world_file_path.display()))?;

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Map rendered to PNG in {:.1?}",
        tile.min_x, tile.min_y, tile.max_x, tile.max_y, duration
    );

    if options.ocad.is_none() {
        return Ok(None);
    }

    let mut features = vegetation_features;
    features.extend(cliff_features);
    features.extend(contour_features);
    features.extend(osm_features);
    Ok(Some(features))
}

/// The files the render step writes. `dem_with_buffer.tif` is not one of
/// them: the render step reads it, and `--skip-lidar` reuses it.
pub const RENDER_STEP_FILES: [&str; 13] = [
    "cliffs.png",
    "contours.png",
    "dem_2m_with_buffer.tif",
    TILE_OCD_FILE_NAME,
    "full-map.pgw",
    "full-map.png",
    "high_vegetation_with_buffer.tif",
    "low_vegetation_with_buffer.tif",
    "medium_vegetation_with_buffer.tif",
    SPILLED_OBJECTS_FILE_NAME,
    "slopes.tif",
    "undergrowth.png",
    "vegetation.png",
];

/// The shapefiles the render step writes, without extension. Their
/// directories can hold other files, which are kept.
const RENDER_STEP_SHAPEFILES: [&str; 3] = [
    "contours/contours",
    "contours-raw/contours-raw",
    "formlines/formlines",
];

/// Removes the files the render step writes from `dir_path`, leaving any
/// other file in place.
pub fn remove_render_step_files(dir_path: &Path) -> Result<()> {
    remove_artifacts(dir_path, &RENDER_STEP_FILES)?;
    remove_shapefile_artifacts(dir_path, &RENDER_STEP_SHAPEFILES)
}

#[cfg(test)]
mod tests {
    use super::{
        remove_render_step_files, render_tile_map, RenderOptions, RENDER_STEP_FILES,
        RENDER_STEP_SHAPEFILES,
    };
    use crate::{
        config::Config,
        lidar::{
            generate_dem_and_vegetation_density_tiff_images_from_laz_file,
            test_tiles::{write_tile, POINTS},
            VegetationLayers,
        },
        tile::Tile,
        vegetation::UndergrowthMode,
    };
    use std::{collections::BTreeSet, path::Path};

    /// The relative paths of the entries of `dir`, recursively.
    fn entries(dir: &Path) -> BTreeSet<String> {
        let mut paths = BTreeSet::new();
        let mut dirs = vec![dir.to_path_buf()];
        while let Some(current) = dirs.pop() {
            for entry in std::fs::read_dir(&current).unwrap() {
                let path = entry.unwrap().path();
                let relative = path.strip_prefix(dir).unwrap();
                paths.insert(relative.to_str().unwrap().replace('\\', "/"));
                if path.is_dir() {
                    dirs.push(path);
                }
            }
        }
        paths
    }

    #[test]
    fn the_render_step_replaces_only_its_own_files() {
        let dir = tempfile::tempdir().unwrap();
        let laz_path = dir.path().join("tile.laz");
        write_tile(&laz_path, 0.0, 100.0, &POINTS);
        let tile_id = generate_dem_and_vegetation_density_tiff_images_from_laz_file(
            &laz_path,
            None,
            dir.path(),
            VegetationLayers::Height,
            true,
        )
        .unwrap();
        // Files of the user, in directories named as those of the render step.
        for name in [
            "contours/notes.txt",
            "formlines/formlines.qml",
            "shapes/roads.shp",
        ] {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, name).unwrap();
        }
        let before = entries(dir.path());
        let tile = Tile {
            lidar_dir_path: dir.path().to_path_buf(),
            render_dir_path: dir.path().to_path_buf(),
            min_x: tile_id.min_x,
            min_y: tile_id.min_y,
            max_x: tile_id.max_x,
            max_y: tile_id.max_y,
        };
        let options = RenderOptions {
            skip_vector: true,
            skip_520: false,
            undergrowth_mode: UndergrowthMode::Symbol406,
            shapefiles_dir: None,
            ocad: None,
        };

        // The second render replaces the output of the first.
        for _ in 0..2 {
            render_tile_map(tile.clone(), vec![], &options, &Config::default()).unwrap();
        }

        let after = entries(dir.path());
        let written: Vec<&String> = after.difference(&before).collect();
        assert!(written.contains(&&"full-map.png".to_owned()));
        for path in &written {
            let is_listed = RENDER_STEP_FILES.contains(&path.as_str())
                || RENDER_STEP_SHAPEFILES.iter().any(|shapefile| {
                    path.strip_prefix(shapefile)
                        .is_some_and(|extension| extension.starts_with('.'))
                        || shapefile.starts_with(&format!("{path}/"))
                });
            assert!(is_listed, "`{path}` is not a render step file");
        }
        remove_render_step_files(dir.path()).unwrap();
        assert_eq!(entries(dir.path()), before);
    }
}
