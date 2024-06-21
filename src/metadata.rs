use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Serialize, Deserialize)]
pub struct Bbox {
    pub maxx: f32,
    pub maxy: f32,
    pub minx: f32,
    pub miny: f32,
}

#[derive(Serialize, Deserialize)]
pub struct FiltersInfo {
    pub bbox: Bbox,
}

#[derive(Serialize, Deserialize)]
pub struct Stages {
    #[serde(rename = "filters.info")]
    pub filters_info: FiltersInfo,
}

#[derive(Serialize, Deserialize)]
pub struct Metadata {
    pub stages: Stages,
}

// TODO: memoize metadata object
pub fn get_metadata() -> Metadata {
    let raw_metadata = match fs::read_to_string("./out/metadata.json") {
        Ok(contents) => contents,
        Err(_) => panic!("No metadata file"),
    };

    return serde_json::from_str(&raw_metadata).unwrap();
}
