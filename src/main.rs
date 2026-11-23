mod batch;
mod buffer;
mod canvas;
mod cliffs;
mod coastlines;
mod config;
mod constants;
mod contours;
mod dem;
mod download;
mod error;
mod helpers;
mod lidar;
mod map_features;
mod map_renderer;
mod merge;
mod ocad_export;
mod ocd;
mod process;
mod pullautin_contours_render;
mod pullautin_smooth_contours;
mod render;
mod tile;
mod vectors;
mod vegetation;
mod world_file;

use batch::batch;
use clap::{CommandFactory, Parser, Subcommand};
use config::{default_config, get_config};
use download::download_osm_file;
use error::{CassiniError, Result, ResultContext};
use las::raw::Header;
use lidar::generate_dem_and_vegetation_density_tiff_images_from_laz_file;
use log::info;
use ocad_export::check_template;
use ocd::OcdTemplate;
use render::{generate_png_from_dem_vegetation_density_tiff_images_and_vector_file, RenderOptions};
use std::{
    fs::{create_dir_all, File},
    num::NonZeroUsize,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
    time::Instant,
};
use tile::{get_extent_from_lidar_dir_path, Tile};
use vegetation::UndergrowthMode;

// Update the docs when modifying
#[derive(Parser, Debug)]
#[command(
    version,
    about = "A software that generates highly accurate topographic maps from LiDAR data. See documentation: https://cassini-map.com. GDAL and PDAL must be installed on the system for this program to work.",
    long_about = "Cassini is a software that generates highly accurate topographic maps from LiDAR data and shapefile vector data in record times."
)]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Generate a map from a single LiDAR file
    Process {
        #[arg(help = "The path to the LiDAR file to process")]
        file_path: String,

        #[arg(
            long,
            short,
            help = "The output directory for the processed LiDAR file",
            default_value = "tile"
        )]
        output_dir: Option<String>,

        #[arg(long, help = "Skip the vector processing stage of the pipeline")]
        skip_vector: bool,

        #[arg(
            long,
            help = "Prevent the vector renderer to draw the 520 (area that shall not be entered) symbol"
        )]
        skip_520: bool,

        #[arg(
            long,
            help = "Path to a directory containing shapefiles to use instead of downloading from OpenStreetMap"
        )]
        shapefiles: Option<String>,

        #[arg(
            long,
            value_enum,
            help = "Undergrowth rendering mode",
            default_value = "merge"
        )]
        undergrowth: UndergrowthMode,

        #[command(flatten)]
        ocad: OcadArgs,
    },

    /// Run only the LiDAR processing step for a single tile
    Lidar {
        #[arg(help = "The path to the LiDAR file to process")]
        file_path: String,

        #[arg(
            long,
            short,
            help = "The output directory for the processed LiDAR file",
            default_value = "lidar"
        )]
        output_dir: Option<String>,

        #[arg(long, short = 'C', help = "The path to the configuration file to use")]
        config: Option<PathBuf>,
    },

    /// Run only the map generation step for a single tile
    Render {
        #[arg(
            help = "The path to the directory containing the output of the LiDAR processing step"
        )]
        input_dir: String,

        #[arg(long, short = 'C', help = "The path to the configuration file to use")]
        config: Option<PathBuf>,

        #[arg(
            long,
            short,
            help = "The output directory for the processed LiDAR file",
            default_value = "tile"
        )]
        output_dir: Option<String>,

        #[arg(
            long,
            short,
            help = "A list of directories containing the output of the LiDAR processing step for neighboring tiles"
        )]
        neighbors: Vec<String>,

        #[arg(long, help = "Skip the vector processing stage of the pipeline")]
        skip_vector: bool,

        #[arg(
            long,
            help = "Prevent the vector renderer to draw the 520 (area that shall not be entered) symbol"
        )]
        skip_520: bool,

        #[arg(
            long,
            help = "Path to a directory containing shapefiles to use instead of downloading from OpenStreetMap"
        )]
        shapefiles: Option<String>,

        #[arg(
            long,
            value_enum,
            help = "Undergrowth rendering mode",
            default_value = "merge"
        )]
        undergrowth: UndergrowthMode,

        #[command(flatten)]
        ocad: OcadArgs,
    },

    /// Process multiple LiDAR files at once
    Batch {
        #[arg(
            help = "The path to the directory containing the LiDAR files to process",
            default_value = "in"
        )]
        input_dir: Option<String>,

        #[arg(long, short = 'C', help = "The path to the configuration file to use")]
        config: Option<PathBuf>,

        #[arg(
            long,
            short,
            help = "The output directory for the processed LiDAR files",
            default_value = "out"
        )]
        output_dir: Option<String>,

        #[arg(
            long,
            short,
            help = "Number of threads used by Cassini to parallelize the work in batch mode",
            default_value = "3"
        )]
        threads: Option<NonZeroUsize>,

        #[arg(
            long,
            help = "Skip the LiDAR processing stage of the pipeline (only if you already ran cassini once with the same input files)."
        )]
        skip_lidar: bool,

        #[arg(long, help = "Skip the vector processing stage of the pipeline")]
        skip_vector: bool,

        #[arg(
            long,
            help = "Prevent the vector renderer to draw the 520 (area that shall not be entered) symbol"
        )]
        skip_520: bool,

        #[arg(
            long,
            value_enum,
            help = "Undergrowth rendering mode",
            default_value = "merge"
        )]
        undergrowth: UndergrowthMode,

        #[command(flatten)]
        ocad: OcadArgs,
    },

    /// Output a default config.json file.
    Config,
}

