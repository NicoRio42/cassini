mod batch;
mod buffer;
mod canvas;
mod cliff_features;
mod cliffs;
mod coastlines;
mod config;
mod constants;
mod contour_shapefile;
mod contours;
mod dem;
mod download;
mod external_dem;
mod fill_nodata;
mod helpers;
mod lidar;
mod map_features;
mod map_renderer;
mod marching_squares;
mod merge;
mod ocad_export;
mod ocad_merge;
mod ocd;
mod point_reader;
mod pullautin_contours_render;
mod pullautin_smooth_contours;
mod render;
mod tile;
mod ui;
mod update;
mod vectorization;
mod vectors;
mod vegetation;
mod vegetation_areas;
mod world_file;

use batch::batch;
use cassini::{error, grid, projection};
use clap::{CommandFactory, Parser, Subcommand};
use config::{default_config, get_config, Config};
use dem::create_dem_with_buffer;
use download::download_osm_file;
use error::{CassiniError, Result, ResultContext};
use lidar::{generate_dem_and_vegetation_density_tiff_images_from_laz_file, VegetationLayers};
use log::info;
use ocad_export::{check_template, write_ocd_file};
use ocd::OcdTemplate;
use render::{render_tile_map, OcadOptions, RenderOptions};
use std::{
    fs::create_dir_all,
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
    about = "A software that generates highly accurate topographic maps from LiDAR data. See documentation: https://cassini-map.com.",
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
            value_name = "FILE",
            help = "A GeoTIFF DEM in Lambert-93 to use instead of computing the DEM from the ground points, such as an IGN MNT LiDAR HD tile"
        )]
        dem: Option<PathBuf>,

        #[command(flatten)]
        vegetation: VegetationArgs,

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
            value_name = "FILE",
            help = "A GeoTIFF DEM in Lambert-93 to use instead of computing the DEM from the ground points, such as an IGN MNT LiDAR HD tile. The LiDAR file is then read for the vegetation only"
        )]
        dem: Option<PathBuf>,

        #[command(flatten)]
        vegetation: VegetationArgs,

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
            help = "Number of tiles processed in parallel in batch mode. LiDAR decompression also uses every CPU core, whatever this number",
            default_value = "3"
        )]
        threads: Option<NonZeroUsize>,

        #[arg(
            long,
            conflicts_with = "vegetation_layers",
            help = "Skip the LiDAR processing stage of the pipeline (only if you already ran cassini once with the same input files). The vegetation layers are those of that run"
        )]
        skip_lidar: bool,

        #[command(flatten)]
        vegetation: VegetationArgs,

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

    /// Open a web page to select IGN LiDAR HD tiles on a map, download them
    /// with their DEMs into `in`, and process them into `out`
    Ui {
        #[arg(
            long,
            help = "The port of the local web server, 0 for any free port",
            default_value_t = 0
        )]
        port: u16,

        #[arg(long, help = "Don't open the web page in the default browser")]
        no_open: bool,
    },

    /// Upgrade Cassini to the latest version
    #[command(alias = "update")]
    Upgrade,
}

#[derive(clap::Args, Debug)]
struct VegetationArgs {
    #[arg(
        long,
        value_enum,
        help = "How the vegetation points are sorted into the low, medium and high vegetation layers: by height above the ground, or by LiDAR classification",
        default_value = "height"
    )]
    vegetation_layers: VegetationLayers,
}

#[derive(clap::Args, Debug)]
struct OcadArgs {
    #[arg(
        long,
        help = "Also export the map as an OCAD 12 file (full-map.ocd, or merged-map.ocd for a batch)"
    )]
    ocad: bool,

    #[arg(
        long,
        requires = "ocad",
        help = "Path to an OCAD 12 file whose symbols to use for the OCAD export, instead of the built-in ISOM 2017-2 symbol set"
    )]
    ocad_template: Option<PathBuf>,

    #[arg(
        long,
        requires = "ocad",
        default_value_t = DEFAULT_OCAD_SIMPLIFY_TOLERANCE,
        value_parser = parse_simplify_tolerance,
        help = "Tolerance in meters for simplifying the vegetation areas and cliffs of the OCAD export, 0 to keep every traced vertex"
    )]
    ocad_simplify: f64,
}

const DEFAULT_OCAD_SIMPLIFY_TOLERANCE: f64 = 0.5; // Update the docs when modifying

fn parse_simplify_tolerance(value: &str) -> std::result::Result<f64, String> {
    match value.parse::<f64>() {
        Ok(tolerance) if tolerance.is_finite() && tolerance >= 0.0 => Ok(tolerance),
        _ => Err("must be a finite, non-negative number of meters".to_owned()),
    }
}

