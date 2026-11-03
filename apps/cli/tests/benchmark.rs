use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
#[ignore]
fn fixture_process() {
    std::thread::sleep(std::time::Duration::from_secs(4));
}

#[test]
fn external_benchmark_records_samples() {
    let output = std::env::temp_dir().join(format!(
        "gdpl-test-{}.json",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let status = Command::new(env!("CARGO_BIN_EXE_gdpl"))
        .args(["benchmark", "run", "--executable"])
        .arg(std::env::current_exe().unwrap())
        .args([
            "--warmup-ms",
            "100",
            "--duration-ms",
            "900",
            "--interval-ms",
            "200",
            "--output",
        ])
        .arg(&output)
        .args(["--", "--ignored", "--exact", "fixture_process"])
        .status()
        .unwrap();
    assert!(status.success());
    let result: serde_json::Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(result["valid"], true);
    assert!(result["samples"].as_array().unwrap().len() >= 2);
    assert!(
        result["samples"][0]["process_memory_bytes"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(
        result["unavailable_metrics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x == "frame timing and FPS")
    );
    fs::remove_file(output).unwrap();
}

#[test]
fn early_exit_is_invalid() {
    let output = std::env::temp_dir().join(format!(
        "gdpl-early-{}.json",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let status = Command::new(env!("CARGO_BIN_EXE_gdpl"))
        .args(["benchmark", "run", "--executable"])
        .arg(env!("CARGO_BIN_EXE_gdpl"))
        .args([
            "--warmup-ms",
            "0",
            "--duration-ms",
            "1000",
            "--interval-ms",
            "200",
            "--output",
        ])
        .arg(&output)
        .args(["--", "doctor"])
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(2));
    let result: serde_json::Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(result["termination"], "process_exited");
    assert_eq!(result["valid"], false);
    fs::remove_file(output).unwrap();
}
