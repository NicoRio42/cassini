//! A job of the web interface: download the selected tiles into the input
//! directory, then run `cassini batch` on it in a child process, so that a
//! failing batch never takes the server down and cancelling it is killing
//! the process.

use clap::ValueEnum;
use log::info;
use reqwest::{
    blocking::Client,
    header::{CONTENT_TYPE, USER_AGENT},
};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{sleep, spawn},
    time::Duration,
};

use super::ign::IgnTile;
use crate::{
    download::{
        is_transient_status, retry_delay_from_response, DEFAULT_RETRY_DELAY, USER_AGENT_VALUE,
    },
    error::{CassiniError, Result, ResultContext},
    lidar::VegetationLayers,
    vegetation::UndergrowthMode,
    DEFAULT_OCAD_SIMPLIFY_TOLERANCE,
};

/// The default of `batch --threads`.
const DEFAULT_THREADS: usize = 3;
const DOWNLOAD_RETRIES: usize = 5;
/// The log lines a job keeps, the oldest ones dropped first.
const MAX_LOG_LINES: usize = 5000;

/// The `batch` options the web interface sets, with the same names and
/// defaults as on the command line.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BatchOptions {
    pub threads: usize,
    pub vegetation_layers: String,
    pub undergrowth: String,
    pub skip_vector: bool,
    pub skip_520: bool,
    pub ocad: bool,
    pub ocad_template: Option<String>,
    pub ocad_simplify: f64,
}

impl Default for BatchOptions {
    fn default() -> Self {
        Self {
            threads: DEFAULT_THREADS,
            vegetation_layers: value_name(VegetationLayers::Height),
            undergrowth: value_name(UndergrowthMode::Merge),
            skip_vector: false,
            skip_520: false,
            ocad: false,
            ocad_template: None,
            ocad_simplify: DEFAULT_OCAD_SIMPLIFY_TOLERANCE,
        }
    }
}

impl BatchOptions {
    /// The values the web interface offers for the options taking one.
    pub fn choices() -> serde_json::Value {
        serde_json::json!({
            "vegetation_layers": value_names::<VegetationLayers>(),
            "undergrowth": value_names::<UndergrowthMode>(),
        })
    }

    /// The `batch` arguments for these options, after the input directory.
    fn args(&self) -> Result<Vec<OsString>> {
        let invalid = |message: String| CassiniError::InvalidInput { message };
        if self.threads == 0 {
            return Err(invalid(
                "the number of threads must be at least 1".to_owned(),
            ));
        }
        VegetationLayers::from_str(&self.vegetation_layers, false).map_err(invalid)?;
        UndergrowthMode::from_str(&self.undergrowth, false).map_err(invalid)?;
        if !self.ocad_simplify.is_finite() || self.ocad_simplify < 0.0 {
            return Err(invalid(
                "the OCAD simplification tolerance must be a finite, non-negative number of meters"
                    .to_owned(),
            ));
        }

        let mut args: Vec<OsString> = vec![
            "--threads".into(),
            self.threads.to_string().into(),
            "--vegetation-layers".into(),
            self.vegetation_layers.clone().into(),
            "--undergrowth".into(),
            self.undergrowth.clone().into(),
        ];
        if self.skip_vector {
            args.push("--skip-vector".into());
        }
        if self.skip_520 {
            args.push("--skip-520".into());
        }
        if self.ocad {
            args.push("--ocad".into());
            args.push("--ocad-simplify".into());
            args.push(self.ocad_simplify.to_string().into());
            if let Some(template) = self.ocad_template.as_deref().map(str::trim) {
                if !template.is_empty() {
                    if !Path::new(template).is_file() {
                        return Err(invalid(format!("OCAD template `{template}` not found")));
                    }
                    args.push("--ocad-template".into());
                    args.push(template.into());
                }
            }
        }
        Ok(args)
    }
}

fn value_name<T: ValueEnum>(value: T) -> String {
    value
        .to_possible_value()
        .map(|value| value.get_name().to_owned())
        .unwrap_or_default()
}

