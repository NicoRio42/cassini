use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Write,
};

const DEFAULT_YELLOW_THRESHOLD: f64 = 1.; // Update the docs when modifying
const DEFAULT_GREEN_THRESHOLD_1: f64 = 0.3; // Update the docs when modifying
const DEFAULT_GREEN_THRESHOLD_2: f64 = 1.0; // Update the docs when modifying
const DEFAULT_GREEN_THRESHOLD_3: f64 = 2.0; // Update the docs when modifying
const DEFAULT_CLIFF_THRESHOLD_1: f32 = 60.; // Update the docs when modifying
const DEFAULT_CLIFF_THRESHOLD_2: f32 = 60.; // Update the docs when modifying
const DEFAULT_DPI_RESOLUTION: f32 = 600.0; // Update the docs when modifying

const DEFAULT_FORM_LINES_THRESHOLD: f64 = 0.05; // Update the docs when modifying
const DEFAULT_FORM_LINES_MIN_DISTANCE_TO_CONTOUR: f64 = 5.0; // Update the docs when modifying
const DEFAULT_FORM_LINES_MAX_DISTANCE_TO_CONTOUR: f64 = 100.0; // Update the docs when modifying
const DEFAULT_FORM_LINES_MIN_LENGTH: f64 = 10.0; // Update the docs when modifying
const DEFAULT_FORM_LINES_MIN_GAP_LENGTH: f64 = 50.0; // Update the docs when modifying
const DEFAULT_FORM_LINES_ADDITIONAL_TAIL_LENGTH: f64 = 15.0; // Update the docs when modifying

#[derive(Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_yellow_threshold")]
    pub yellow_threshold: f64,
    #[serde(default = "default_green_threshold_1")]
    pub green_threshold_1: f64,
    #[serde(default = "default_green_threshold_2")]
    pub green_threshold_2: f64,
    #[serde(default = "default_green_threshold_3")]
    pub green_threshold_3: f64,
    #[serde(default = "default_cliff_threshold_1")]
    pub cliff_threshold_1: f32,
    #[serde(default = "default_cliff_threshold_2")]
    pub cliff_threshold_2: f32,
    #[serde(default = "default_dpi_resolution")]
    pub dpi_resolution: f32,
    // #[serde(default = "FormLineConfig::default")]
    // pub form_lines: FormLineConfig,
}

#[derive(Serialize, Deserialize)]
pub struct _FormLineConfig {
    #[serde(default = "default_form_lines_threshold")]
    pub threshold: f64,
    #[serde(default = "default_form_lines_min_distance_to_contour")]
    pub min_distance_to_contour: f64,
    #[serde(default = "default_form_lines_max_distance_to_contour")]
    pub max_distance_to_contour: f64,
    #[serde(default = "default_form_lines_min_length")]
    pub min_length: f64,
    #[serde(default = "default_form_lines_min_gap_length")]
    pub min_gap_length: f64,
    #[serde(default = "default_form_lines_additional_tail_length")]
    pub additional_tail_length: f64,
}

impl _FormLineConfig {
    fn _default() -> Self {
        Self {
            threshold: DEFAULT_FORM_LINES_THRESHOLD,
            min_distance_to_contour: DEFAULT_FORM_LINES_MIN_DISTANCE_TO_CONTOUR,
            max_distance_to_contour: DEFAULT_FORM_LINES_MAX_DISTANCE_TO_CONTOUR,
            min_length: DEFAULT_FORM_LINES_MIN_LENGTH,
            min_gap_length: DEFAULT_FORM_LINES_MIN_GAP_LENGTH,
            additional_tail_length: DEFAULT_FORM_LINES_ADDITIONAL_TAIL_LENGTH,
        }
    }
}

pub fn get_config() -> Config {
    let raw_config = fs::read_to_string("./config.json").unwrap_or("{}".to_owned());
    return serde_json::from_str(&raw_config).unwrap();
}

pub fn default_config() {
    let default_config: Config = serde_json::from_str("{}").unwrap();
    let json_string = serde_json::to_string_pretty(&default_config).unwrap();
    let mut file = File::create("config.json").unwrap();
    file.write_all(json_string.as_bytes()).unwrap();
}

fn default_yellow_threshold() -> f64 {
    DEFAULT_YELLOW_THRESHOLD
}

fn default_green_threshold_1() -> f64 {
    DEFAULT_GREEN_THRESHOLD_1
}

fn default_green_threshold_2() -> f64 {
    DEFAULT_GREEN_THRESHOLD_2
}

fn default_green_threshold_3() -> f64 {
    DEFAULT_GREEN_THRESHOLD_3
}

fn default_cliff_threshold_1() -> f32 {
    DEFAULT_CLIFF_THRESHOLD_1
}

fn default_cliff_threshold_2() -> f32 {
    DEFAULT_CLIFF_THRESHOLD_2
}

fn default_dpi_resolution() -> f32 {
    DEFAULT_DPI_RESOLUTION
}

fn default_form_lines_threshold() -> f64 {
    DEFAULT_FORM_LINES_THRESHOLD
}

fn default_form_lines_min_distance_to_contour() -> f64 {
    DEFAULT_FORM_LINES_MIN_DISTANCE_TO_CONTOUR
}

fn default_form_lines_max_distance_to_contour() -> f64 {
    DEFAULT_FORM_LINES_MAX_DISTANCE_TO_CONTOUR
}

fn default_form_lines_min_length() -> f64 {
    DEFAULT_FORM_LINES_MIN_LENGTH
}

fn default_form_lines_min_gap_length() -> f64 {
    DEFAULT_FORM_LINES_MIN_GAP_LENGTH
}

fn default_form_lines_additional_tail_length() -> f64 {
    DEFAULT_FORM_LINES_ADDITIONAL_TAIL_LENGTH
}
