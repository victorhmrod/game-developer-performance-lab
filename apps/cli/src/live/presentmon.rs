use super::OwnedChild;
use std::{
    error::Error,
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub(super) fn find_backend(explicit: Option<&Path>) -> Result<PathBuf, Box<dyn Error>> {
    if let Some(path) = explicit {
        let path = fs::canonicalize(path)?;
        if !path.is_file() {
            return Err("PresentMon path is not a file".into());
        }
        return Ok(path);
    }
    let paths = std::env::var_os("PATH").ok_or("PATH is unavailable; specify --presentmon")?;
    for dir in std::env::split_paths(&paths) {
        let candidate = dir.join(if cfg!(windows) {
            "PresentMon.exe"
        } else {
            "PresentMon"
        });
        if candidate.is_file() {
            return Ok(fs::canonicalize(candidate)?);
        }
    }
    Err("PresentMon not found on PATH; specify --presentmon".into())
}

// Inspect help before using the documented console switches. Never wait indefinitely.
pub(super) fn probe(path: &Path, directory: &Path) -> Result<Option<String>, Box<dyn Error>> {
    let help_path = directory.join("help.txt");
    let mut child = OwnedChild(
        Command::new(path)
            .arg("--help")
            .stdin(Stdio::null())
            .stdout(File::create(&help_path)?)
            .stderr(File::create(directory.join("help-stderr.txt"))?)
            .spawn()?,
    );
    let start = Instant::now();
    loop {
        if child.0.try_wait()?.is_some() {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            return Err("PresentMon --help timed out".into());
        }
        thread::sleep(Duration::from_millis(25));
    }
    let help = format!(
        "{}\n{}",
        fs::read_to_string(help_path)?,
        fs::read_to_string(directory.join("help-stderr.txt"))?
    );
    for option in [
        "--process_id",
        "--output_file",
        "--timed",
        "--terminate_after_timed",
        "--session_name",
        "--no_console_stats",
    ] {
        if !help.contains(option) {
            return Err(format!("PresentMon help does not advertise {option}").into());
        }
    }
    Ok(help
        .lines()
        .find_map(|line| line.trim().strip_prefix("PresentMon "))
        .map(str::to_owned))
}

pub(super) trait FrameCaptureBackend {
    fn start(
        &self,
        pid: u32,
        csv: &Path,
        session: &str,
        duration: u64,
        directory: &Path,
    ) -> Result<OwnedChild, Box<dyn Error>>;
}

pub(super) struct PresentMon(pub PathBuf);
impl FrameCaptureBackend for PresentMon {
    fn start(
        &self,
        pid: u32,
        csv: &Path,
        session: &str,
        duration: u64,
        directory: &Path,
    ) -> Result<OwnedChild, Box<dyn Error>> {
        let log = File::create(directory.join("presentmon.log"))?;
        Ok(OwnedChild(
            Command::new(&self.0)
                .args(["--process_id", &pid.to_string(), "--output_file"])
                .arg(csv)
                .args([
                    "--timed",
                    &duration.to_string(),
                    "--terminate_after_timed",
                    "--session_name",
                    session,
                    "--no_console_stats",
                ])
                .stdin(Stdio::null())
                .stdout(log.try_clone()?)
                .stderr(log)
                .spawn()?,
        ))
    }
}
