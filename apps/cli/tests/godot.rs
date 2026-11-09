use serde_json::json;
use std::{fs, path::Path, process::Command};

#[test]
#[ignore = "requires an installed Godot 4.5+ engine executable in GDPL_GODOT"]
fn godot_headless_end_to_end() {
    let engine = std::env::var("GDPL_GODOT")
        .expect("set GDPL_GODOT to the engine executable, not the console wrapper");
    let project = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../adapters/godot/example")
        .canonicalize()
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config.json");
    let output = temp.path().join("report.json");
    for workload in ["nodes", "physics", "rendering", "animation"] {
        let data = json!({
            "scenario":{"id":"external_process","parameters":{"workload":workload},"initial_load":1,"load_increment":1,"maximum_load":2,"measurement_seconds":3,"maximum_duration_seconds":30,"maximum_memory_bytes":1073741824,"minimum_fps":null},
            "strategy":"linear","warmup_seconds":1,"repetitions":1,"maximum_cpu_percent":null,"sustained_samples":2,
            "external":{"mode":"launch","executable":engine,"arguments":["--no-header","--headless","--path",project],"working_directory":null,"controlled":true,"timeout_ms":10000}
        });
        fs::write(&config, serde_json::to_vec(&data).unwrap()).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_gdpl"))
            .args(["stress", "run", "--scenario"])
            .arg(&config)
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap();
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(&output).unwrap_or_else(|error| {
                panic!(
                    "missing report: {error}; stdout: {}; stderr: {}",
                    String::from_utf8_lossy(&result.stdout),
                    String::from_utf8_lossy(&result.stderr)
                )
            }))
            .unwrap();
        assert!(
            result.status.success(),
            "report: {report}; stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(report["termination"], "completed");
        assert_eq!(report["provider_info"]["engine"], "Godot");
        assert_eq!(
            report["provider_info"]["process_id"]
                .as_str()
                .unwrap()
                .parse::<u64>()
                .unwrap(),
            report["target_pid"].as_u64().unwrap()
        );
        assert_eq!(report["cleanup"], "completed");
        assert_eq!(report["maximum_acknowledged_load"], 2);
        assert!(report["provider_metric_metadata"]["godot_clock_us"].is_string());
        assert!(
            report["stages"][0]["provider_metric_summaries"]["godot_clock_us"]["count"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert!(report["maximum_validated_load"].is_null());
        for stage in report["stages"].as_array().unwrap() {
            assert_eq!(stage["status"], "completed");
            let load = stage["load"].as_f64().unwrap();
            let metrics = &stage["provider_metric_summaries"];
            assert_eq!(
                metrics["godot_workload_count"]["mean"].as_f64().unwrap(),
                load,
                "{workload}: {stage}"
            );
            if workload == "nodes" {
                assert_eq!(
                    metrics["godot_processing_nodes"]["mean"].as_f64().unwrap(),
                    load
                );
            }
            if workload == "physics" {
                assert!(metrics["godot_physics_active"]["mean"].as_f64().unwrap() > 0.0);
            }
            if workload == "animation" {
                assert_eq!(
                    metrics["godot_animation_playing"]["mean"].as_f64().unwrap(),
                    load
                );
                assert!(
                    metrics["godot_animation_position_sum_s"]["mean"]
                        .as_f64()
                        .unwrap()
                        > 0.0
                );
            }
        }
        fs::remove_file(&output).unwrap();
    }
}