fn process_single_tile(
    file_path: &Path,
    dem_path: Option<&Path>,
    vegetation_layers: VegetationLayers,
    output_dir_path: &Path,
    options: RenderOptions,
) -> Result<()> {
    let config = get_config(None)?;

    let extent = generate_dem_and_vegetation_density_tiff_images_from_laz_file(
        file_path,
        dem_path,
        output_dir_path,
        vegetation_layers,
        true,
    )?;
    let tile = Tile {
        lidar_dir_path: output_dir_path.to_path_buf(),
        render_dir_path: output_dir_path.to_path_buf(),
        min_x: extent.min_x,
        min_y: extent.min_y,
        max_x: extent.max_x,
        max_y: extent.max_y,
    };

    if options.shapefiles_dir.is_none() && !options.skip_vector {
        download_osm_file_if_needed(&tile)?;
    }

    render_and_export_tile(tile, vec![], &options, &config)
}

fn process_single_tile_lidar_step(
    file_path: &Path,
    dem_path: Option<&Path>,
    vegetation_layers: VegetationLayers,
    output_dir_path: &Path,
    config_path: Option<&Path>,
) -> Result<()> {
    if config_path.is_some() {
        get_config(config_path)?;
    }

    generate_dem_and_vegetation_density_tiff_images_from_laz_file(
        file_path,
        dem_path,
        output_dir_path,
        vegetation_layers,
        false,
    )
    .map(|_| ())
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

    create_dem_with_buffer(&tile, &neighbor_tiles)?;
    render_and_export_tile(tile, neighbor_tiles, &options, &config)
}

/// Renders a single tile from its `dem_with_buffer.tif`, and exports it to
/// `full-map.ocd` when requested.
fn render_and_export_tile(
    tile: Tile,
    neighbor_tiles: Vec<PathBuf>,
    options: &RenderOptions,
    config: &Config,
) -> Result<()> {
    let features = render_tile_map(tile.clone(), neighbor_tiles, options, config)?;

    match (&options.ocad, features) {
        (Some(ocad), Some(features)) => write_ocd_file(&tile, &ocad.template, &features),
        _ => Ok(()),
    }
}

