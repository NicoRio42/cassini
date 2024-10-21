use clap::Parser;

// Update the docs when modifying
#[derive(Parser, Debug)]
#[command(
    version,
    about = "A software that generates highly accurate topographic maps from LiDAR data. See documentation: https://cassini-map.com",
    long_about = "Cassini is a software that generates highly accurate topographic maps from LiDAR data and shapefile vector data in record times."
)]
pub struct Args {
    #[arg(help = "The LiDAR file path When processing a single file.")]
    pub file_path: Option<String>,
    #[arg(
        long,
        help = "Enable batch mode for processing multiple LiDAR files placed in the in directory"
    )]
    pub batch: bool,
    #[arg(
        long,
        help = "Skip the LiDAR processing stage of the pipeline (only if you already ran cassini once with the same input files)."
    )]
    pub skip_lidar: bool,
    #[arg(long, help = "Skip the vector processing stage of the pipeline.")]
    pub skip_vector: bool,
    #[arg(
        long,
        help = "Number of threads used by Cassini to parallelize the work in batch mode (default 3)."
    )]
    pub threads: Option<usize>,
    #[arg(long, help = "Output a default config.json file.")]
    pub default_config: bool,
}
