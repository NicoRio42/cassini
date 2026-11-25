//! `cassini ui`: a local web page to pick IGN LiDAR HD tiles on a map,
//! download them with their DEMs into `in`, and run `batch` on them.
//!
//! The server listens on 127.0.0.1 only. Every API call needs the random
//! token of the URL the server prints, and the `Host` of that URL, so other
//! web pages open in the browser cannot start downloads.

mod ign;
mod job;

use log::{info, warn};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::{
    collections::{hash_map::RandomState, HashMap},
    fs::{self, File},
    hash::{BuildHasher, Hasher},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread::spawn,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::{
    config::Config,
    error::{CassiniError, Result, ResultContext},
    files::{is_input_file, LAZ_EXTENSIONS},
    projection::lambert93_to_wgs84,
};
use ign::{tiles_in_geometry, Geometry, IgnTile};
use job::{BatchOptions, Job, JobInput};

/// The directories of `batch`'s defaults, in the working directory.
const INPUT_DIR: &str = "in";
const OUTPUT_DIR: &str = "out";
/// The configuration `batch` reads by default, which the page edits.
const CONFIG_PATH: &str = "config.json";
const MAP_FILE_NAME: &str = "full-map.png";
const MAX_BODY_SIZE: u64 = 10 << 20;

const INDEX_HTML: &str = include_str!("assets/index.html");
const APP_JS: &str = include_str!("assets/app.js");
const STYLE_CSS: &str = include_str!("assets/style.css");

struct App {
    token: String,
    hosts: [String; 2],
    client: Client,
    /// The tiles the page selected, by id, so that a job gets their URLs
    /// from IGN's index rather than from the page.
    tiles: Mutex<HashMap<String, IgnTile>>,
    job: Mutex<Option<Arc<Job>>>,
}

pub fn run_ui(port: u16, open_browser: bool) -> Result<()> {
    let server = Server::http(("127.0.0.1", port)).map_err(|source| CassiniError::Operation {
        context: format!("could not listen on 127.0.0.1:{port}"),
        source,
    })?;
    let port = server
        .server_addr()
        .to_ip()
        .map(|address| address.port())
        .unwrap_or(port);

    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        // Applies to each read of a response, not to a whole download.
        .timeout(Duration::from_secs(60))
        .build()
        .context("could not create the HTTP client")?;
    let app = Arc::new(App {
        token: random_token(),
        hosts: [format!("127.0.0.1:{port}"), format!("localhost:{port}")],
        client,
        tiles: Mutex::new(HashMap::new()),
        job: Mutex::new(None),
    });

    let url = format!("http://127.0.0.1:{port}/?token={}", app.token);
    info!("Cassini is running at {url}");
    info!("Tiles are downloaded to `{INPUT_DIR}` and maps written to `{OUTPUT_DIR}`. Press Ctrl+C to stop");
    if open_browser {
        if let Err(error) = open::that_detached(&url) {
            warn!("Could not open a web browser ({error}). Open {url} yourself");
        }
    }

    for request in server.incoming_requests() {
        let app = app.clone();
        spawn(move || handle(&app, request));
    }
    Ok(())
}

/// A hard to guess token, from the random keys of `RandomState`.
fn random_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    (0..2)
        .map(|index| {
            let mut hasher = RandomState::new().build_hasher();
            hasher.write_u128(nanos);
            hasher.write_usize(index);
            format!("{:016x}", hasher.finish())
        })
        .collect()
}

/// The error and its causes on one line.
pub(crate) fn error_message(error: &CassiniError) -> String {
    let mut message = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        message.push_str(&format!(": {cause}"));
        source = cause.source();
    }
    if let CassiniError::Batch { failures, .. } = error {
        for failure in failures {
            message.push_str(&format!("; {}", error_message(failure)));
        }
    }
    message
}

