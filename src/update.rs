//! The `upgrade` subcommand, and the check for a newer version that runs in
//! the background when Cassini starts.
//!
//! Both use the GitHub Releases that dist publishes. `upgrade` reruns the
//! release's install script on the directory that the install receipt names,
//! as dist's own updater would.

use log::info;
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, USER_AGENT};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    env,
    fs::{self, OpenOptions},
    io::{IsTerminal, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{
    download::USER_AGENT_VALUE,
    error::{CassiniError, Result, ResultContext},
};

const REPOSITORY: &str = "NicoRio42/cassini";
const APP_NAME: &str = env!("CARGO_PKG_NAME");
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const INSTALLATION_GUIDE_URL: &str = "https://cassini-map.com/guides/installation-and-setup";
/// How long the result of a check is reused before checking again.
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const CHECK_TIMEOUT: Duration = Duration::from_secs(5);
const UPGRADE_TIMEOUT: Duration = Duration::from_secs(60);
const CACHE_FILE_NAME: &str = "update-check.json";
/// Disables the check for a newer version.
const NO_UPDATE_CHECK_VAR: &str = "CASSINI_NO_UPDATE_CHECK";
/// Set in the Docker image, which is upgraded by pulling a newer image.
const DOCKER_VAR: &str = "CASSINI_DOCKER";

#[cfg(windows)]
const INSTALLER_NAME: &str = "cassini-installer.ps1";
#[cfg(not(windows))]
const INSTALLER_NAME: &str = "cassini-installer.sh";

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
}

struct Release {
    tag: String,
    version: Version,
}

#[derive(Serialize, Deserialize)]
struct CheckCache {
    /// Last attempt, including failed or interrupted checks, in Unix seconds.
    #[serde(alias = "checked_at")]
    attempted_at: u64,
    /// The last successfully fetched version, if any.
    latest_version: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum InstallLayout {
    #[default]
    Flat,
    CargoHome,
}

/// The fields of the install receipt that dist's install scripts write.
#[derive(Deserialize)]
struct InstallReceipt {
    install_prefix: PathBuf,
    #[serde(default)]
    install_layout: InstallLayout,
    #[serde(default = "default_modify_path")]
    modify_path: bool,
}

fn default_modify_path() -> bool {
    true
}

/// A check for a newer version, running in the background.
pub struct UpdateCheck {
    /// The latest version that the previous check found.
    cached: Option<Version>,
    /// Gets the latest version, if the previous check is too old.
    receiver: Option<Receiver<Version>>,
}

impl UpdateCheck {
    /// Prints a notice if there is a newer version, without waiting for the
    /// check: if it hasn't finished, uses the result of the previous one.
    pub fn print_notice(self) {
        let latest = self
            .receiver
            .and_then(|receiver| receiver.try_recv().ok())
            .or(self.cached);
        print_notice_if_newer(latest.as_ref());
    }

    /// Prints a notice once the check finishes, if there is a newer version.
    /// For the commands that run until they are stopped.
    pub fn print_notice_when_done(self) {
        thread::spawn(move || {
            let latest = self
                .receiver
                .and_then(|receiver| receiver.recv().ok())
                .or(self.cached);
            print_notice_if_newer(latest.as_ref());
        });
    }
}

/// Starts checking for a newer version in the background, at most once per
/// `CHECK_INTERVAL`. Returns `None` when the check is disabled, or when
/// Cassini doesn't run in a terminal, as in scripts or CI.
pub fn start_update_check() -> Option<UpdateCheck> {
    if env_flag(NO_UPDATE_CHECK_VAR)
        || env_flag(DOCKER_VAR)
        || env_flag("CI")
        || !std::io::stderr().is_terminal()
    {
        return None;
    }

    let cache_path = cache_dir().map(|dir| dir.join(CACHE_FILE_NAME));
    Some(start_cached_check(cache_path, unix_now(), || {
        fetch_latest_release(CHECK_TIMEOUT)
    }))
}

fn start_cached_check(
    cache_path: Option<PathBuf>,
    now: u64,
    fetch: impl FnOnce() -> Result<Release> + Send + 'static,
) -> UpdateCheck {
    let cache = cache_path.as_deref().and_then(read_cache);
    let cached = cache
        .as_ref()
        .and_then(|cache| cache.latest_version.as_deref())
        .and_then(|version| Version::parse(version).ok());

    if cache
        .as_ref()
        .is_some_and(|cache| is_fresh(cache.attempted_at, now))
    {
        return UpdateCheck {
            cached,
            receiver: None,
        };
    }

    let mut cache = CheckCache {
        attempted_at: now,
        latest_version: cache.and_then(|cache| cache.latest_version),
    };
    // Save before spawning: even a short-lived command or failed request must
    // not cause the next invocation to immediately try again.
    if let Some(path) = &cache_path {
        write_cache(path, &cache);
    }

    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        // A failed check is ignored: it must not get in the way.
        if let Ok(release) = fetch() {
            cache.latest_version = Some(release.version.to_string());
            if let Some(path) = &cache_path {
                write_cache(path, &cache);
            }
            let _ = sender.send(release.version);
        }
    });

    UpdateCheck {
        cached,
        receiver: Some(receiver),
    }
}

