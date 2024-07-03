use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[arg(long)]
    pub skip_lidar: bool,
    #[arg(long)]
    pub input: Option<String>,
    #[arg(long)]
    pub output: Option<String>,
    #[arg(long)]
    pub batch: bool,
    #[arg(long)]
    pub threads: Option<usize>,
}