enum Reply {
    Json(u16, String),
    Static(&'static str, &'static str),
    File(File, &'static str),
    Error(u16, String),
}

fn json<T: Serialize>(value: &T) -> Reply {
    match serde_json::to_string(value) {
        Ok(body) => Reply::Json(200, body),
        Err(error) => Reply::Error(500, error.to_string()),
    }
}

fn bad_request(error: CassiniError) -> Reply {
    Reply::Error(400, error_message(&error))
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("valid header")
}

fn handle(app: &App, mut request: Request) {
    let reply = route(app, &mut request);
    let no_store = header("Cache-Control", "no-store");
    let result = match reply {
        Reply::Json(status, body) => request.respond(
            Response::from_string(body)
                .with_status_code(status)
                .with_header(header("Content-Type", "application/json"))
                .with_header(no_store),
        ),
        Reply::Error(status, message) => request.respond(
            Response::from_string(serde_json::json!({ "error": message }).to_string())
                .with_status_code(status)
                .with_header(header("Content-Type", "application/json"))
                .with_header(no_store),
        ),
        Reply::Static(body, content_type) => request.respond(
            Response::from_string(body)
                .with_header(header("Content-Type", content_type))
                .with_header(no_store),
        ),
        Reply::File(file, content_type) => request
            .respond(Response::from_file(file).with_header(header("Content-Type", content_type))),
    };
    if let Err(error) = result {
        warn!("Could not answer a web interface request: {error}");
    }
}

fn route(app: &App, request: &mut Request) -> Reply {
    let host = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Host"))
        .map(|header| header.value.as_str().to_owned());
    if !host.is_some_and(|host| app.hosts.contains(&host)) {
        return Reply::Error(403, "unexpected Host header".to_owned());
    }

    let url = request.url().to_owned();
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    let query: HashMap<&str, &str> = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .collect();
    let method = request.method().clone();

    match (&method, path) {
        (Method::Get, "/") => return Reply::Static(INDEX_HTML, "text/html; charset=utf-8"),
        (Method::Get, "/app.js") => return Reply::Static(APP_JS, "text/javascript; charset=utf-8"),
        (Method::Get, "/style.css") => return Reply::Static(STYLE_CSS, "text/css; charset=utf-8"),
        _ => {}
    }

    let token = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("X-Cassini-Token"))
        .map(|header| header.value.as_str().to_owned())
        .or_else(|| query.get("token").map(|token| (*token).to_owned()));
    if token.as_deref() != Some(app.token.as_str()) {
        return Reply::Error(
            403,
            "missing or wrong token: open the URL cassini ui printed".to_owned(),
        );
    }

    match (&method, path) {
        (Method::Get, "/api/state") => state(),
        (Method::Post, "/api/tiles") => match read_json(request) {
            Ok(body) => select_tiles(app, body),
            Err(reply) => reply,
        },
        (Method::Post, "/api/jobs") => match read_json(request) {
            Ok(body) => start_job(app, body),
            Err(reply) => reply,
        },
        (Method::Get, "/api/job") => {
            let since = query
                .get("since")
                .and_then(|since| since.parse().ok())
                .unwrap_or(0);
            json(
                &app.job
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|job| job.snapshot(since)),
            )
        }
        (Method::Post, "/api/job/cancel") => {
            if let Some(job) = app.job.lock().unwrap().as_ref() {
                job.cancel();
            }
            json(&serde_json::json!({}))
        }
        (Method::Get, "/api/results") => json(&results(&app.token)),
        (Method::Get, path) if path.starts_with("/results/") => result_image(path),
        _ => Reply::Error(404, format!("no route for {method} {path}")),
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(request: &mut Request) -> std::result::Result<T, Reply> {
    let mut body = vec![];
    request
        .as_reader()
        .take(MAX_BODY_SIZE + 1)
        .read_to_end(&mut body)
        .map_err(|error| Reply::Error(400, error.to_string()))?;
    if body.len() as u64 > MAX_BODY_SIZE {
        return Err(Reply::Error(413, "request too large".to_owned()));
    }
    serde_json::from_slice(&body)
        .map_err(|error| Reply::Error(400, format!("invalid request: {error}")))
}

#[derive(Serialize)]
struct State {
    config: Config,
    /// Why `config.json` couldn't be read, if it couldn't.
    config_error: Option<String>,
    default_config: Config,
    options: BatchOptions,
    choices: serde_json::Value,
    /// The LAZ files already in the input directory.
    input_files: Vec<String>,
    input_dir: String,
    output_dir: String,
}

fn state() -> Reply {
    let (config, config_error) = match Config::load(Some(Path::new(CONFIG_PATH))) {
        Ok(config) => (config, None),
        Err(_) if !Path::new(CONFIG_PATH).exists() => (Config::default(), None),
        Err(error) => (Config::default(), Some(error_message(&error))),
    };
    json(&State {
        config,
        config_error,
        default_config: Config::default(),
        options: BatchOptions::default(),
        choices: BatchOptions::choices(),
        input_files: laz_files_in(Path::new(INPUT_DIR)),
        input_dir: INPUT_DIR.to_owned(),
        output_dir: OUTPUT_DIR.to_owned(),
    })
}

fn laz_files_in(dir: &Path) -> Vec<String> {
    let mut files: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| is_input_file(&entry.path(), &LAZ_EXTENSIONS))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    files
}

#[derive(Deserialize)]
struct TilesRequest {
    geometry: Geometry,
}

#[derive(Serialize)]
struct SelectedTile {
    #[serde(flatten)]
    tile: IgnTile,
    /// Whether its LAZ file is already in the input directory.
    downloaded: bool,
}

fn select_tiles(app: &App, body: TilesRequest) -> Reply {
    let tiles = match tiles_in_geometry(&app.client, &body.geometry) {
        Ok(tiles) => tiles,
        Err(error) => return bad_request(error),
    };
    let mut cache = app.tiles.lock().unwrap();
    let selected: Vec<SelectedTile> = tiles
        .into_iter()
        .map(|tile| {
            cache.insert(tile.id.clone(), tile.clone());
            SelectedTile {
                downloaded: Path::new(INPUT_DIR).join(&tile.laz_file_name).exists(),
                tile,
            }
        })
        .collect();
    json(&selected)
}

