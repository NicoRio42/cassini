use cassini::{
    batch_process_tiles, generate_default_config, new_process_single_tile_lidar_step, process_single_tile,
    process_single_tile_lidar_step, process_single_tile_render_step,
};
use clap::{CommandFactory, Parser, Subcommand};
use log::info;
use std::{
    path::{Path, PathBuf},
    time::Instant,
};

// Update the docs when modifying
#[derive(Parser, Debug)]
#[command(
    version,
    about = "A software that generates highly accurate topographic maps from LiDAR data. See documentation: https://cassini-map.com. GDAL and PDAL must be installed on the system for this program to work.",
    long_about = "Cassini is a software that generates highly accurate topographic maps from LiDAR data and shapefile vector data in record times."
)]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
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
    },

    /// Run only the map generation step for a single tile
    Render {
        #[arg(help = "The path to the directory containing the output of the LiDAR processing step")]
        input_dir: String,

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
    },

    /// Process multiple LiDAR files at once
    Batch {
        #[arg(
            help = "The path to the directory containing the LiDAR files to process",
            default_value = "in"
        )]
        input_dir: Option<String>,

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
        threads: Option<usize>,

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
    },

    /// Output a default config.json file.
    Config,
}

fn main() {
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
        Args::command().print_help().unwrap();
        return;
    }

    if let Some(command) = args.command {
        match command {
            Commands::Config {} => {
                generate_default_config();
            }

            Commands::Process {
                file_path,
                output_dir: maybe_output_dir,
                skip_vector,
                skip_520,
            } => {
                info!("Tile processing");
                let start = Instant::now();

                let output_dir = maybe_output_dir.unwrap_or("tile".to_owned());
                let laz_path = Path::new(&file_path).to_path_buf();
                let dir_path = Path::new(&output_dir).to_path_buf();
                process_single_tile(&laz_path, &dir_path, skip_vector, skip_520);

                let duration = start.elapsed();
                info!("Tile generated in {:.1?}", duration);
            }

            Commands::Lidar {
                file_path,
                output_dir: maybe_output_dir,
            } => {
                info!("LiDAR processing");
                let start = Instant::now();

                let output_dir = maybe_output_dir.unwrap_or("lidar".to_owned());
                let laz_path = Path::new(&file_path).to_path_buf();
                let dir_path = Path::new(&output_dir).to_path_buf();
                new_process_single_tile_lidar_step(&laz_path, &dir_path);

                let duration = start.elapsed();
                info!("LiDAR file processed in {:.1?}", duration);
            }

            Commands::Render {
                input_dir,
                output_dir: maybe_output_dir,
                neighbors,
                skip_vector,
                skip_520,
            } => {
                info!("Map rendering");
                let start = Instant::now();

                let output_dir = maybe_output_dir.unwrap_or("tile".to_owned());
                let input_dir_path = Path::new(&input_dir).to_path_buf();
                let output_dir_path = Path::new(&output_dir).to_path_buf();

                let mut neighbor_tiles: Vec<PathBuf> = vec![];

                for neighbor in neighbors {
                    let neighbor_path = Path::new(&neighbor).to_path_buf();

                    if !neighbor_path.exists() {
                        panic!("{} does not exist", neighbor)
                    }

                    neighbor_tiles.push(neighbor_path);
                }

                process_single_tile_render_step(
                    &input_dir_path,
                    &output_dir_path,
                    neighbor_tiles,
                    skip_vector,
                    skip_520,
                );

                let duration = start.elapsed();
                info!("Map rendered in {:.1?}", duration);
            }

            Commands::Batch {
                input_dir: maybe_input_dir,
                output_dir: maybe_output_dir,
                threads: maybe_threads,
                skip_lidar,
                skip_vector,
                skip_520,
            } => {
                info!("Batch processing");
                let start = Instant::now();

                let input_dir = maybe_input_dir.unwrap_or("in".to_owned());
                let output_dir = maybe_output_dir.unwrap_or("out".to_owned());
                let threads = maybe_threads.unwrap_or(3);
                batch_process_tiles(
                    &input_dir,
                    &output_dir,
                    threads,
                    skip_lidar,
                    skip_vector,
                    skip_520,
                );

                let duration = start.elapsed();
                info!("Tiles generated in {:.1?}", duration);
            }
        }
    }
}