#[derive(clap::Args, Debug)]
struct OcadArgs {
    #[arg(long, help = "Also export the map as an OCAD 12 file (full-map.ocd)")]
    ocad: bool,

    #[arg(
        long,
        requires = "ocad",
        help = "Path to an OCAD 12 file whose symbols to use for the OCAD export, instead of the built-in ISOM 2017-2 symbol set"
    )]
    ocad_template: Option<PathBuf>,
}

fn process_single_tile(
    file_path: &Path,
    output_dir_path: &Path,
    options: RenderOptions,
) -> Result<()> {
    let config = get_config(None)?;

    generate_dem_and_vegetation_density_tiff_images_from_laz_file(file_path, output_dir_path)?;

    let mut file = File::open(file_path).context(format!(
        "could not open LiDAR file `{}`",
        file_path.display()
    ))?;
    let header = Header::read_from(&mut file).context(format!(
        "could not read LiDAR header `{}`",
        file_path.display()
    ))?;

    let tile = Tile {
        lidar_dir_path: output_dir_path.to_path_buf(),
        render_dir_path: output_dir_path.to_path_buf(),
        min_x: header.min_x.round() as i64,
        min_y: header.min_y.round() as i64,
        max_x: header.max_x.round() as i64,
        max_y: header.max_y.round() as i64,
    };

    if options.shapefiles_dir.is_none() && !options.skip_vector {
        download_osm_file_if_needed(&tile)?;
    }

    generate_png_from_dem_vegetation_density_tiff_images_and_vector_file(
        tile,
        vec![],
        &options,
        &config,
    )
}

fn process_single_tile_lidar_step(
    file_path: &Path,
    output_dir_path: &Path,
    config_path: Option<&Path>,
) -> Result<()> {
    if config_path.is_some() {
        get_config(config_path)?;
    }

    generate_dem_and_vegetation_density_tiff_images_from_laz_file(file_path, output_dir_path)
}

fn process_single_tile_render_step(
    input_dir_path: &Path,
    output_dir_path: &Path,
    neighbor_tiles: Vec<PathBuf>,
    options: RenderOptions,
    config_path: Option<&Path>,
) -> Result<()> {
    let config = get_config(config_path)?;
    create_dir_all(output_dir_path).context(format!(
        "could not create render output directory `{}`",
        output_dir_path.display()
    ))?;

    let (min_x, min_y, max_x, max_y) = get_extent_from_lidar_dir_path(input_dir_path)?;

    let tile = Tile {
        lidar_dir_path: input_dir_path.to_path_buf(),
        render_dir_path: output_dir_path.to_path_buf(),
        min_x,
        min_y,
        max_x,
        max_y,
    };

    if options.shapefiles_dir.is_none() && !options.skip_vector {
        download_osm_file_if_needed(&tile)?;
    }

    generate_png_from_dem_vegetation_density_tiff_images_and_vector_file(
        tile,
        neighbor_tiles,
        &options,
        &config,
    )
}