/// Installs the latest version of Cassini, by running the install script of
/// its release on the directory that the current one was installed in.
pub fn upgrade() -> Result<()> {
    if env_flag(DOCKER_VAR) {
        return Err(CassiniError::Upgrade {
            message: "this is Cassini's Docker image: pull the latest image instead".to_owned(),
        });
    }

    let receipt = load_receipt()?;
    let exe = env::current_exe().context("could not find the path of the Cassini executable")?;
    check_executable_is_installed(&receipt, &exe)?;

    info!("Looking for the latest version of Cassini");
    let release = fetch_latest_release(UPGRADE_TIMEOUT)?;
    if let Some(dir) = cache_dir() {
        write_cache(
            &dir.join(CACHE_FILE_NAME),
            &CheckCache {
                attempted_at: unix_now(),
                latest_version: Some(release.version.to_string()),
            },
        );
    }

    let current = current_version();
    if release.version <= current {
        info!("Cassini {current} is the latest version");
        return Ok(());
    }

    info!("Upgrading Cassini from {current} to {}", release.version);
    let installer = download_installer(&release.tag)?;
    let result = run_installer(&installer, &receipt);
    let _ = fs::remove_file(&installer);
    result?;

    info!("Cassini upgraded to {}", release.version);
    Ok(())
}

/// Removes the executable that `upgrade` renamed on Windows, which can't
/// delete it while it runs.
pub fn remove_previous_executable() {
    if cfg!(windows) {
        if let Ok(exe) = env::current_exe() {
            let _ = fs::remove_file(previous_executable_path(&exe));
        }
    }
}

fn print_notice_if_newer(latest: Option<&Version>) {
    let current = current_version();
    if let Some(latest) = latest.filter(|latest| **latest > current) {
        eprintln!(
            "\nCassini {latest} is available (you have {current}). Run `cassini upgrade` to install it."
        );
    }
}

fn current_version() -> Version {
    Version::parse(CURRENT_VERSION).expect("the package version should be valid semver")
}

fn env_flag(name: &str) -> bool {
    env::var_os(name).is_some_and(|value| !value.is_empty())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// Whether a check made at `checked_at` can be reused at `now`. A check from
/// the future, after a clock change, is not.
fn is_fresh(checked_at: u64, now: u64) -> bool {
    checked_at <= now && now - checked_at < CHECK_INTERVAL.as_secs()
}

/// Parses a release tag such as `0.19.0` or `v0.19.0`.
fn parse_tag(tag: &str) -> Option<Version> {
    Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()
}

fn fetch_latest_release(timeout: Duration) -> Result<Release> {
    let url = format!("https://api.github.com/repos/{REPOSITORY}/releases/latest");
    let client = Client::builder()
        .timeout(timeout)
        .build()
        .context("could not create the HTTP client")?;
    let response = client
        .get(&url)
        .header(USER_AGENT, USER_AGENT_VALUE)
        .header(ACCEPT, "application/vnd.github+json")
        .send()
        .context("could not query the latest release of Cassini")?;
    let status = response.status();
    if !status.is_success() {
        return Err(CassiniError::HttpStatus {
            url,
            status: status.as_u16(),
        });
    }
    let body = response
        .bytes()
        .context("could not read the latest release of Cassini")?;
    let release: GithubRelease =
        serde_json::from_slice(&body).context("could not parse the latest release of Cassini")?;
    let version = parse_tag(&release.tag_name).ok_or_else(|| CassiniError::Upgrade {
        message: format!(
            "the latest release has an invalid tag `{}`",
            release.tag_name
        ),
    })?;

    Ok(Release {
        tag: release.tag_name,
        version,
    })
}

/// Where the result of the last check is kept: Cassini's directory in the
/// user's cache directory.
fn cache_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        env::var_os("HOME").map(|home| PathBuf::from(home).join("Library").join("Caches"))
    } else {
        env::var_os("XDG_CACHE_HOME")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
    };
    base.map(|base| base.join(APP_NAME))
}

