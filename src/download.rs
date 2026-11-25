use log::{info, warn};
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_TYPE, RETRY_AFTER, USER_AGENT};
use std::{
    fs::{self, File},
    io::copy,
    path::Path,
    thread::sleep,
    time::{Duration, Instant, SystemTime},
};

use crate::{
    constants::BUFFER,
    error::{CassiniError, Result, ResultContext, TileId},
    projection::lambert93_to_wgs84,
};

const OVERPASS_API_URL: &str = "https://overpass-api.de/api/interpreter";
pub(crate) const DEFAULT_RETRY_DELAY: Duration = Duration::from_secs(5);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(60);
pub(crate) const USER_AGENT_VALUE: &str = concat!(
    env!("CARGO_PKG_NAME"),
    "/",
    env!("CARGO_PKG_VERSION"),
    " (+",
    env!("CARGO_PKG_REPOSITORY"),
    ")"
);

pub fn download_osm_file(
    min_x: i64,
    min_y: i64,
    max_x: i64,
    max_y: i64,
    output_dir_path: &Path,
) -> Result<()> {
    let osm_file_path = output_dir_path.join(format!("{:0>7}_{:0>7}.osm", min_x, max_y));
    // The download goes to a temporary file, renamed when complete: an
    // existing `.osm` file is never downloaded again.
    let partial_osm_file_path = osm_file_path.with_extension("osm.part");

    let tile_id = TileId {
        min_x,
        min_y,
        max_x,
        max_y,
    };

    info!("Tile {tile_id}. Downloading osm file");

    let start = Instant::now();

    let (min_lon, min_lat) = lambert93_to_wgs84(
        (min_x - BUFFER as i64) as f64,
        (min_y - BUFFER as i64) as f64,
    );
    let (max_lon, max_lat) = lambert93_to_wgs84(
        (max_x + BUFFER as i64) as f64,
        (max_y + BUFFER as i64) as f64,
    );

    // Overpass Query
    let query = r#"
[out:xml][timeout:25];
(
  way["building"]({{bbox}});
  relation["building"]({{bbox}});
  way["natural"="water"]({{bbox}});
  relation["natural"="water"]({{bbox}});
  way["natural"="wetland"]({{bbox}});
  relation["natural"="wetland"]({{bbox}});
  way["landuse"="residential"]({{bbox}});
  relation["landuse"="residential"]({{bbox}});
  way["landuse"="railway"]({{bbox}});
  relation["landuse"="railway"]({{bbox}});
  way["landuse"="industrial"]({{bbox}});
  relation["landuse"="industrial"]({{bbox}});
  way["natural"="coastline"]({{bbox}});
  way["highway"]({{bbox}});
  way["waterway"]({{bbox}});
  way["railway"]({{bbox}});
  way["power"]({{bbox}});
  way["aerialway"]({{bbox}});
);
out body;
>;
out skel qt;
"#;

    // Replace {{bbox}} with your bounding box (south, west, north, east)
    let bbox = format!("{min_lat:.8},{min_lon:.8},{max_lat:.8},{max_lon:.8}");
    let formatted_query = query.replace("{{bbox}}", &bbox);
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .build()
        .context("could not create the Overpass HTTP client")?;

    let mut retries_left = 5;

    let mut response = loop {
        let response_result = client
            .post(OVERPASS_API_URL)
            .body(formatted_query.clone())
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(USER_AGENT, USER_AGENT_VALUE)
            .send();

        let retry_delay = match response_result {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    break response;
                }

                if !is_transient_status(status.as_u16()) || retries_left == 0 {
                    return Err(CassiniError::HttpStatus {
                        url: OVERPASS_API_URL.to_owned(),
                        status: status.as_u16(),
                    });
                }

                let retry_delay = retry_delay_from_response(&response);
                warn!(
                    "Overpass API returned error status {}. Retrying in {:.1?} ({} retries left)",
                    status, retry_delay, retries_left
                );

                retry_delay
            }
            Err(error) => {
                if retries_left == 0 {
                    return Err(CassiniError::operation(
                        format!("could not download vector data for tile {tile_id}"),
                        error,
                    ));
                }

                warn!(
                    "Overpass API request failed: {}. Retrying in {:.1?} ({} retries left)",
                    error, DEFAULT_RETRY_DELAY, retries_left
                );

                DEFAULT_RETRY_DELAY
            }
        };

        retries_left -= 1;
        sleep(retry_delay);
    };

    let mut file = File::create(&partial_osm_file_path).context(format!(
        "could not create `{}`",
        partial_osm_file_path.display()
    ))?;
    copy(&mut response, &mut file).context(format!(
        "could not write `{}`",
        partial_osm_file_path.display()
    ))?;
    drop(file);
    fs::rename(&partial_osm_file_path, &osm_file_path).context(format!(
        "could not rename `{}` to `{}`",
        partial_osm_file_path.display(),
        osm_file_path.display()
    ))?;

    let duration = start.elapsed();

    info!("Tile {tile_id}. Osm files downloaded in {duration:.1?}");

    Ok(())
}

pub(crate) fn retry_delay_from_response(response: &reqwest::blocking::Response) -> Duration {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_retry_after)
        .unwrap_or(DEFAULT_RETRY_DELAY)
        .min(MAX_RETRY_DELAY)
}

pub(crate) fn is_transient_status(status: u16) -> bool {
    status == 429 || matches!(status, 500 | 502 | 503 | 504)
}

fn parse_retry_after(value: &str) -> Option<Duration> {
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }

    httpdate::parse_http_date(value).ok().map(|date| {
        date.duration_since(SystemTime::now())
            .unwrap_or(Duration::ZERO)
    })
}

#[cfg(test)]
mod tests {
    use super::parse_retry_after;
    use std::time::Duration;

    #[test]
    fn parses_retry_after_as_seconds() {
        assert_eq!(parse_retry_after("12"), Some(Duration::from_secs(12)));
    }

    #[test]
    fn parses_retry_after_as_http_date() {
        assert!(parse_retry_after("Wed, 21 Oct 2099 07:28:00 GMT").is_some());
    }

    #[test]
    fn ignores_invalid_retry_after() {
        assert_eq!(parse_retry_after("not a delay"), None);
    }
}
