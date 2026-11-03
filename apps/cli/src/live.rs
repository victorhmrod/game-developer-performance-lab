mod presentmon;
mod process;

use gdpl_analyzer::{FrameAnalysis, SwapChainActivity};
use presentmon::FrameCaptureBackend;
use serde::Serialize;
use std::{
    error::Error,
    fs::{self, File, OpenOptions},
    io::Write,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub struct Options {
    pub exe: Option<PathBuf>,
    pub pid: Option<u32>,
    pub process_name: Option<String>,
    pub presentmon: Option<PathBuf>,
    pub swap_chain: Option<String>,
    pub warmup: u64,
    pub duration: u64,
    pub shutdown_timeout: u64,
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
    target_owned: bool,
    target_creation_time_100ns: Option<u64>,
    warmup_seconds: u64,
    requested_capture_seconds: u64,
    started_unix_ms: u128,
    hardware: gdpl_core::Hardware,
    execution: String,
    measurement_validity: &'static str,
    error: Option<String>,
    analysis: Option<FrameAnalysis>,
    swap_chains: Vec<SwapChainActivity>,
    capture_integrity: &'static str,
    statistical_validity: &'static str,
    reproducibility: &'static str,
    performance_target: &'static str,
    diagnostics_error: Option<String>,
    etw_session_name: String,
    backend_shutdown: String,
    etw_cleanup: &'static str,
    warnings: Vec<&'static str>,
}

pub fn run(options: Options) -> Result<bool, Box<dyn Error>> {
    if !cfg!(windows) {
        return Err("live PresentMon capture is only available on Windows".into());
    }
    if options.duration == 0
        || options.duration > 3600
        || options.warmup > 3600
        || options.shutdown_timeout > 60
        || options.swap_chain.as_ref().is_some_and(String::is_empty)
    {
        return Err(
            "duration must be 1–3600 seconds, warmup at most 3600 seconds, shutdown timeout at most 60 seconds, and any swap chain must be nonempty"
                .into(),
        );
    }
    if options.exe.is_none() && !options.arguments.is_empty() {
        return Err("arguments after -- are only valid with --exe".into());
    }
    let exe = options.exe.as_ref().map(fs::canonicalize).transpose()?;
    let backend = presentmon::find_backend(options.presentmon.as_deref())?;
    let work = tempfile::tempdir()?;
    let backend_version = presentmon::probe(&backend, work.path())?;
    // Reserve the result path before launching anything; never overwrite previous evidence.
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&options.output)?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    ctrlc::set_handler(move || flag.store(true, Ordering::SeqCst))?;
    let started_unix_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let mut owned = match &exe {
        Some(exe) => Some(OwnedChild(
            Command::new(exe)
                .args(&options.arguments)
                .stdin(Stdio::null())
                .spawn()?,
        )),
        None => None,
    };
    let pid = if let Some(child) = &owned {
        child.0.id()
    } else if let Some(pid) = options.pid {
        pid
    } else {
        let name = options.process_name.as_deref().ok_or("target required")?;
        let system = sysinfo::System::new_all();
        let matches: Vec<_> = system
            .processes()
            .values()
            .filter(|p| p.name().to_string_lossy().eq_ignore_ascii_case(name))
            .collect();
        if matches.len() != 1 {
            return Err(format!(
                "process name matched {} processes; provide --pid",
                matches.len()
            )
            .into());
        }
        matches[0].pid().as_u32()
    };
    let target = process::ProcessHandle::open(pid)?;
    let session = format!("GDPL-{}-{started_unix_ms}", std::process::id());
    let mut execution = "completed".to_string();
    let mut swap_chains = Vec::new();
    let mut backend_shutdown = "not_started".to_string();
    let result = (|| -> Result<FrameAnalysis, Box<dyn Error>> {
        let warmup_start = Instant::now();
        while warmup_start.elapsed() < Duration::from_secs(options.warmup) {
            if cancelled.load(Ordering::SeqCst) {
                return Err("cancelled during warmup".into());
            }
            if target.exited()? {
                return Err("target exited during warmup".into());
            }
            thread::sleep(Duration::from_millis(25));
        }
        if target.exited()? {
            return Err("target exited before capture".into());
        }
        if cancelled.load(Ordering::SeqCst) {
            return Err("cancelled before capture".into());
        }
        target.verify_identity(pid)?;
        let csv = work.path().join("capture.csv");
        let mut capture = presentmon::PresentMon(backend.clone()).start(
            pid,
            &csv,
            &session,
            options.duration,
            work.path(),
        )?;
        let outcome = (|| -> Result<(), Box<dyn Error>> {
            target.verify_identity(pid)?;
            let start = Instant::now();
            loop {
                if cancelled.load(Ordering::SeqCst) {
                    return Err("cancelled during capture".into());
                }
                if target.exited()? {
                    return Err("target exited during capture".into());
                }
                if let Some(status) = capture.0.try_wait()? {
                    if !status.success() {
                        return Err(format!("PresentMon exited with {status}").into());
                    }
                    if start.elapsed() < Duration::from_secs(options.duration) {
                        return Err(
                            "PresentMon terminated before requested capture duration".into()
                        );
                    }
                    return Ok(());
                }
                if start.elapsed() > Duration::from_secs(options.duration + 30) {
                    return Err("PresentMon capture timed out".into());
                }
                thread::sleep(Duration::from_millis(25));
            }
        })();
        if outcome.is_err() {
            backend_shutdown =
                presentmon::stop(&mut capture, Duration::from_secs(options.shutdown_timeout));
        } else {
            backend_shutdown = "timed_exit".into();
        }
        outcome?;
        swap_chains = gdpl_analyzer::presentmon_swap_chains(File::open(&csv)?, pid)?;
        let selected = match &options.swap_chain {
            Some(address) => address.as_str(),
            None if swap_chains.len() == 1 => &swap_chains[0].address,
            None if swap_chains.is_empty() => {
                return Err("no swap chains observed for target process".into());
            }
            None => {
                return Err("multiple swap chains observed; select one using --swap-chain".into());
            }
        };
        gdpl_analyzer::analyze_presentmon(File::open(csv)?, pid, selected, 50.)
    })();
    let target_creation_time_100ns = target.creation_time();
    drop(target);
    drop(owned.take());
    let (analysis, error) = match result {
        Ok(analysis) => (Some(analysis), None),
        Err(error) => {
            execution = if cancelled.load(Ordering::SeqCst) {
                "cancelled"
            } else if error.to_string()
                == "multiple swap chains observed; select one using --swap-chain"
            {
                "inconclusive"
            } else {
                "failed"
            }
            .into();
            (None, Some(error.to_string()))
        }
    };
    let diagnostics_error = options.diagnostics.and_then(|directory| {
        let result = (|| -> Result<(), Box<dyn Error>> {
            fs::create_dir(&directory)?;
            for file in [
                "help.txt",
                "help-stderr.txt",
                "presentmon.log",
                "capture.csv",
            ] {
                let source = work.path().join(file);
                if source.exists() {
                    fs::copy(source, directory.join(file))?;
                }
            }
            Ok(())
        })();
        result
            .err()
            .map(|error| format!("{}: {error}", directory.display()))
    });
    let report = Report {
        schema_version: 1,
        gdpl_version: env!("CARGO_PKG_VERSION"),
        backend: "PresentMon console",
        backend_path: backend.display().to_string(),
        backend_version,
        target_executable: exe
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        target_arguments: options.arguments,
        target_pid: pid,
        target_owned: exe.is_some(),
        target_creation_time_100ns,
        warmup_seconds: options.warmup,
        requested_capture_seconds: options.duration,
        started_unix_ms,
        hardware: gdpl_hardware::inspect(),
        execution,
        measurement_validity: "inconclusive",
        error,
        analysis,
        swap_chains,
        capture_integrity: "unknown",
        statistical_validity: "inconclusive",
        reproducibility: "not_assessed",
        performance_target: "not_evaluated",
        diagnostics_error,
        etw_session_name: session,
        backend_shutdown,
        etw_cleanup: "unknown",
        warnings: vec![
            "Experimental backend; real PresentMon capture has not been verified",
            "Capture start and display timestamps, event loss, and coverage are not verified",
            "Warmup precedes launching PresentMon; capture startup introduces an unmeasured gap",
            "Only the selected process is targeted; launcher descendants are excluded",
            "CPU and memory metrics are unavailable for this capture",
            "Cancellation requests backend shutdown then may force termination; ETW cleanup is unverified",
            "Owned targets are terminated; attached targets and descendants are not terminated",
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
        assert!(presentmon::find_backend(Some(&temp.path().join("missing"))).is_err());
        assert!(presentmon::find_backend(Some(temp.path())).is_err());
    }
}
