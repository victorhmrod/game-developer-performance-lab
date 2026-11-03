use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn analyzes_csv_and_rejects_missing_columns() {
    let path = std::env::temp_dir().join(format!(
        "gdpl-frames-{}.csv",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(
        &path,
        "ProcessID,SwapChainAddress,MsBetweenDisplayChange\n7,0x1,10\n7,0x1,20\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_gdpl"))
        .args(["frames", "analyze", "--input"])
        .arg(&path)
        .args(["--pid", "7", "--swap-chain", "0x1"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["distribution_ms"]["count"], 2);
    assert_eq!(result["capture_coverage"], serde_json::Value::Null);
    fs::write(&path, "ProcessID\n7\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_gdpl"))
        .args(["frames", "analyze", "--input"])
        .arg(&path)
        .args(["--pid", "7", "--swap-chain", "0x1"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing PresentMon column"));
    fs::remove_file(path).unwrap();
}