/// Loads the OCAD template when the export is requested, before any
/// processing, so that a template lacking symbols fails fast.
fn load_ocad_template(args: &OcadArgs) -> Result<Option<Arc<OcdTemplate>>> {
    if !args.ocad {
        return Ok(None);
    }

    let template = OcdTemplate::load(args.ocad_template.as_deref())?;
    check_template(&template)?;
    Ok(Some(Arc::new(template)))
}

fn download_osm_file_if_needed(tile: &Tile) -> Result<()> {
    let osm_path = tile
        .render_dir_path
        .join(format!("{:0>7}_{:0>7}.osm", tile.min_x, tile.max_y));

    if !osm_path.exists() {
        download_osm_file(
            tile.min_x,
            tile.min_y,
            tile.max_x,
            tile.max_y,
            &tile.render_dir_path,
        )?;
    }
    Ok(())
}

fn batch_process_tiles(
    input_dir: &str,
    output_dir: &str,
    number_of_threads: usize,
    skip_lidar: bool,
    options: RenderOptions,
    config_path: Option<&Path>,
) -> Result<()> {
    let config = get_config(config_path)?;
    batch(
        input_dir,
        output_dir,
        number_of_threads,
        skip_lidar,
        options,
        config,
    )
}

fn run() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format(|buf, record| {
            use std::io::Write;
            let ts = buf.timestamp_seconds();
            let level_style = buf.default_level_style(record.level());

            writeln!(
                buf,
                "[{} {:?} {level_style}{}{level_style:#}] {}",
                ts,
                std::thread::current().id(),
                record.level(),
                record.args()
            )
        })
        .init();

    let args = Args::parse();

    if std::env::args().len() == 1 {
        Args::command()
            .print_help()
            .context("could not print command help")?;
        return Ok(());
    }

    if let Some(command) = args.command {
        match command {
            Commands::Config {} => {
                default_config()?;
            }

            Commands::Process {
                file_path,
                output_dir: maybe_output_dir,
                skip_vector,
                skip_520,
                shapefiles,
                undergrowth,
                ocad,
            } => {
                info!("Tile processing");
                let start = Instant::now();

                let options = RenderOptions {
                    skip_vector,
                    skip_520,
                    undergrowth_mode: undergrowth,
                    shapefiles_dir: shapefiles.map(PathBuf::from),
                    ocad_template: load_ocad_template(&ocad)?,
                };

                let output_dir = maybe_output_dir.unwrap_or("tile".to_owned());
                let laz_path = Path::new(&file_path).to_path_buf();
                let dir_path = Path::new(&output_dir).to_path_buf();
                process_single_tile(&laz_path, &dir_path, options)?;

                let duration = start.elapsed();
                info!("Tile generated in {:.1?}", duration);
            }

            Commands::Lidar {
                file_path,
                output_dir: maybe_output_dir,
                config,
            } => {
                info!("LiDAR processing");
                let start = Instant::now();

                let output_dir = maybe_output_dir.unwrap_or("lidar".to_owned());
                let laz_path = Path::new(&file_path).to_path_buf();
                let dir_path = Path::new(&output_dir).to_path_buf();
                process_single_tile_lidar_step(&laz_path, &dir_path, config.as_deref())?;

                let duration = start.elapsed();
                info!("LiDAR file processed in {:.1?}", duration);
            }

            Commands::Render {
                input_dir,
                config,
                output_dir: maybe_output_dir,
                neighbors,
                skip_vector,
                skip_520,
                shapefiles,
                undergrowth,
                ocad,
            } => {
                info!("Map rendering");
                let start = Instant::now();

                let options = RenderOptions {
                    skip_vector,
                    skip_520,
                    undergrowth_mode: undergrowth,
                    shapefiles_dir: shapefiles.map(PathBuf::from),
                    ocad_template: load_ocad_template(&ocad)?,
                };

                let output_dir = maybe_output_dir.unwrap_or("tile".to_owned());
                let input_dir_path = Path::new(&input_dir).to_path_buf();
                let output_dir_path = Path::new(&output_dir).to_path_buf();

                let mut neighbor_tiles: Vec<PathBuf> = vec![];

                for neighbor in neighbors {
                    let neighbor_path = Path::new(&neighbor).to_path_buf();

                    if !neighbor_path.exists() {
                        return Err(CassiniError::InvalidInput {
                            message: format!("neighbor directory `{neighbor}` does not exist"),
                        });
                    }

                    neighbor_tiles.push(neighbor_path);
                }

                process_single_tile_render_step(
                    &input_dir_path,
                    &output_dir_path,
                    neighbor_tiles,
                    options,
                    config.as_deref(),
                )?;

                let duration = start.elapsed();
                info!("Map rendered in {:.1?}", duration);
            }

            Commands::Batch {
                input_dir: maybe_input_dir,
                config,
                output_dir: maybe_output_dir,
                threads: maybe_threads,
                skip_lidar,
                skip_vector,
                skip_520,
                undergrowth,
                ocad,
            } => {
                info!("Batch processing");
                let start = Instant::now();

                let options = RenderOptions {
                    skip_vector,
                    skip_520,
                    undergrowth_mode: undergrowth,
                    shapefiles_dir: None,
                    ocad_template: load_ocad_template(&ocad)?,
                };

                let input_dir = maybe_input_dir.unwrap_or("in".to_owned());
                let output_dir = maybe_output_dir.unwrap_or("out".to_owned());
                let threads = maybe_threads.map(NonZeroUsize::get).unwrap_or(3);

                batch_process_tiles(
                    &input_dir,
                    &output_dir,
                    threads,
                    skip_lidar,
                    options,
                    config.as_deref(),
                )?;

                let duration = start.elapsed();
                info!("Tiles generated in {:.1?}", duration);
            }
        }
    }

    Ok(())
}

