use gdpl_analyzer::FrameAnalysis;
use serde::Serialize;
use std::{
    error::Error,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub struct Options {
    pub exe: PathBuf,
    pub presentmon: Option<PathBuf>,
    pub swap_chain: String,
    pub warmup: u64,
    pub duration: u64,
    pub output: PathBuf,
    pub diagnostics: Option<PathBuf>,
    pub arguments: Vec<String>,
}

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    gdpl_version: &'static str,
    backend: &'static str,
    backend_path: String,
    backend_version: Option<String>,
    target_executable: String,
    target_arguments: Vec<String>,
    target_pid: u32,
    warmup_seconds: u64,
    requested_capture_seconds: u64,
    started_unix_ms: u128,
    hardware: gdpl_core::Hardware,
    execution: String,
    measurement_validity: &'static str,
    error: Option<String>,
    analysis: Option<FrameAnalysis>,
    warnings: Vec<&'static str>,
}

fn find_backend(explicit: Option<&Path>) -> Result<PathBuf, Box<dyn Error>> {
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
fn probe(path: &Path, directory: &Path) -> Result<(), Box<dyn Error>> {
    let help_path = directory.join("help.txt");
    let mut child = OwnedChild(
        Command::new(path)
            .arg("--help")
            .stdin(Stdio::null())
            .stdout(File::create(&help_path)?)
            .stderr(Stdio::null())
            .spawn()?,
    );
    let start = Instant::now();
    loop {
        if let Some(status) = child.0.try_wait()? {
            if !status.success() {
                return Err("PresentMon --help failed".into());
            }
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            return Err("PresentMon --help timed out".into());
        }
        thread::sleep(Duration::from_millis(25));
    }
    let help = fs::read_to_string(help_path)?;
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
    Ok(())
}

pub fn run(options: Options) -> Result<bool, Box<dyn Error>> {
    if !cfg!(windows) {
        return Err("live PresentMon capture is only available on Windows".into());
    }
    if options.duration == 0
        || options.duration > 3600
        || options.warmup > 3600
        || options.swap_chain.is_empty()
    {
        return Err(
            "duration must be 1–3600 seconds, warmup at most 3600 seconds, and swap chain nonempty"
                .into(),
        );
    }
    let exe = fs::canonicalize(&options.exe)?;
    let backend = find_backend(options.presentmon.as_deref())?;
    let work = tempfile::tempdir()?;
    probe(&backend, work.path())?;
    // Reserve the result path before launching anything; never overwrite previous evidence.
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&options.output)?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    ctrlc::set_handler(move || flag.store(true, Ordering::SeqCst))?;
    let started_unix_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let mut target = OwnedChild(
        Command::new(&exe)
            .args(&options.arguments)
            .stdin(Stdio::null())
            .spawn()?,
    );
    let pid = target.0.id();
    let mut execution = "completed".to_string();
    let result = (|| -> Result<FrameAnalysis, Box<dyn Error>> {
        let warmup_start = Instant::now();
        while warmup_start.elapsed() < Duration::from_secs(options.warmup) {
            if cancelled.load(Ordering::SeqCst) {
                return Err("cancelled during warmup".into());
            }
            if target.0.try_wait()?.is_some() {
                return Err("target exited during warmup".into());
            }
            thread::sleep(Duration::from_millis(25));
        }
        if target.0.try_wait()?.is_some() {
            return Err("target exited before capture".into());
        }
        if cancelled.load(Ordering::SeqCst) {
            return Err("cancelled before capture".into());
        }
        let csv = work.path().join("capture.csv");
        let log = File::create(work.path().join("presentmon.log"))?;
        let session = format!("GDPL-{}-{started_unix_ms}", std::process::id());
        let mut capture = OwnedChild(
            Command::new(&backend)
                .args(["--process_id", &pid.to_string(), "--output_file"])
                .arg(&csv)
                .args([
                    "--timed",
                    &options.duration.to_string(),
                    "--terminate_after_timed",
                    "--session_name",
                    &session,
                    "--no_console_stats",
                ])
                .stdin(Stdio::null())
                .stdout(log.try_clone()?)
                .stderr(log)
                .spawn()?,
        );
        let start = Instant::now();
        loop {
            if cancelled.load(Ordering::SeqCst) {
                return Err("cancelled during capture".into());
            }
            if target.0.try_wait()?.is_some() {
                return Err("target exited during capture".into());
            }
            if let Some(status) = capture.0.try_wait()? {
                if !status.success() {
                    return Err(format!("PresentMon exited with {status}").into());
                }
                if start.elapsed() < Duration::from_secs(options.duration) {
                    return Err("PresentMon terminated before requested capture duration".into());
                }
                break;
            }
            if start.elapsed() > Duration::from_secs(options.duration + 30) {
                return Err("PresentMon capture timed out".into());
            }
            thread::sleep(Duration::from_millis(25));
        }
        gdpl_analyzer::analyze_presentmon(File::open(csv)?, pid, &options.swap_chain, 50.)
    })();
    drop(target);
    let (analysis, error) = match result {
        Ok(analysis) => (Some(analysis), None),
        Err(error) => {
            execution = if cancelled.load(Ordering::SeqCst) {
                "cancelled"
            } else {
                "failed"
            }
            .into();
            (None, Some(error.to_string()))
        }
    };
    if let Some(directory) = options.diagnostics {
        fs::create_dir(&directory)?;
        for file in ["help.txt", "presentmon.log", "capture.csv"] {
            let source = work.path().join(file);
            if source.exists() {
                fs::copy(source, directory.join(file))?;
            }
        }
    }
    let report = Report {
        schema_version: 1,
        gdpl_version: env!("CARGO_PKG_VERSION"),
        backend: "PresentMon console",
        backend_path: backend.display().to_string(),
        backend_version: None,
        target_executable: exe.display().to_string(),
        target_arguments: options.arguments,
        target_pid: pid,
        warmup_seconds: options.warmup,
        requested_capture_seconds: options.duration,
        started_unix_ms,
        hardware: gdpl_hardware::inspect(),
        execution,
        measurement_validity: "inconclusive",
        error,
        analysis,
        warnings: vec![
            "Experimental backend; real PresentMon capture has not been verified",
            "Capture start and display timestamps, event loss, and coverage are not verified",
            "Warmup precedes launching PresentMon; capture startup introduces an unmeasured gap",
            "Only the launched process is targeted; launcher descendants are excluded",
            "CPU and memory metrics are unavailable for this capture",
            "Cancellation force-terminates owned processes; descendants are not terminated",
        ],
    };
    serde_json::to_writer_pretty(&mut output, &report)?;
    output.write_all(b"\n")?;
    output.sync_all()?;
    println!(
        "Capture {}; measurement validity: inconclusive",
        report.execution
    );
    // Inconclusive capture must not pass a CI validity gate.
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_missing_backend_and_directory() {
        let temp = tempfile::tempdir().unwrap();
        assert!(find_backend(Some(&temp.path().join("missing"))).is_err());
        assert!(find_backend(Some(temp.path())).is_err());
    }
}
