use clap::Parser;

// Update the docs when modifying
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Optional name to operate on
    pub file_path: Option<String>,
    #[arg(long)]
    pub skip_lidar: bool,
    #[arg(long)]
    pub batch: bool,
    #[arg(long)]
    pub threads: Option<usize>,
    #[arg(long)]
    pub default_config: bool,
}
