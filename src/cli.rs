use clap::{Parser, Subcommand};

// Update the docs when modifying
#[derive(Parser, Debug)]
#[command(
    version,
    about = "A software that generates highly accurate topographic maps from LiDAR data. See documentation: https://cassini-map.com",
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
        output_dir: String,

        #[arg(long, short, help = "Skip the vector processing stage of the pipeline")]
        skip_vector: bool,
    },

    /// Run only the LiDAR processing step for a single tile
    Lidar {
        #[arg(help = "The path to the LiDAR file to process")]
        file_path: String,

        #[arg(
            long,
            short,
            help = "The output directory for the processed LiDAR file",
            default_value = "tile"
        )]
        output_dir: String,
    },

    /// Run only the map generation step for a single tile
    Render {
        #[arg(
            help = "The path to the directory containing the output of the LiDAR processing step"
        )]
        input_dir: String,

        #[arg(
            long,
            short,
            help = "The output directory for the processed LiDAR file",
            default_value = "tile"
        )]
        output_dir: String,

        #[arg(
            long,
            short,
            help = "A list of directories containing the output of the LiDAR processing step for neighboring tiles"
        )]
        neighbors: Vec<String>,

        #[arg(long, short, help = "Skip the vector processing stage of the pipeline")]
        skip_vector: bool,
    },

    /// Process multiple LiDAR files at once
    Batch {
        #[arg(
            help = "The path to the directory containing the LiDAR files to process",
            default_value = "in"
        )]
        input_dir: String,

        #[arg(
            long,
            short,
            help = "The output directory for the processed LiDAR files",
            default_value = "out"
        )]
        output_dir: String,

        #[arg(
            long,
            short,
            help = "Number of threads used by Cassini to parallelize the work in batch mode",
            default_value = "3"
        )]
        threads: usize,

        #[arg(
            long,
            help = "Skip the LiDAR processing stage of the pipeline (only if you already ran cassini once with the same input files)."
        )]
        skip_lidar: bool,

        #[arg(long, help = "Skip the vector processing stage of the pipeline")]
        skip_vector: bool,
    },

    /// Output a default config.json file.
    DefaultConfig,
}