fn read_cache(path: &Path) -> Option<CheckCache> {
    let content = fs::read(path).ok()?;
    serde_json::from_slice(&content).ok()
}

fn write_cache(path: &Path, cache: &CheckCache) {
    if let (Some(dir), Ok(content)) = (path.parent(), serde_json::to_vec(cache)) {
        let _ = fs::create_dir_all(dir).and_then(|()| fs::write(path, content));
    }
}

/// The directories where the install scripts write the install receipt, in
/// the order dist's updater looks in them.
fn receipt_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![];
    if let Some(dir) = env::var_os("XDG_CONFIG_HOME").filter(|path| !path.is_empty()) {
        dirs.push(PathBuf::from(dir).join(APP_NAME));
    }
    let home = if cfg!(windows) {
        env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
    };
    if let Some(home) = home {
        dirs.push(home.join(APP_NAME));
    }
    dirs
}

fn not_installed_by_script() -> CassiniError {
    CassiniError::Upgrade {
        message: format!(
            "Cassini wasn't installed with the install script. Install the latest version as explained in {INSTALLATION_GUIDE_URL}"
        ),
    }
}

fn load_receipt() -> Result<InstallReceipt> {
    let path = receipt_dirs()
        .into_iter()
        .map(|dir| dir.join(format!("{APP_NAME}-receipt.json")))
        .find(|path| path.exists())
        .ok_or_else(not_installed_by_script)?;
    let content = fs::read(&path).context(format!(
        "could not read the install receipt `{}`",
        path.display()
    ))?;
    serde_json::from_slice(&content).context(format!(
        "could not parse the install receipt `{}`",
        path.display()
    ))
}

/// Checks that the running executable is the one that the install script
/// installed, rather than for example one built from source while an
/// installed one is also present.
fn check_executable_is_installed(receipt: &InstallReceipt, exe: &Path) -> Result<()> {
    let exe = exe
        .canonicalize()
        .context("could not find the path of the Cassini executable")?;
    let install_dir = match receipt.install_layout {
        InstallLayout::Flat => receipt.install_prefix.clone(),
        InstallLayout::CargoHome => receipt.install_prefix.join("bin"),
    };
    let install_dir = install_dir.canonicalize().unwrap_or(install_dir);

    if exe.parent() != Some(install_dir.as_path()) {
        return Err(CassiniError::Upgrade {
            message: format!(
                "this executable (`{}`) isn't the one that the install script installed in `{}`. Install the latest version as explained in {INSTALLATION_GUIDE_URL}",
                exe.display(),
                install_dir.display()
            ),
        });
    }

    Ok(())
}

fn download_installer(tag: &str) -> Result<PathBuf> {
    let url = format!("https://github.com/{REPOSITORY}/releases/download/{tag}/{INSTALLER_NAME}");
    let client = Client::builder()
        .timeout(UPGRADE_TIMEOUT)
        .build()
        .context("could not create the HTTP client")?;
    let response = client
        .get(&url)
        .header(USER_AGENT, USER_AGENT_VALUE)
        .send()
        .context("could not download the install script")?;
    let status = response.status();
    if !status.is_success() {
        return Err(CassiniError::HttpStatus {
            url,
            status: status.as_u16(),
        });
    }
    let script = response
        .bytes()
        .context("could not download the install script")?;

    let path = env::temp_dir().join(format!("{}-{INSTALLER_NAME}", std::process::id()));
    // `create_new` doesn't follow a file or link that is already there.
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .and_then(|mut file| file.write_all(&script))
        .context(format!(
            "could not write the install script to `{}`",
            path.display()
        ))?;
    Ok(path)
}

fn previous_executable_path(exe: &Path) -> PathBuf {
    exe.with_extension("previous.exe")
}

