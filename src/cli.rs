use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[arg(long)]
    pub skip_lidar: bool,
    #[arg(long)]
    pub batch: bool,
}
