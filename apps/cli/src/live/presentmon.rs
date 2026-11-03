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

fn decode_help(bytes: &[u8]) -> Result<String, Box<dyn Error>> {
    if let Some(bytes) = bytes.strip_prefix(&[0xff, 0xfe]) {
        if bytes.len() % 2 != 0 {
            return Err("truncated UTF-16 help output".into());
        }
        let words = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes(*b))
            .collect::<Vec<_>>();
        return Ok(String::from_utf16(&words)?);
    }
    Ok(std::str::from_utf8(bytes)?
        .trim_start_matches('\u{feff}')
        .to_string())
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
        decode_help(&fs::read(help_path)?)?,
        decode_help(&fs::read(directory.join("help-stderr.txt"))?)?
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
        let mut command = Command::new(&self.0);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP);
        }
        Ok(OwnedChild(
            command
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

/// Best effort; sending a console event and observing child exit do not verify ETW cleanup.
pub(super) fn stop(child: &mut OwnedChild, timeout: Duration) -> String {
    if matches!(child.0.try_wait(), Ok(Some(_))) {
        return "already_exited; ETW cleanup unknown".into();
    }
    #[cfg(windows)]
    let request = {
        use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, GenerateConsoleCtrlEvent};
        if unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, child.0.id()) } == 0 {
            format!("request_failed: {}", std::io::Error::last_os_error())
        } else {
            "break_requested".to_owned()
        }
    };
    #[cfg(not(windows))]
    let request = "request_unsupported".to_owned();
    let start = Instant::now();
    while start.elapsed() < timeout {
        match child.0.try_wait() {
            Ok(Some(status)) => return format!("{request}; exited {status}; ETW cleanup unknown"),
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(error) => return format!("{request}; wait error: {error}; ETW cleanup unknown"),
        }
    }
    match child.0.kill() {
        Ok(()) => match child.0.wait() {
            Ok(status) => format!("{request}; forced termination {status}; ETW cleanup unknown"),
            Err(error) => {
                format!("{request}; forced termination wait error: {error}; ETW cleanup unknown")
            }
        },
        Err(error) => format!("{request}; forced termination failed: {error}; ETW cleanup unknown"),
    }
}

#[cfg(test)]
mod tests {
    use super::decode_help;
    #[test]
    fn help_decodes_utf16_and_rejects_truncation() {
        let mut bytes = vec![0xff, 0xfe];
        for word in "PresentMon 2.3\n--process_id".encode_utf16() {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        assert_eq!(decode_help(&bytes).unwrap(), "PresentMon 2.3\n--process_id");
        bytes.push(1);
        assert!(decode_help(&bytes).is_err());
    }
}