/// Loads the OCAD template when the export is requested, before any
/// processing, so that a template lacking symbols fails fast.
fn load_ocad_options(args: &OcadArgs) -> Result<Option<OcadOptions>> {
    if !args.ocad {
        return Ok(None);
    }

    let template = OcdTemplate::load(args.ocad_template.as_deref())?;
    check_template(&template)?;
    Ok(Some(OcadOptions {
        template: Arc::new(template),
        simplify_tolerance: args.ocad_simplify,
    }))
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
    vegetation_layers: VegetationLayers,
    options: RenderOptions,
    config_path: Option<&Path>,
) -> Result<()> {
    let config = get_config(config_path)?;
    batch(
        input_dir,
        output_dir,
        number_of_threads,
        skip_lidar,
        vegetation_layers,
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

    update::remove_previous_executable();

    let mut update_check = match args.command {
        Some(Commands::Upgrade) | None => None,
        Some(_) => update::start_update_check(),
    };

    if let Some(command) = args.command {
        match command {
            Commands::Config {} => {
                default_config()?;
            }

            Commands::Ui { port, no_open } => {
                if let Some(check) = update_check.take() {
                    check.print_notice_when_done();
                }
                ui::run_ui(port, !no_open)?;
            }

            Commands::Upgrade => {
                update::upgrade()?;
            }

            Commands::Process {
                file_path,
                dem,
                vegetation: VegetationArgs { vegetation_layers },
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
                    ocad: load_ocad_options(&ocad)?,
                };

                let output_dir = maybe_output_dir.unwrap_or("tile".to_owned());
                let laz_path = Path::new(&file_path).to_path_buf();
                let dir_path = Path::new(&output_dir).to_path_buf();
                process_single_tile(
                    &laz_path,
                    dem.as_deref(),
                    vegetation_layers,
                    &dir_path,
                    options,
                )?;

                let duration = start.elapsed();
                info!("Tile generated in {:.1?}", duration);
            }

            Commands::Lidar {
                file_path,
                dem,
                vegetation: VegetationArgs { vegetation_layers },
                output_dir: maybe_output_dir,
                config,
            } => {
                info!("LiDAR processing");
                let start = Instant::now();

                let output_dir = maybe_output_dir.unwrap_or("lidar".to_owned());
                let laz_path = Path::new(&file_path).to_path_buf();
                let dir_path = Path::new(&output_dir).to_path_buf();
                process_single_tile_lidar_step(
                    &laz_path,
                    dem.as_deref(),
                    vegetation_layers,
                    &dir_path,
                    config.as_deref(),
                )?;

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
                    ocad: load_ocad_options(&ocad)?,
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
                vegetation: VegetationArgs { vegetation_layers },
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
                    ocad: load_ocad_options(&ocad)?,
                };

                let input_dir = maybe_input_dir.unwrap_or("in".to_owned());
                let output_dir = maybe_output_dir.unwrap_or("out".to_owned());
                let threads = maybe_threads.map(NonZeroUsize::get).unwrap_or(3);

                batch_process_tiles(
                    &input_dir,
                    &output_dir,
                    threads,
                    skip_lidar,
                    vegetation_layers,
                    options,
                    config.as_deref(),
                )?;

                let duration = start.elapsed();
                info!("Tiles generated in {:.1?}", duration);
            }
        }
    }

    if let Some(check) = update_check {
        check.print_notice();
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
    use super::{Args, Commands, VegetationLayers, DEFAULT_OCAD_SIMPLIFY_TOLERANCE};
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
    fn dem_option_is_optional_for_lidar_and_process() {
        let dem = |command: &str, options: &[&str]| {
            let args = ["cassini", command, "input"]
                .into_iter()
                .chain(options.iter().copied());
            match Args::try_parse_from(args).unwrap().command.unwrap() {
                Commands::Lidar { dem, .. } | Commands::Process { dem, .. } => dem,
                _ => panic!("unexpected command"),
            }
        };
        let dem_file = Some(PathBuf::from("dem.tif"));
        assert_eq!(dem("lidar", &["--dem", "dem.tif"]), dem_file);
        assert_eq!(dem("process", &["--dem", "dem.tif"]), dem_file);
        assert_eq!(dem("lidar", &[]), None);
        assert_eq!(dem("process", &[]), None);
        assert!(Args::try_parse_from(["cassini", "render", "input", "--dem", "dem.tif"]).is_err());
    }

    #[test]
    fn vegetation_layers_option_defaults_to_height() {
        let vegetation_layers = |command: &str, options: &[&str]| {
            let args = ["cassini", command, "input"]
                .into_iter()
                .chain(options.iter().copied());
            match Args::try_parse_from(args).unwrap().command.unwrap() {
                Commands::Lidar { vegetation, .. }
                | Commands::Process { vegetation, .. }
                | Commands::Batch { vegetation, .. } => vegetation.vegetation_layers,
                _ => panic!("unexpected command"),
            }
        };
        for command in ["lidar", "process", "batch"] {
            assert_eq!(vegetation_layers(command, &[]), VegetationLayers::Height);
            assert_eq!(
                vegetation_layers(command, &["--vegetation-layers", "lidar-class"]),
                VegetationLayers::LidarClass
            );
            assert!(Args::try_parse_from([
                "cassini",
                command,
                "input",
                "--vegetation-layers",
                "class"
            ])
            .is_err());
        }
        assert!(Args::try_parse_from([
            "cassini",
            "render",
            "input",
            "--vegetation-layers",
            "height"
        ])
        .is_err());
    }

    #[test]
    fn skip_lidar_reuses_the_vegetation_layers_of_the_previous_run() {
        assert!(Args::try_parse_from(["cassini", "batch", "--skip-lidar"]).is_ok());
        for layers in ["height", "lidar-class"] {
            assert!(Args::try_parse_from([
                "cassini",
                "batch",
                "--skip-lidar",
                "--vegetation-layers",
                layers
            ])
            .is_err());
        }
    }

    #[test]
    fn vegetation_layers_names_are_the_command_line_values() {
        use clap::ValueEnum;
        for layers in VegetationLayers::value_variants() {
            assert_eq!(
                VegetationLayers::from_str(layers.name(), false),
                Ok(*layers)
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

    fn parsed_ocad_simplify(options: &[&str]) -> Result<f64, clap::Error> {
        let args = Args::try_parse_from(["cassini", "render", "input"].iter().chain(options))?;

        match args.command.unwrap() {
            Commands::Render { ocad, .. } => Ok(ocad.ocad_simplify),
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn ocad_simplify_option_defaults_and_validates() {
        assert_eq!(
            parsed_ocad_simplify(&["--ocad"]).unwrap(),
            DEFAULT_OCAD_SIMPLIFY_TOLERANCE
        );
        assert_eq!(
            parsed_ocad_simplify(&["--ocad", "--ocad-simplify", "1.5"]).unwrap(),
            1.5
        );
        assert_eq!(
            parsed_ocad_simplify(&["--ocad", "--ocad-simplify", "0"]).unwrap(),
            0.0
        );
        assert!(parsed_ocad_simplify(&["--ocad-simplify", "1"]).is_err());
        assert!(parsed_ocad_simplify(&["--ocad", "--ocad-simplify=-1"]).is_err());
        assert!(parsed_ocad_simplify(&["--ocad", "--ocad-simplify", "NaN"]).is_err());
    }

    #[test]
    fn lossy_option_is_rejected() {
        for command in ["process", "render", "batch"] {
            assert!(Args::try_parse_from(["cassini", command, "input", "--lossy"]).is_err());
        }
    }
}
