use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Write,
};

const DEFAULT_YELLOW_THRESHOLD: f64 = 0.5; // Update the docs when modifying
const DEFAULT_GREEN_THRESHOLD_1: f64 = 0.2; // Update the docs when modifying
const DEFAULT_GREEN_THRESHOLD_2: f64 = 1.0; // Update the docs when modifying
const DEFAULT_GREEN_THRESHOLD_3: f64 = 2.0; // Update the docs when modifying
const DEFAULT_CLIFF_THRESHOLD_1: f32 = 45.; // Update the docs when modifying
const DEFAULT_CLIFF_THRESHOLD_2: f32 = 55.; // Update the docs when modifying
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

// #[derive(Serialize, Deserialize)]
// pub struct VectorConfig {
//     pub uncrossable_body_of_water_301: HashMap<String, String>,
//     pub shallow_body_of_water_302: HashMap<String, String>,
//     pub crossable_watercourse_304: HashMap<String, String>,
//     pub small_crossable_watercourse_305: HashMap<String, String>,
//     pub minor_or_seasonal_water_channel_306: HashMap<String, String>,
//     pub uncrossable_marsh_307: HashMap<String, String>,
//     pub marsh_308: HashMap<String, String>,
//     pub cultivated_land_412: HashMap<String, String>,
//     pub orchard_413: HashMap<String, String>,
//     pub vineyard_414: HashMap<String, String>,
//     pub paved_area_501: HashMap<String, String>,
//     pub wide_road_502: HashMap<String, String>,
//     pub road_503: HashMap<String, String>,
//     pub vehicle_track_504: HashMap<String, String>,
//     pub footpath_505: HashMap<String, String>,
//     pub small_footpath_506: HashMap<String, String>,
//     pub less_distinct_small_footpath_507: HashMap<String, String>,
//     pub narrow_ride_508: HashMap<String, String>,
//     pub railway_509: HashMap<String, String>,
//     pub power_line_510: HashMap<String, String>,
//     pub major_power_line_511: HashMap<String, String>,
//     pub wall_513_1: HashMap<String, String>,
//     pub retained_wall_513_2: HashMap<String, String>,
//     pub ruined_wall_515: HashMap<String, String>,
//     pub impassable_wall_515: HashMap<String, String>,
//     pub fence_516: HashMap<String, String>,
//     pub ruined_fence_517: HashMap<String, String>,
//     pub impassable_fence_518: HashMap<String, String>,
//     pub out_of_bounds_area_520: HashMap<String, String>,
//     pub building_521: HashMap<String, String>,
//     pub canopy_522: HashMap<String, String>,
//     pub ruin_523: HashMap<String, String>,
//     pub high_tower_524: HashMap<String, String>,
//     pub prominent_line_feature_528: HashMap<String, String>,
//     pub impassable_prominent_line_feature_529: HashMap<String, String>,
//     pub stairway_532: HashMap<String, String>,
// }

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