fn value_names<T: ValueEnum + Clone>() -> Vec<String> {
    T::value_variants()
        .iter()
        .map(|value| value_name(value.clone()))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Downloading,
    Processing,
    Done,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn is_running(self) -> bool {
        matches!(self, Self::Downloading | Self::Processing)
    }
}

/// The file being downloaded.
#[derive(Clone, Debug, Serialize)]
pub struct Download {
    file_name: String,
    /// Its rank among the files to download, from 1.
    index: usize,
    count: usize,
    bytes: u64,
    total: Option<u64>,
}

/// What the web interface shows of a job.
#[derive(Debug, Serialize)]
pub struct JobSnapshot {
    status: JobStatus,
    error: Option<String>,
    tiles: Vec<String>,
    download: Option<Download>,
    /// The log lines from `since`, or from the oldest one kept.
    logs: Vec<String>,
    /// The index of the next log line, the `since` of the next snapshot.
    next_log_index: usize,
}

struct JobState {
    status: JobStatus,
    error: Option<String>,
    download: Option<Download>,
    logs: Vec<String>,
    /// The index of `logs[0]` among every line of the job.
    first_log_index: usize,
}

/// What a job runs on: the tiles and where their files go.
pub struct JobInput {
    pub tiles: Vec<IgnTile>,
    pub options: BatchOptions,
    pub config_path: PathBuf,
    pub input_dir: PathBuf,
    pub output_dir: PathBuf,
}

pub struct Job {
    tile_ids: Vec<String>,
    state: Mutex<JobState>,
    cancelled: AtomicBool,
    cancel_wakeup: Condvar,
    child: Mutex<Option<Child>>,
}

impl Job {
    /// Checks the options and starts the job in its own thread.
    pub fn start(input: JobInput, client: Client) -> Result<Arc<Self>> {
        let batch_args = input.options.args()?;
        let job = Arc::new(Self {
            tile_ids: input.tiles.iter().map(|tile| tile.id.clone()).collect(),
            state: Mutex::new(JobState {
                status: JobStatus::Downloading,
                error: None,
                download: None,
                logs: vec![],
                first_log_index: 0,
            }),
            cancelled: AtomicBool::new(false),
            cancel_wakeup: Condvar::new(),
            child: Mutex::new(None),
        });

        let running = job.clone();
        spawn(move || {
            let (status, error) = match running.run(&input, &batch_args, &client) {
                Ok(status) => (status, None),
                Err(_) if running.is_cancelled() => (JobStatus::Cancelled, None),
                Err(error) => {
                    let message = super::error_message(&error);
                    running.log(format!("Error: {message}"));
                    (JobStatus::Failed, Some(message))
                }
            };
            let mut state = running.state.lock().unwrap();
            state.status = status;
            state.error = error;
            state.download = None;
        });
        Ok(job)
    }

    pub fn status(&self) -> JobStatus {
        self.state.lock().unwrap().status
    }

    pub fn snapshot(&self, since: usize) -> JobSnapshot {
        let state = self.state.lock().unwrap();
        let start = since
            .saturating_sub(state.first_log_index)
            .min(state.logs.len());
        JobSnapshot {
            status: state.status,
            error: state.error.clone(),
            tiles: self.tile_ids.clone(),
            download: state.download.clone(),
            logs: state.logs[start..].to_vec(),
            next_log_index: state.first_log_index + state.logs.len(),
        }
    }

