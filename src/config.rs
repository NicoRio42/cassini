use image::Rgba;
use serde::{Deserialize, Serialize};
use std::fs;

const DEFAULT_DEM_RESOLUTION: u32 = 1;
const DEFAULT_VEGETATION_RESOLUTION: u32 = 2;
const DEFAULT_YELLOW_THRESHOLD: f64 = 700.0;
const DEFAULT_GREEN_1_THRESHOLD: f64 = 25.0;
const DEFAULT_GREEN_2_THRESHOLD: f64 = 100.0;
const DEFAULT_GREEN_3_THRESHOLD: f64 = 150.0;
const DEFAULT_SLOPE_THRESHOLD: f32 = 40.0;

#[derive(Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_dem_resolution")]
    pub dem_resolution: u32,
    #[serde(default = "default_vegetation_resolution")]
    pub vegetation_resolution: u32,
    #[serde(default = "default_yellow_threshold")]
    pub yellow_threshold: f64,
    #[serde(default = "default_green_1_threshold")]
    pub green_1_threshold: f64,
    #[serde(default = "default_green_2_threshold")]
    pub green_2_threshold: f64,
    #[serde(default = "default_green_3_threshold")]
    pub green_3_threshold: f64,
    #[serde(default = "default_slope_threshold")]
    pub slope_threshold: f32,
}

// TODO: memoize config object
pub fn get_config() -> Config {
    let raw_config = fs::read_to_string("./config.json").unwrap_or("{}".to_owned());

    return serde_json::from_str(&raw_config).unwrap();
}

fn default_dem_resolution() -> u32 {
    DEFAULT_DEM_RESOLUTION
}

fn default_vegetation_resolution() -> u32 {
    DEFAULT_VEGETATION_RESOLUTION
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

fn default_slope_threshold() -> f32 {
    DEFAULT_SLOPE_THRESHOLD
}
