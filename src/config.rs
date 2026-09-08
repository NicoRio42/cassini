use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
};

use crate::error::{CassiniError, Result, ResultContext};

const DEFAULT_YELLOW_THRESHOLD: f32 = 1.; // Update the docs when modifying
const DEFAULT_GREEN_THRESHOLD_1: f32 = 1.; // Update the docs when modifying
const DEFAULT_GREEN_THRESHOLD_2: f32 = 2.; // Update the docs when modifying
const DEFAULT_GREEN_THRESHOLD_3: f32 = 3.; // Update the docs when modifying
const DEFAULT_LOW_VEGETATION_DENSITY_THRESHOLD: f32 = 1.; // Update the docs when modifying
const DEFAULT_CLIFF_THRESHOLD_1: f32 = 60.; // Update the docs when modifying
const DEFAULT_CLIFF_THRESHOLD_2: f32 = 60.; // Update the docs when modifying
const DEFAULT_DPI_RESOLUTION: f32 = 600.0; // Update the docs when modifying

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_yellow_threshold")]
    pub yellow_threshold: f32,
    #[serde(default = "default_green_threshold_1")]
    pub green_threshold_1: f32,
    #[serde(default = "default_green_threshold_2")]
    pub green_threshold_2: f32,
    #[serde(default = "default_green_threshold_3")]
    pub green_threshold_3: f32,
    #[serde(default = "default_low_vegetation_density_threshold")]
    pub low_vegetation_density_threshold: f32,
    #[serde(default = "default_cliff_threshold_1")]
    pub cliff_threshold_1: f32,
    #[serde(default = "default_cliff_threshold_2")]
    pub cliff_threshold_2: f32,
    #[serde(default = "default_dpi_resolution")]
    pub dpi_resolution: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            yellow_threshold: DEFAULT_YELLOW_THRESHOLD,
            green_threshold_1: DEFAULT_GREEN_THRESHOLD_1,
            green_threshold_2: DEFAULT_GREEN_THRESHOLD_2,
            green_threshold_3: DEFAULT_GREEN_THRESHOLD_3,
            low_vegetation_density_threshold: DEFAULT_LOW_VEGETATION_DENSITY_THRESHOLD,
            cliff_threshold_1: DEFAULT_CLIFF_THRESHOLD_1,
            cliff_threshold_2: DEFAULT_CLIFF_THRESHOLD_2,
            dpi_resolution: DEFAULT_DPI_RESOLUTION,
        }
    }
}

impl Config {
    pub fn load(config_path: Option<&Path>) -> Result<Self> {
        let raw_config = match config_path {
            Some(path) => fs::read_to_string(path)
                .context(format!("could not read configuration `{}`", path.display()))?,
            None => match fs::read_to_string("./config.json") {
                Ok(contents) => contents,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => "{}".to_owned(),
                Err(error) => {
                    return Err(CassiniError::operation(
                        "could not read configuration `./config.json`",
                        error,
                    ));
                }
            },
        };

        let path = config_path.unwrap_or_else(|| Path::new("./config.json"));
        let config: Self = serde_json::from_str(&raw_config).context(format!(
            "could not parse configuration `{}`",
            path.display()
        ))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        let finite_non_negative = [
            ("yellow_threshold", self.yellow_threshold),
            ("green_threshold_1", self.green_threshold_1),
            ("green_threshold_2", self.green_threshold_2),
            ("green_threshold_3", self.green_threshold_3),
            (
                "low_vegetation_density_threshold",
                self.low_vegetation_density_threshold,
            ),
            ("cliff_threshold_1", self.cliff_threshold_1),
            ("cliff_threshold_2", self.cliff_threshold_2),
        ];

        if let Some((name, _)) = finite_non_negative
            .iter()
            .find(|(_, value)| !value.is_finite() || *value < 0.0)
        {
            return Err(CassiniError::InvalidConfig {
                message: format!("`{name}` must be a finite, non-negative number"),
            });
        }

        if !(self.green_threshold_1 <= self.green_threshold_2
            && self.green_threshold_2 <= self.green_threshold_3)
        {
            return Err(CassiniError::InvalidConfig {
                message: "green thresholds must be ordered from 1 through 3".to_owned(),
            });
        }

        if self.cliff_threshold_1 > self.cliff_threshold_2 {
            return Err(CassiniError::InvalidConfig {
                message: "`cliff_threshold_1` must not exceed `cliff_threshold_2`".to_owned(),
            });
        }

        if !self.dpi_resolution.is_finite()
            || self.dpi_resolution <= 0.0
            || self.dpi_resolution > 2400.0
        {
            return Err(CassiniError::InvalidConfig {
                message: "`dpi_resolution` must be greater than 0 and at most 2400".to_owned(),
            });
        }

        Ok(())
    }
}

pub fn get_config(config_path: Option<&Path>) -> Result<Config> {
    Config::load(config_path)
}

pub fn default_config() -> Result<()> {
    let json_string = serde_json::to_string_pretty(&Config::default())
        .context("could not serialize the default configuration")?;
    let mut file = File::create("config.json").context("could not create `config.json`")?;
    file.write_all(json_string.as_bytes())
        .context("could not write `config.json`")?;
    Ok(())
}

fn default_yellow_threshold() -> f32 {
    DEFAULT_YELLOW_THRESHOLD
}

fn default_green_threshold_1() -> f32 {
    DEFAULT_GREEN_THRESHOLD_1
}

fn default_green_threshold_2() -> f32 {
    DEFAULT_GREEN_THRESHOLD_2
}

fn default_green_threshold_3() -> f32 {
    DEFAULT_GREEN_THRESHOLD_3
}

fn default_low_vegetation_density_threshold() -> f32 {
    DEFAULT_LOW_VEGETATION_DENSITY_THRESHOLD
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

#[cfg(test)]
mod tests {
    use super::get_config;
    use std::{fs, time::SystemTime};

    #[test]
    fn loads_config_from_the_selected_path() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("cassini-config-{unique}.json"));
        fs::write(&path, r#"{"dpi_resolution": 300}"#).unwrap();

        let config = get_config(Some(&path)).unwrap();

        fs::remove_file(path).unwrap();
        assert_eq!(config.dpi_resolution, 300.0);
    }

    #[test]
    fn rejects_invalid_dpi() {
        let config = super::Config {
            dpi_resolution: 0.0,
            ..super::Config::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_misordered_green_thresholds() {
        let config = super::Config {
            green_threshold_1: 3.0,
            green_threshold_2: 2.0,
            ..super::Config::default()
        };

        assert!(config.validate().is_err());
    }
}
