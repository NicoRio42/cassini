use serde::{Deserialize, Serialize};
use std::fs;

const DEFAULT_DEM_BLOCK_SIZE: u32 = 1;
const DEFAULT_VEGETATION_BLOCK_SIZE: u32 = 2;
const DEFAULT_YELLOW_THRESHOLD: f64 = 700.0;
const DEFAULT_GREEN_1_THRESHOLD: f64 = 100.0;
const DEFAULT_GREEN_2_THRESHOLD: f64 = 200.0;
const DEFAULT_GREEN_3_THRESHOLD: f64 = 300.0;
const DEFAULT_SLOPE_THRESHOLD_1: f32 = 47.0;
const DEFAULT_SLOPE_THRESHOLD_2: f32 = 60.0;
const DEFAULT_DPI_RESOLUTION: f32 = 600.0;

#[derive(Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_dem_block_size")]
    pub dem_block_size: u32,
    #[serde(default = "default_vegetation_block_size")]
    pub vegetation_block_size: u32,
    #[serde(default = "default_yellow_threshold")]
    pub yellow_threshold: f64,
    #[serde(default = "default_green_1_threshold")]
    pub green_1_threshold: f64,
    #[serde(default = "default_green_2_threshold")]
    pub green_2_threshold: f64,
    #[serde(default = "default_green_3_threshold")]
    pub green_3_threshold: f64,
    #[serde(default = "default_slope_threshold_1")]
    pub slope_threshold_1: f32,
    #[serde(default = "default_slope_threshold_2")]
    pub slope_threshold_2: f32,
    #[serde(default = "default_dpi_resolution")]
    pub dpi_resolution: f32,
}

// TODO: memoize config object
pub fn get_config() -> Config {
    let raw_config = fs::read_to_string("./config.json").unwrap_or("{}".to_owned());
    return serde_json::from_str(&raw_config).unwrap();
}

fn default_dem_block_size() -> u32 {
    DEFAULT_DEM_BLOCK_SIZE
}

fn default_vegetation_block_size() -> u32 {
    DEFAULT_VEGETATION_BLOCK_SIZE
}

fn default_yellow_threshold() -> f64 {
    DEFAULT_YELLOW_THRESHOLD
}

fn default_green_1_threshold() -> f64 {
    DEFAULT_GREEN_1_THRESHOLD
}

fn default_green_2_threshold() -> f64 {
    DEFAULT_GREEN_2_THRESHOLD
}

fn default_green_3_threshold() -> f64 {
    DEFAULT_GREEN_3_THRESHOLD
}

fn default_slope_threshold_1() -> f32 {
    DEFAULT_SLOPE_THRESHOLD_1
}

fn default_slope_threshold_2() -> f32 {
    DEFAULT_SLOPE_THRESHOLD_2
}

fn default_dpi_resolution() -> f32 {
    DEFAULT_DPI_RESOLUTION
}
