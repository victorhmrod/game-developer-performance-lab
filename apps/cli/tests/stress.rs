use std::{fs, process::Command};
#[test]
fn synthetic_scenario_runs_and_reports_real_process_samples() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("result.json");
    let config = temp.path().join("scenario.json");
    fs::write(&config, r#"{"scenario":{"id":"synthetic_memory","parameters":{},"initial_load":1,"load_increment":1,"maximum_load":1,"measurement_seconds":1,"maximum_duration_seconds":4,"maximum_memory_bytes":536870912,"minimum_fps":null},"strategy":"fixed","warmup_seconds":0,"repetitions":1,"maximum_cpu_percent":null,"sustained_samples":2}"#).unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_gdpl"))
        .args(["stress", "run", "--scenario"])
        .arg(&config)
        .arg("--output")
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success());
    let report: serde_json::Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(report["termination"], "completed");
    assert_eq!(report["cleanup"], "completed");
    assert_eq!(report["scenario_kind"], "synthetic_memory");
    assert!(report["maximum_validated_load"].is_null());
    assert!(
        report["stages"][0]["samples"][0]["process_memory_bytes"]
            .as_u64()
            .unwrap()
            > 0
    );
}

#[test]
fn stress_memory_stop_has_distinct_exit_code() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("result.json");
    let config = temp.path().join("scenario.json");
    fs::write(&config, r#"{"scenario":{"id":"synthetic_memory","parameters":{},"initial_load":1,"load_increment":1,"maximum_load":1,"measurement_seconds":1,"maximum_duration_seconds":4,"maximum_memory_bytes":1,"minimum_fps":null},"strategy":"fixed","warmup_seconds":0,"repetitions":1,"maximum_cpu_percent":null,"sustained_samples":2}"#).unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_gdpl"))
        .args(["stress", "run", "--scenario"])
        .arg(&config)
        .arg("--output")
        .arg(&output)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(4));
    let report: serde_json::Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(report["termination"], "memory_limit");
    assert_eq!(report["cleanup"], "completed");
    assert!(report["maximum_completed_load"].is_null());
}