#[derive(Deserialize)]
struct JobRequest {
    tiles: Vec<String>,
    config: serde_json::Value,
    options: BatchOptions,
}

fn start_job(app: &App, body: JobRequest) -> Reply {
    let mut job = app.job.lock().unwrap();
    if job.as_ref().is_some_and(|job| job.status().is_running()) {
        return Reply::Error(409, "a job is already running".to_owned());
    }
    if body.tiles.is_empty() {
        return Reply::Error(400, "no tile selected".to_owned());
    }

    let tiles = {
        let cache = app.tiles.lock().unwrap();
        let tiles: Option<Vec<IgnTile>> =
            body.tiles.iter().map(|id| cache.get(id).cloned()).collect();
        match tiles {
            Some(tiles) => tiles,
            None => return Reply::Error(400, "unknown tile: select the tiles again".to_owned()),
        }
    };

    let config = match serde_json::from_value::<Config>(body.config)
        .map_err(|error| CassiniError::InvalidConfig {
            message: error.to_string(),
        })
        .and_then(|config| config.validate().map(|()| config))
    {
        Ok(config) => config,
        Err(error) => return bad_request(error),
    };
    let written = serde_json::to_string_pretty(&config)
        .context("could not serialize the configuration")
        .and_then(|config| {
            fs::write(CONFIG_PATH, config).context(format!("could not write `{CONFIG_PATH}`"))
        });
    if let Err(error) = written {
        return Reply::Error(500, error_message(&error));
    }

    let input = JobInput {
        tiles,
        options: body.options,
        config_path: PathBuf::from(CONFIG_PATH),
        input_dir: PathBuf::from(INPUT_DIR),
        output_dir: PathBuf::from(OUTPUT_DIR),
    };
    match Job::start(input, app.client.clone()) {
        Ok(started) => {
            *job = Some(started);
            json(&serde_json::json!({}))
        }
        Err(error) => bad_request(error),
    }
}

/// A rendered tile of the output directory.
#[derive(Serialize)]
struct ResultTile {
    name: String,
    url: String,
    /// North-west, north-east, south-east and south-west corners, in
    /// longitudes and latitudes, as MapLibre's image sources take them.
    corners: [[f64; 2]; 4],
}

/// The rendered tiles of the output directory: its `<min_x>_<min_y>_<max_x>_<max_y>`
/// directories with a map.
fn results(token: &str) -> Vec<ResultTile> {
    let mut tiles: Vec<ResultTile> = fs::read_dir(OUTPUT_DIR)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let [min_x, min_y, max_x, max_y] = tile_extent(&name)?;
            let modified = fs::metadata(entry.path().join(MAP_FILE_NAME))
                .and_then(|metadata| metadata.modified())
                .ok()?
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or_default();
            let corner = |x: i64, y: i64| {
                let (lon, lat) = lambert93_to_wgs84(x as f64, y as f64);
                [lon, lat]
            };
            Some(ResultTile {
                url: format!("/results/{name}/{MAP_FILE_NAME}?token={token}&v={modified}"),
                corners: [
                    corner(min_x, max_y),
                    corner(max_x, max_y),
                    corner(max_x, min_y),
                    corner(min_x, min_y),
                ],
                name,
            })
        })
        .collect();
    tiles.sort_by(|a, b| a.name.cmp(&b.name));
    tiles
}

/// The extent in a tile directory name, `907000_6454000_908000_6455000`.
fn tile_extent(name: &str) -> Option<[i64; 4]> {
    let values: Vec<i64> = name
        .split('_')
        .map(|value| value.parse().ok())
        .collect::<Option<_>>()?;
    values.try_into().ok()
}

fn result_image(path: &str) -> Reply {
    let Some(name) = path
        .strip_prefix("/results/")
        .and_then(|rest| rest.strip_suffix(&format!("/{MAP_FILE_NAME}")))
    else {
        return Reply::Error(404, "not found".to_owned());
    };
    // Only the tile directories of the output directory are served.
    if tile_extent(name).is_none() {
        return Reply::Error(404, "not found".to_owned());
    }
    match File::open(Path::new(OUTPUT_DIR).join(name).join(MAP_FILE_NAME)) {
        Ok(file) => Reply::File(file, "image/png"),
        Err(_) => Reply::Error(404, "not found".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_tile_extents_from_output_directory_names_only() {
        assert_eq!(
            tile_extent("907000_6454000_908000_6455000"),
            Some([907000, 6454000, 908000, 6455000])
        );
        assert_eq!(tile_extent("../907000_6454000_908000_6455000"), None);
        assert_eq!(tile_extent("907000_6454000_908000"), None);
        assert_eq!(tile_extent("merged-map.png"), None);
    }

    #[test]
    fn tokens_differ() {
        assert_ne!(random_token(), random_token());
        assert_eq!(random_token().len(), 32);
    }
}
