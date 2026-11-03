#![cfg(windows)]

use std::{fs, path::Path, process::Command};

fn fixture(dir: &Path) -> std::path::PathBuf {
    let binary = dir.join(if cfg!(windows) { "mock.exe" } else { "mock" });
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock_capture.rs");
    assert!(
        Command::new("rustc")
            .arg(source)
            .arg("-o")
            .arg(&binary)
            .status()
            .unwrap()
            .success()
    );
    binary
}

#[test]
#[cfg(windows)]
fn supervised_mock_produces_inconclusive_report_and_cleans_up() {
    let temp = tempfile::tempdir().unwrap();
    let mock = fixture(temp.path());
    let output = temp.path().join("result.json");
    let diagnostics = temp.path().join("diagnostics");
    let result = Command::new(env!("CARGO_BIN_EXE_gdpl"))
        .args(["benchmark", "live", "--exe"])
        .arg(&mock)
        .arg("--presentmon")
        .arg(&mock)
        .args([
            "--swap-chain",
            "test",
            "--warmup",
            "0",
            "--duration",
            "1",
            "--output",
        ])
        .arg(&output)
        .arg("--diagnostics")
        .arg(&diagnostics)
        .args(["--", "target"])
        .output()
        .unwrap();
    assert_eq!(
        result.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(report["execution"], "completed");
    assert_eq!(report["measurement_validity"], "inconclusive");
    assert_eq!(report["analysis"]["distribution_ms"]["count"], 2);
    assert!(diagnostics.join("capture.csv").exists());
    let pid = sysinfo::Pid::from_u32(report["target_pid"].as_u64().unwrap() as u32);
    let system = sysinfo::System::new_all();
    assert!(
        system.process(pid).is_none(),
        "owned target was not terminated"
    );
}

#[test]
#[cfg(windows)]
fn early_target_exit_is_reported() {
    let temp = tempfile::tempdir().unwrap();
    let mock = fixture(temp.path());
    let output = temp.path().join("result.json");
    let result = Command::new(env!("CARGO_BIN_EXE_gdpl"))
        .args(["benchmark", "live", "--exe"])
        .arg(env!("CARGO_BIN_EXE_gdpl"))
        .arg("--presentmon")
        .arg(&mock)
        .args([
            "--swap-chain",
            "test",
            "--warmup",
            "1",
            "--duration",
            "1",
            "--output",
        ])
        .arg(&output)
        .args(["--", "doctor"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(report["execution"], "failed");
    assert!(report["error"].as_str().unwrap().contains("target exited"));
}