    /// Stops the download or kills the batch, whichever is running.
    pub fn cancel(&self) {
        {
            // Use the retry wait's mutex to avoid missing a cancellation
            // between testing the flag and going to sleep.
            let _state = self.state.lock().unwrap();
            self.cancelled.store(true, Ordering::SeqCst);
            self.cancel_wakeup.notify_all();
        }
        if let Some(child) = self.child.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    fn wait_for_retry(&self, delay: Duration) {
        let state = self.state.lock().unwrap();
        let _ = self
            .cancel_wakeup
            .wait_timeout_while(state, delay, |_| !self.is_cancelled())
            .unwrap();
    }

    fn log(&self, line: String) {
        info!("{line}");
        self.push_log(line);
    }

    fn push_log(&self, line: String) {
        let mut state = self.state.lock().unwrap();
        state.logs.push(line);
        if state.logs.len() > MAX_LOG_LINES {
            let dropped = state.logs.len() - MAX_LOG_LINES;
            state.logs.drain(..dropped);
            state.first_log_index += dropped;
        }
    }

    fn run(
        self: &Arc<Self>,
        input: &JobInput,
        batch_args: &[OsString],
        client: &Client,
    ) -> Result<JobStatus> {
        fs::create_dir_all(&input.input_dir).context(format!(
            "could not create input directory `{}`",
            input.input_dir.display()
        ))?;

        let files: Vec<(&str, PathBuf)> = input
            .tiles
            .iter()
            .flat_map(|tile| {
                [
                    (
                        tile.laz_url.as_str(),
                        input.input_dir.join(&tile.laz_file_name),
                    ),
                    (
                        tile.dem_url.as_str(),
                        input.input_dir.join(&tile.dem_file_name),
                    ),
                ]
            })
            .collect();

        for (index, (url, path)) in files.iter().enumerate() {
            if self.is_cancelled() {
                return Ok(JobStatus::Cancelled);
            }
            if path.exists() {
                self.log(format!("{} is already downloaded", path.display()));
                continue;
            }
            let download = Download {
                file_name: file_name(path),
                index: index + 1,
                count: files.len(),
                bytes: 0,
                total: None,
            };
            self.state.lock().unwrap().download = Some(download);
            self.log(format!("Downloading {}", path.display()));
            if !self.download_file(client, url, path)? {
                return Ok(JobStatus::Cancelled);
            }
        }

        if self.is_cancelled() {
            return Ok(JobStatus::Cancelled);
        }
        {
            let mut state = self.state.lock().unwrap();
            state.status = JobStatus::Processing;
            state.download = None;
        }
        self.run_batch(input, batch_args)
    }

    /// Downloads `url` to `path`, through a `.part` file renamed when
    /// complete. Returns `false` if the job was cancelled meanwhile.
    fn download_file(&self, client: &Client, url: &str, path: &Path) -> Result<bool> {
        let mut part_path = path.as_os_str().to_owned();
        part_path.push(".part");
        let part_path = PathBuf::from(part_path);

        let mut retries_left = DOWNLOAD_RETRIES;
        loop {
            if self.is_cancelled() {
                let _ = fs::remove_file(&part_path);
                return Ok(false);
            }
            let attempt = self.download_attempt(client, url, &part_path);
            // A cancellation during a blocking request takes precedence
            // over its eventual HTTP or read error.
            if self.is_cancelled() {
                let _ = fs::remove_file(&part_path);
                return Ok(false);
            }
            match attempt {
                Ok(completed) => {
                    if !completed {
                        let _ = fs::remove_file(&part_path);
                        return Ok(false);
                    }
                    break;
                }
                Err(Attempt::Fail(error)) => {
                    let _ = fs::remove_file(&part_path);
                    return Err(error);
                }
                Err(Attempt::Retry(delay, reason)) => {
                    if retries_left == 0 {
                        let _ = fs::remove_file(&part_path);
                        return Err(CassiniError::InvalidInput {
                            message: format!("could not download `{url}`: {reason}"),
                        });
                    }
                    retries_left -= 1;
                    self.log(format!(
                        "Download failed: {reason}. Retrying in {delay:.0?} ({retries_left} retries left)"
                    ));
                    self.wait_for_retry(delay);
                }
            }
        }

        fs::rename(&part_path, path).context(format!(
            "could not rename `{}` to `{}`",
            part_path.display(),
            path.display()
        ))?;
        Ok(true)
    }

    fn download_attempt(
        &self,
        client: &Client,
        url: &str,
        part_path: &Path,
    ) -> std::result::Result<bool, Attempt> {
        let mut response = client
            .get(url)
            .header(USER_AGENT, USER_AGENT_VALUE)
            .send()
            .map_err(|error| Attempt::Retry(DEFAULT_RETRY_DELAY, error.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            return Err(if is_transient_status(status.as_u16()) {
                Attempt::Retry(
                    retry_delay_from_response(&response),
                    format!("HTTP status {status}"),
                )
            } else {
                Attempt::Fail(CassiniError::HttpStatus {
                    url: url.to_owned(),
                    status: status.as_u16(),
                })
            });
        }

        // The DEM service answers errors with an XML document and status 200.
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_owned();
        if content_type.contains("xml") || content_type.contains("html") {
            let body = response.text().unwrap_or_default();
            return Err(Attempt::Fail(CassiniError::InvalidInput {
                message: format!(
                    "`{url}` returned `{content_type}` instead of a file: {}",
                    body.chars().take(500).collect::<String>()
                ),
            }));
        }

        if let Some(download) = self.state.lock().unwrap().download.as_mut() {
            download.total = response.content_length();
            download.bytes = 0;
        }

        let fail = |context: String| {
            move |error: std::io::Error| Attempt::Fail(CassiniError::operation(context, error))
        };
        let mut file = File::create(part_path)
            .map_err(fail(format!("could not create `{}`", part_path.display())))?;
        let mut buffer = vec![0; 1 << 18];
        loop {
            if self.is_cancelled() {
                return Ok(false);
            }
            let read = response
                .read(&mut buffer)
                .map_err(|error| Attempt::Retry(DEFAULT_RETRY_DELAY, error.to_string()))?;
            if read == 0 {
                break;
            }
            file.write_all(&buffer[..read])
                .map_err(fail(format!("could not write `{}`", part_path.display())))?;
            if let Some(download) = self.state.lock().unwrap().download.as_mut() {
                download.bytes += read as u64;
            }
        }
        file.flush()
            .map_err(fail(format!("could not write `{}`", part_path.display())))?;
        Ok(true)
    }

    fn run_batch(self: &Arc<Self>, input: &JobInput, batch_args: &[OsString]) -> Result<JobStatus> {
        let executable =
            std::env::current_exe().context("could not find the cassini executable")?;
        let mut command = Command::new(executable);
        command
            .arg("batch")
            .arg(&input.input_dir)
            .arg("--output-dir")
            .arg(&input.output_dir)
            .arg("--config")
            .arg(&input.config_path)
            .args(batch_args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        self.log(format!(
            "Running cassini {}",
            command
                .get_args()
                .map(|arg| arg.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        ));

        let mut child = command.spawn().context("could not start cassini batch")?;
        let readers: Vec<_> = [
            child
                .stdout
                .take()
                .map(|out| Box::new(out) as Box<dyn Read + Send>),
            child
                .stderr
                .take()
                .map(|err| Box::new(err) as Box<dyn Read + Send>),
        ]
        .into_iter()
        .flatten()
        .map(|output| {
            let job = self.clone();
            spawn(move || {
                for line in BufReader::new(output).lines().map_while(|line| line.ok()) {
                    eprintln!("{line}");
                    job.push_log(line);
                }
            })
        })
        .collect();

        *self.child.lock().unwrap() = Some(child);
        if self.is_cancelled() {
            self.cancel();
        }

        // Polled, so that `cancel` can take the child to kill it meanwhile.
        let exit_status = loop {
            let status = self
                .child
                .lock()
                .unwrap()
                .as_mut()
                .expect("the batch child process is stored")
                .try_wait()
                .context("could not wait for cassini batch")?;
            if let Some(status) = status {
                break status;
            }
            sleep(Duration::from_millis(200));
        };
        for reader in readers {
            let _ = reader.join();
        }
        *self.child.lock().unwrap() = None;

        if self.is_cancelled() {
            self.log("Cancelled".to_owned());
            Ok(JobStatus::Cancelled)
        } else if exit_status.success() {
            self.log("Done".to_owned());
            Ok(JobStatus::Done)
        } else {
            Err(CassiniError::InvalidInput {
                message: format!("cassini batch failed ({exit_status})"),
            })
        }
    }
}

enum Attempt {
    Retry(Duration, String),
    Fail(CassiniError),
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::TcpListener, time::Instant};
    use tiny_http::{Header, Response, Server};

    fn start_download(url: String, dir: &Path) -> Arc<Job> {
        Job::start(
            JobInput {
                tiles: vec![IgnTile {
                    id: "test".to_owned(),
                    min_x: 0,
                    max_y: 1000,
                    laz_url: url.clone(),
                    dem_url: url,
                    laz_file_name: "tile.laz".to_owned(),
                    dem_file_name: "tile.tif".to_owned(),
                    points: 0,
                    corners: [[0.0; 2]; 4],
                }],
                options: BatchOptions::default(),
                config_path: dir.join("config.json"),
                input_dir: dir.to_owned(),
                output_dir: dir.join("out"),
            },
            Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
        )
        .unwrap()
    }

    fn wait_until(condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !condition() {
            assert!(Instant::now() < deadline, "job did not respond in time");
            sleep(Duration::from_millis(5));
        }
    }

    fn assert_cancelled(job: &Job) {
        wait_until(|| !job.status().is_running());
        let snapshot = job.snapshot(0);
        assert_eq!(snapshot.status, JobStatus::Cancelled);
        assert!(snapshot.error.is_none());
        assert!(snapshot.download.is_none());
    }

    #[test]
    fn cancellation_during_a_request_takes_precedence_over_http_errors() {
        for status in [503, 404] {
            let dir = tempfile::tempdir().unwrap();
            let server = Server::http(("127.0.0.1", 0)).unwrap();
            let job = start_download(format!("http://{}/tile", server.server_addr()), dir.path());
            let request = server
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .unwrap();
            job.cancel();
            request
                .respond(
                    Response::empty(status)
                        .with_header(Header::from_bytes("Retry-After", "60").unwrap()),
                )
                .unwrap();
            assert_cancelled(&job);
            assert!(server.try_recv().unwrap().is_none());
            assert!(!dir.path().join("tile.laz").exists());
        }
    }

    #[test]
    fn cancellation_interrupts_retry_after_without_another_request() {
        let dir = tempfile::tempdir().unwrap();
        let server = Server::http(("127.0.0.1", 0)).unwrap();
        let job = start_download(format!("http://{}/tile", server.server_addr()), dir.path());
        server
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap()
            .respond(
                Response::empty(503).with_header(Header::from_bytes("Retry-After", "60").unwrap()),
            )
            .unwrap();
        wait_until(|| {
            job.snapshot(0)
                .logs
                .iter()
                .any(|line| line.contains("Retrying"))
        });
        job.cancel();
        assert_cancelled(&job);
        assert!(server.try_recv().unwrap().is_none());
    }

    #[test]
    fn cancellation_after_a_read_failure_removes_the_partial_download() {
        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let job = start_download(
            format!("http://{}/tile", listener.local_addr().unwrap()),
            dir.path(),
        );
        let (mut stream, _) = listener.accept().unwrap();
        // The request has no body, so it ends with its first blank line.
        let mut request = BufReader::new(&stream);
        let mut line = String::new();
        while line != "\r\n" {
            line.clear();
            assert_ne!(request.read_line(&mut line).unwrap(), 0);
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial")
            .unwrap();
        drop(stream);
        wait_until(|| {
            job.snapshot(0)
                .logs
                .iter()
                .any(|line| line.contains("Retrying"))
        });
        assert!(dir.path().join("tile.laz.part").exists());
        job.cancel();
        assert_cancelled(&job);
        assert!(!dir.path().join("tile.laz.part").exists());
        assert!(!dir.path().join("tile.laz").exists());
        listener.set_nonblocking(true).unwrap();
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}