fn report_error(error: &CassiniError) {
    eprintln!("error: {error}");

    if let CassiniError::Batch { failures, .. } = error {
        for (index, failure) in failures.iter().enumerate() {
            eprintln!("  {}. {failure}", index + 1);
            let mut source = std::error::Error::source(failure);
            while let Some(cause) = source {
                eprintln!("     caused by: {cause}");
                source = cause.source();
            }
        }
        return;
    }

    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        eprintln!("  caused by: {cause}");
        source = cause.source();
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            report_error(&error);
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Args, Commands};
    use clap::Parser;
    use std::path::PathBuf;

    fn parsed_config(command: &str, option: &str) -> Option<PathBuf> {
        let args = Args::try_parse_from(["cassini", command, "input", option, "custom.json"])
            .expect("command should accept a config path");

        match args.command.unwrap() {
            Commands::Lidar { config, .. }
            | Commands::Render { config, .. }
            | Commands::Batch { config, .. } => config,
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn config_option_is_available_on_processing_commands() {
        for command in ["lidar", "render", "batch"] {
            assert_eq!(
                parsed_config(command, "--config"),
                Some(PathBuf::from("custom.json"))
            );
            assert_eq!(
                parsed_config(command, "-C"),
                Some(PathBuf::from("custom.json"))
            );
        }
    }

    #[test]
    fn ocad_template_option_requires_ocad() {
        for command in ["process", "render", "batch"] {
            assert!(Args::try_parse_from(["cassini", command, "input", "--ocad"]).is_ok());
            assert!(Args::try_parse_from([
                "cassini",
                command,
                "input",
                "--ocad",
                "--ocad-template",
                "symbols.ocd"
            ])
            .is_ok());
            assert!(Args::try_parse_from([
                "cassini",
                command,
                "input",
                "--ocad-template",
                "symbols.ocd"
            ])
            .is_err());
        }
    }

    #[test]
    fn lossy_option_is_rejected() {
        for command in ["process", "render", "batch"] {
            assert!(Args::try_parse_from(["cassini", command, "input", "--lossy"]).is_err());
        }
    }
}