fn run_installer(installer: &Path, receipt: &InstallReceipt) -> Result<()> {
    let mut command = if cfg!(windows) {
        let mut command = Command::new("powershell");
        command
            .args(["-ExecutionPolicy", "ByPass", "-File"])
            .arg(installer)
            // PowerShell Core's modules break Windows PowerShell's:
            // https://github.com/PowerShell/PowerShell/issues/18530
            .env_remove("PSModulePath");
        command
    } else {
        let mut command = Command::new("sh");
        command.arg(installer);
        command
    };
    command.env("CASSINI_INSTALL_DIR", &receipt.install_prefix);
    if matches!(receipt.install_layout, InstallLayout::CargoHome) {
        // dist recognizes this layout by comparing the prefix with CARGO_HOME.
        // Preserve it even if the user's CARGO_HOME has changed since install.
        command.env("CARGO_HOME", &receipt.install_prefix);
    }
    if !receipt.modify_path {
        command.env("CASSINI_NO_MODIFY_PATH", "1");
    }

    // Windows can't overwrite a running executable, but can rename it.
    let renamed = if cfg!(windows) {
        let exe =
            env::current_exe().context("could not find the path of the Cassini executable")?;
        let previous = previous_executable_path(&exe);
        let _ = fs::remove_file(&previous);
        fs::rename(&exe, &previous).context(format!("could not rename `{}`", exe.display()))?;
        Some((previous, exe))
    } else {
        None
    };

    let result = command.status();
    let succeeded = result.as_ref().is_ok_and(|status| status.success());
    if let Some((previous, exe)) = renamed {
        if !succeeded {
            let _ = fs::rename(previous, exe);
        }
    }

    let status = result.context("could not run the install script")?;
    if !status.success() {
        return Err(CassiniError::Upgrade {
            message: format!("the install script failed ({status})"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use semver::Version;

    #[test]
    fn failed_checks_are_throttled_with_or_without_a_previous_version() {
        for previous_version in [None, Some("0.20.0")] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(CACHE_FILE_NAME);
            if let Some(version) = previous_version {
                // Also exercise migration from the original cache format.
                fs::write(
                    &path,
                    format!(r#"{{"checked_at":1000,"latest_version":"{version}"}}"#),
                )
                .unwrap();
            }
            let now = 1_000 + CHECK_INTERVAL.as_secs();
            let check = start_cached_check(Some(path.clone()), now, || {
                Err(CassiniError::Upgrade {
                    message: "simulated network failure".into(),
                })
            });
            assert!(check.receiver.unwrap().recv().is_err());
            let cache = read_cache(&path).unwrap();
            assert_eq!(cache.attempted_at, now);
            assert_eq!(cache.latest_version.as_deref(), previous_version);

            let check = start_cached_check(Some(path.clone()), now + 1, || {
                panic!("a failed check must not be retried before the interval expires")
            });
            assert!(check.receiver.is_none());
            assert_eq!(check.cached, previous_version.and_then(parse_tag));

            let check =
                start_cached_check(Some(path.clone()), now + CHECK_INTERVAL.as_secs(), || {
                    Ok(Release {
                        tag: "0.21.0".into(),
                        version: Version::new(0, 21, 0),
                    })
                });
            assert_eq!(
                check.receiver.unwrap().recv().unwrap(),
                Version::new(0, 21, 0)
            );
            assert_eq!(
                read_cache(&path).unwrap().latest_version.as_deref(),
                Some("0.21.0")
            );
        }
    }

    #[test]
    fn records_an_attempt_before_the_background_request_finishes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CACHE_FILE_NAME);
        let (finish, wait) = mpsc::channel();
        let check = start_cached_check(Some(path.clone()), 1_000, move || {
            wait.recv().unwrap();
            Ok(Release {
                tag: "0.20.0".into(),
                version: Version::new(0, 20, 0),
            })
        });
        let cache = read_cache(&path).unwrap();
        assert_eq!(cache.attempted_at, 1_000);
        assert!(cache.latest_version.is_none());
        let next = start_cached_check(Some(path), 1_001, || {
            panic!("an unfinished check must also be throttled")
        });
        assert!(next.receiver.is_none());
        finish.send(()).unwrap();
        assert_eq!(
            check.receiver.unwrap().recv().unwrap(),
            Version::new(0, 20, 0)
        );
    }

    #[test]
    fn reuses_a_fresh_legacy_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CACHE_FILE_NAME);
        fs::write(&path, r#"{"checked_at":1000,"latest_version":"0.20.0"}"#).unwrap();
        let check = start_cached_check(Some(path), 1_001, || {
            panic!("a fresh legacy cache should still be reused")
        });
        assert!(check.receiver.is_none());
        assert_eq!(check.cached, Some(Version::new(0, 20, 0)));
    }

    #[test]
    fn validates_the_executable_directory_for_each_install_layout() {
        for layout in ["flat", "cargo-home"] {
            let dir = tempfile::tempdir().unwrap();
            let prefix = dir.path().join("install");
            fs::create_dir_all(prefix.join("bin")).unwrap();
            let receipt: InstallReceipt = serde_json::from_value(serde_json::json!({
                "install_prefix": prefix,
                "install_layout": layout,
            }))
            .unwrap();
            let flat_exe = prefix.join("cassini");
            let cargo_exe = prefix.join("bin").join("cassini");
            let unrelated_exe = dir.path().join("cassini");
            for exe in [&flat_exe, &cargo_exe, &unrelated_exe] {
                fs::write(exe, b"test executable").unwrap();
            }
            let (installed, wrong_layout) = if layout == "flat" {
                (&flat_exe, &cargo_exe)
            } else {
                (&cargo_exe, &flat_exe)
            };
            check_executable_is_installed(&receipt, installed).unwrap();
            assert!(check_executable_is_installed(&receipt, wrong_layout).is_err());
            assert!(check_executable_is_installed(&receipt, &unrelated_exe).is_err());
        }
    }

    #[test]
    #[cfg(unix)]
    fn passes_the_original_cargo_home_prefix_to_the_installer() {
        let dir = tempfile::tempdir().unwrap();
        let installer = dir.path().join("installer.sh");
        fs::write(
            &installer,
            "printf '%s\\n' \"$CASSINI_INSTALL_DIR\" \"$CARGO_HOME\" \"$CASSINI_NO_MODIFY_PATH\" > \"$0.env\"\n",
        )
        .unwrap();
        let receipt = InstallReceipt {
            install_prefix: dir.path().join("cargo home"),
            install_layout: InstallLayout::CargoHome,
            modify_path: false,
        };
        run_installer(&installer, &receipt).unwrap();
        let prefix = receipt.install_prefix.display();
        assert_eq!(
            fs::read_to_string(dir.path().join("installer.sh.env")).unwrap(),
            format!("{prefix}\n{prefix}\n1\n"),
        );
        fs::write(&installer, "exit 7\n").unwrap();
        assert!(matches!(
            run_installer(&installer, &receipt),
            Err(CassiniError::Upgrade { .. })
        ));
    }

    #[test]
    fn parses_release_tags() {
        assert_eq!(parse_tag("0.19.0"), Some(Version::new(0, 19, 0)));
        assert_eq!(parse_tag("v0.20.1"), Some(Version::new(0, 20, 1)));
        assert_eq!(
            parse_tag("0.20.0-rc.1"),
            Some(Version::parse("0.20.0-rc.1").unwrap())
        );
        assert_eq!(parse_tag("latest"), None);
    }

    #[test]
    fn a_prerelease_is_older_than_its_release() {
        assert!(parse_tag("0.20.0-rc.1").unwrap() < parse_tag("0.20.0").unwrap());
        assert!(parse_tag("0.20.0-rc.1").unwrap() > parse_tag("0.19.0").unwrap());
    }

    #[test]
    fn reuses_a_check_for_a_day() {
        let day = CHECK_INTERVAL.as_secs();
        assert!(is_fresh(1_000, 1_000));
        assert!(is_fresh(1_000, 1_000 + day - 1));
        assert!(!is_fresh(1_000, 1_000 + day));
        assert!(!is_fresh(1_000, 999));
    }

    #[test]
    fn reads_the_install_receipt_of_dist() {
        let receipt: InstallReceipt = serde_json::from_str(
            r#"{"install_prefix":"/home/user/.local/bin","install_layout":"flat","binaries":["cassini"],"cdylibs":[],"cstaticlibs":[],"source":{"release_type":"github","owner":"NicoRio42","name":"cassini","app_name":"cassini"},"version":"0.19.0","provider":{"source":"cargo-dist","version":"0.34.0"},"binary_aliases":{},"modify_path":false}"#,
        )
        .unwrap();
        assert_eq!(
            receipt.install_prefix.to_str(),
            Some("/home/user/.local/bin")
        );
        assert!(!receipt.modify_path);
    }
}
