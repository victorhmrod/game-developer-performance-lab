use serde_json::{Value, json};
use std::{
    fs,
    process::{Command, Stdio},
};

fn config(external: Value, maximum_load: u64) -> Value {
    json!({"scenario":{"id":"external_process","parameters":{},"initial_load":1,"load_increment":1,"maximum_load":maximum_load,"measurement_seconds":1,"maximum_duration_seconds":10,"maximum_memory_bytes":536870912,"minimum_fps":null},"strategy":if maximum_load == 1 { "fixed" } else { "linear" },"warmup_seconds":0,"repetitions":1,"maximum_cpu_percent":null,"sustained_samples":2,"external":external})
}
fn execute(dir: &std::path::Path, config: Value) -> (std::process::ExitStatus, Value) {
    let input = dir.join("config.json");
    let output = dir.join("report.json");
    fs::write(&input, serde_json::to_vec(&config).unwrap()).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_gdpl"))
        .args(["stress", "run", "--scenario"])
        .arg(input)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    let report = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    (result.status, report)
}
#[test]
fn controls_and_measures_real_external_process() {
    let temp = tempfile::tempdir().unwrap();
    let (status, report) = execute(
        temp.path(),
        config(
            json!({"mode":"launch","executable":env!("CARGO_BIN_EXE_gdpl-workload"),"arguments":[],"working_directory":null,"controlled":true,"timeout_ms":2000}),
            2,
        ),
    );
    assert!(status.success());
    assert_eq!(report["termination"], "completed");
    assert_eq!(report["provider_info"]["name"], "synthetic reference");
    assert!(
        report["provider_metric_metadata"]["allocated_bytes"]
            .as_str()
            .unwrap()
            .contains("bytes")
    );
    assert_eq!(report["cleanup"], "completed");
    assert_eq!(report["stages"].as_array().unwrap().len(), 2);
    assert_eq!(report["maximum_acknowledged_load"], 2);
    assert_eq!(report["maximum_completed_load"], 2);
    assert!(report["maximum_validated_load"].is_null());
    assert_eq!(report["stages"][0]["acknowledged_load"], 1);
    assert!(report["stages"][0]["samples"][0]["process_cpu_percent"].is_null());
    assert!(report["stages"][1]["samples"][0]["process_cpu_percent"].is_null());
    assert_eq!(
        report["stages"][0]["samples"][0]["provider_metrics"]["allocated_bytes"].as_f64(),
        Some(1024.0)
    );
    assert!(report["stages"][0]["peak_memory_bytes"].as_u64().unwrap() > 0);
    assert_eq!(
        report["stages"][0]["provider_metric_summaries"]["allocated_bytes"]["count"],
        report["stages"][0]["sample_count"]
    );
    assert!(report["target_pid"].as_u64().unwrap() != std::process::id() as u64);
}
#[test]
fn attached_process_is_only_observed_and_remains_alive() {
    let temp = tempfile::tempdir().unwrap();
    let mut target = Command::new(env!("CARGO_BIN_EXE_gdpl-workload"))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let (status, report) = execute(
        temp.path(),
        config(json!({"mode":"attach", "pid":target.id()}), 1),
    );
    assert!(status.success());
    assert_eq!(report["target_pid"], target.id());
    assert!(report["maximum_acknowledged_load"].is_null());
    assert!(report["stages"][0]["acknowledged_load"].is_null());
    assert!(target.try_wait().unwrap().is_none());
    target.kill().unwrap();
    target.wait().unwrap();
}
#[test]
fn rejects_noncooperating_child_without_faking_acknowledgement() {
    let temp = tempfile::tempdir().unwrap();
    let (status, report) = execute(
        temp.path(),
        config(
            json!({"mode":"launch","executable":env!("CARGO_BIN_EXE_gdpl"),"arguments":["doctor"],"working_directory":null,"controlled":true,"timeout_ms":500}),
            2,
        ),
    );
    assert_eq!(status.code(), Some(5));
    assert_eq!(report["termination"], "initialization_failed");
    assert!(report["maximum_acknowledged_load"].is_null());
}

fn fault_fixture(dir: &std::path::Path) -> std::path::PathBuf {
    let binary = dir.join(if cfg!(windows) { "fault.exe" } else { "fault" });
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stress_fault.rs");
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
fn protocol_failures_remain_distinct_and_incomplete() {
    for mode in ["bad_version", "bad_metadata", "hang", "bad_ack", "early"] {
        let temp = tempfile::tempdir().unwrap();
        let fixture = fault_fixture(temp.path());
        let (status, report) = execute(
            temp.path(),
            config(
                json!({"mode":"launch","executable":fixture,"arguments":[mode],"working_directory":null,"controlled":true,"timeout_ms":300}),
                1,
            ),
        );
        assert_eq!(status.code(), Some(5), "{mode}: {report}");
        let termination = report["termination"].as_str().unwrap();
        match mode {
            "bad_version" | "bad_metadata" | "hang" => {
                assert_eq!(termination, "initialization_failed")
            }
            "bad_ack" => {
                assert_eq!(termination, "workload_failed");
                assert!(report["stages"][0]["acknowledged_load"].is_null());
            }
            "early" => {
                assert_eq!(termination, "target_failed");
                assert_eq!(report["stages"][0]["status"], "failed");
            }
            _ => unreachable!(),
        }
        assert!(report["maximum_completed_load"].is_null());
        if mode == "hang" {
            let pid = sysinfo::Pid::from_u32(report["target_pid"].as_u64().unwrap() as u32);
            assert!(
                sysinfo::System::new_all().process(pid).is_none(),
                "timed-out child remains alive"
            );
        }
    }
}

#[test]
fn missing_and_invalid_provider_metrics_are_explicit() {
    for mode in ["missing_metric", "invalid_metric"] {
        let temp = tempfile::tempdir().unwrap();
        let fixture = fault_fixture(temp.path());
        let (status, report) = execute(
            temp.path(),
            config(
                json!({"mode":"launch","executable":fixture,"arguments":[mode],"working_directory":null,"controlled":true,"timeout_ms":1000}),
                1,
            ),
        );
        if mode == "missing_metric" {
            assert!(status.success(), "{report}");
            assert_eq!(
                report["stages"][0]["missing_provider_metrics"][0],
                "frame_ms"
            );
            assert!(report["maximum_validated_load"].is_null());
        } else {
            assert_eq!(status.code(), Some(5));
            assert_eq!(report["termination"], "target_failed");
            assert!(report["diagnostic"].as_str().unwrap().contains("finite"));
        }
    }
}

#[test]
fn accepts_exact_float_protocol_numbers() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = fault_fixture(temp.path());
    let (status, report) = execute(
        temp.path(),
        config(
            json!({"mode":"launch","executable":fixture,"arguments":["float_numbers"],"working_directory":null,"controlled":true,"timeout_ms":1000}),
            1,
        ),
    );
    assert!(status.success(), "{report}");
    assert_eq!(report["maximum_acknowledged_load"], 1);
}
