use log::{info, warn};
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_TYPE, RETRY_AFTER, USER_AGENT};
use std::{
    fs::File,
    io::{copy, BufRead, BufReader, BufWriter, Write},
    path::Path,
    process::Command,
    thread::sleep,
    time::{Duration, Instant, SystemTime},
};

use crate::{
    constants::BUFFER,
    error::{CassiniError, Result, ResultContext, Stage, TileId},
    process::checked_output_with_input,
};

const OVERPASS_API_URL: &str = "https://overpass-api.de/api/interpreter";
const DEFAULT_RETRY_DELAY: Duration = Duration::from_secs(5);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(60);
const USER_AGENT_VALUE: &str = concat!(
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
    let raw_osm_file_path = output_dir_path.join(format!("{:0>7}_{:0>7}_raw.osm", min_x, max_y));
    let osm_file_path = output_dir_path.join(format!("{:0>7}_{:0>7}.osm", min_x, max_y));

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Downloading osm file",
        min_x, min_y, max_x, max_y
    );

    let start = Instant::now();

    let tile_id = TileId {
        min_x,
        min_y,
        max_x,
        max_y,
    };

    let (min_lon, min_lat) = convert_coords_from_lambert_93_to_gps(
        (min_x - BUFFER as i64) as f64,
        (min_y - BUFFER as i64) as f64,
        tile_id,
    )?;

    let (max_lon, max_lat) = convert_coords_from_lambert_93_to_gps(
        (max_x + BUFFER as i64) as f64,
        (max_y + BUFFER as i64) as f64,
        tile_id,
    )?;

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
    let bbox = format!("{},{},{},{}", min_lat, min_lon, max_lat, max_lon);
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

    let mut file = File::create(&raw_osm_file_path).context(format!(
        "could not create `{}`",
        raw_osm_file_path.display()
    ))?;
    copy(&mut response, &mut file)
        .context(format!("could not write `{}`", raw_osm_file_path.display()))?;

    fix_osm_file(&raw_osm_file_path, &osm_file_path)?;
    std::fs::remove_file(&raw_osm_file_path).context(format!(
        "could not remove temporary download `{}`",
        raw_osm_file_path.display()
    ))?;

    let duration = start.elapsed();

    info!(
        "Tile min_x={} min_y={} max_x={} max_y={}. Osm files downloaded in {:.1?}",
        min_x, min_y, max_x, max_y, duration
    );

    Ok(())
}

fn retry_delay_from_response(response: &reqwest::blocking::Response) -> Duration {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_retry_after)
        .unwrap_or(DEFAULT_RETRY_DELAY)
        .min(MAX_RETRY_DELAY)
}

fn is_transient_status(status: u16) -> bool {
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

fn convert_coords_from_lambert_93_to_gps(x: f64, y: f64, tile: TileId) -> Result<(f64, f64)> {
    let input = format!("{x:.1} {y:.1}\n");
    let output = checked_output_with_input(
        Command::new("cs2cs").args(["+init=epsg:2154", "+to", "+init=epsg:4326", "-f", "%.8f"]),
        input.as_bytes(),
        Stage::Download,
        Some(tile),
    )?;

    let result = String::from_utf8(output.stdout)
        .context("`cs2cs` returned output that was not valid UTF-8")?;
    let coords: Vec<&str> = result.split_whitespace().collect();

    if coords.len() < 2 {
        return Err(CassiniError::InvalidInput {
            message: format!("`cs2cs` returned malformed coordinates for tile {tile}"),
        });
    }

    let lon: f64 = coords[0]
        .parse()
        .context("could not parse longitude returned by `cs2cs`")?;
    let lat: f64 = coords[1]
        .parse()
        .context("could not parse latitude returned by `cs2cs`")?;

    Ok((lon, lat))
}

fn fix_osm_file(input: &Path, output: &Path) -> Result<()> {
    let reader =
        BufReader::new(File::open(input).context(format!("could not open `{}`", input.display()))?);
    let mut writer = BufWriter::new(
        File::create(output).context(format!("could not create `{}`", output.display()))?,
    );
    let mut relations_lines: Vec<String> = vec![];

    let mut is_inside_relation = false;

    for line in reader.lines() {
        let line = line.context(format!("could not read `{}`", input.display()))?;

        if line.contains("</osm>") {
            for relations_line in &relations_lines {
                writeln!(writer, "{}", relations_line)
                    .context(format!("could not write `{}`", output.display()))?;
            }

            writeln!(writer, "{}", line)
                .context(format!("could not write `{}`", output.display()))?;
            break;
        }

        if line.contains("</relation>") {
            is_inside_relation = false;
            relations_lines.push(line);
            continue;
        }

        if line.contains("<relation") {
            is_inside_relation = true;
            relations_lines.push(line);
            continue;
        }

        if is_inside_relation {
            relations_lines.push(line);
            continue;
        }

        writeln!(writer, "{}", line).context(format!("could not write `{}`", output.display()))?;
    }
    writer
        .flush()
        .context(format!("could not flush `{}`", output.display()))?;
    Ok(())
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
