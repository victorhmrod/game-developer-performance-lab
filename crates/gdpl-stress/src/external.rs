use gdpl_core::{StressScenario, StressScenarioProvider};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExternalConfig {
    Launch {
        executable: PathBuf,
        arguments: Vec<String>,
        working_directory: Option<PathBuf>,
        controlled: bool,
        timeout_ms: u64,
    },
    Attach {
        pid: u32,
    },
}

// Some JSON implementations emit integral protocol numbers as floats.
fn protocol_integer(value: &Value) -> Option<u64> {
    if let Some(number) = value.as_u64() {
        return Some(number);
    }
    let number = value.as_f64()?;
    if number.is_finite()
        && (0.0..=9_007_199_254_740_992.0).contains(&number)
        && number.fract() == 0.0
    {
        return Some(number as u64);
    }
    None
}

pub struct External {
    config: ExternalConfig,
    child: Option<Child>,
    writer: Option<Sender<Vec<u8>>>,
    write_results: Option<Receiver<Result<(), String>>>,
    responses: Option<Receiver<Result<String, String>>>,
    pid: Option<u32>,
    sequence: u64,
    controlled: bool,
    supports_reset: bool,
    supports_metrics: bool,
    metric_metadata: Option<std::collections::BTreeMap<String, String>>,
    provider_info: Option<std::collections::BTreeMap<String, String>>,
    timeout: Duration,
}

impl External {
    pub fn new(config: ExternalConfig) -> Self {
        Self {
            config,
            child: None,
            writer: None,
            write_results: None,
            responses: None,
            pid: None,
            sequence: 0,
            controlled: false,
            supports_reset: false,
            supports_metrics: false,
            metric_metadata: None,
            provider_info: None,
            timeout: Duration::from_secs(2),
        }
    }
    fn request(&mut self, command: &str, value: Value) -> Result<Value, String> {
        self.sequence += 1;
        let request = json!({"version": 1, "id": self.sequence, "command": command, "data": value});
        let mut line = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
        if line.len() > 8191 {
            return Err("protocol request exceeds 8192 bytes".into());
        }
        line.push(b'\n');
        self.writer
            .as_ref()
            .ok_or("protocol stdin unavailable")?
            .send(line)
            .map_err(|e| format!("protocol writer unavailable: {e}"))?;
        self.write_results
            .as_ref()
            .ok_or("protocol writer status unavailable")?
            .recv_timeout(self.timeout)
            .map_err(|e| format!("protocol write timeout or disconnect: {e}"))??;
        let line = self
            .responses
            .as_ref()
            .ok_or("protocol response unavailable")?
            .recv_timeout(self.timeout)
            .map_err(|e| format!("protocol response timeout or disconnect: {e}"))??;
        let response: Value =
            serde_json::from_str(&line).map_err(|e| format!("malformed protocol response: {e}"))?;
        if protocol_integer(&response["version"]) != Some(1)
            || protocol_integer(&response["id"]) != Some(self.sequence)
        {
            return Err("protocol version or request id mismatch".into());
        }
        if response["ok"] != true {
            return Err(format!(
                "workload rejected {command}: {}",
                response["error"]
            ));
        }
        Ok(response)
    }
}

impl StressScenarioProvider for External {
    type Error = String;
    fn initialize(&mut self, scenario: &StressScenario) -> Result<(), String> {
        if scenario.id != "external_process" {
            return Err("external provider requires scenario id external_process".into());
        }
        match self.config.clone() {
            ExternalConfig::Attach { pid } => {
                if pid == 0 || pid == std::process::id() {
                    return Err("invalid attached PID".into());
                }
                self.pid = Some(pid);
            }
            ExternalConfig::Launch {
                executable,
                arguments,
                working_directory,
                controlled,
                timeout_ms,
            } => {
                if !(100..=10000).contains(&timeout_ms) {
                    return Err("timeout_ms must be 100–10000".into());
                }
                self.controlled = controlled;
                self.timeout = Duration::from_millis(timeout_ms);
                let mut command = Command::new(executable);
                command.args(arguments).stderr(Stdio::inherit());
                if let Some(directory) = working_directory {
                    command.current_dir(directory);
                }
                if controlled {
                    command.stdin(Stdio::piped()).stdout(Stdio::piped());
                } else {
                    command.stdin(Stdio::null()).stdout(Stdio::null());
                }
                let mut child = command.spawn().map_err(|e| format!("launch failed: {e}"))?;
                self.pid = Some(child.id());
                if controlled {
                    let mut input = child.stdin.take().ok_or("protocol stdin unavailable")?;
                    let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>();
                    let (result_tx, result_rx) = mpsc::sync_channel(1);
                    thread::spawn(move || {
                        while let Ok(line) = write_rx.recv() {
                            let result = input
                                .write_all(&line)
                                .and_then(|()| input.flush())
                                .map_err(|e| format!("protocol write failed: {e}"));
                            let failed = result.is_err();
                            if result_tx.send(result).is_err() || failed {
                                break;
                            }
                        }
                    });
                    self.writer = Some(write_tx);
                    self.write_results = Some(result_rx);
                    let stdout = child.stdout.take().ok_or("protocol stdout unavailable")?;
                    let (tx, rx) = mpsc::sync_channel(1);
                    thread::spawn(move || {
                        let mut reader = BufReader::new(stdout);
                        loop {
                            let mut line = String::new();
                            let result = (&mut reader).take(8193).read_line(&mut line);
                            match result {
                                Ok(0) => {
                                    let _ = tx.send(Err("protocol EOF".into()));
                                    break;
                                }
                                Ok(_) if line.len() > 8192 || !line.ends_with('\n') => {
                                    let _ = tx.send(Err(
                                        "protocol message too large or incomplete".into(),
                                    ));
                                    break;
                                }
                                Ok(_) => {
                                    if tx.send(Ok(line)).is_err() {
                                        break;
                                    }
                                }
                                Err(e) => {
                                    let _ = tx.send(Err(format!("protocol read failed: {e}")));
                                    break;
                                }
                            }
                        }
                    });
                    self.responses = Some(rx);
                }
                self.child = Some(child);
            }
        }
        if !self.controlled {
            if scenario.initial_load != scenario.maximum_load {
                return Err("observation-only target requires a fixed load; no workload control is available".into());
            }
            return Ok(());
        }
        let hello = self.request("hello", json!({"versions": [1]}))?;
        if !hello["capabilities"]
            .as_array()
            .is_some_and(|caps| caps.iter().any(|v| v == "set_load"))
        {
            return Err("protocol lacks set_load capability".into());
        }
        let capabilities = hello["capabilities"].as_array().unwrap();
        self.supports_reset = capabilities.iter().any(|value| value == "reset");
        self.supports_metrics = capabilities.iter().any(|value| value == "metrics");
        if let Some(info) = hello.get("provider_info") {
            let entries = info.as_object().ok_or("provider_info must be an object")?;
            if entries.len() > 16 {
                return Err("too many provider information fields".into());
            }
            let mut values = std::collections::BTreeMap::new();
            for (key, value) in entries {
                let text = value
                    .as_str()
                    .ok_or("provider information must be a string")?;
                if key.is_empty() || key.len() > 128 || text.len() > 512 {
                    return Err("invalid provider information".into());
                }
                values.insert(key.clone(), text.to_string());
            }
            if let Some(pid) = values.get("process_id") {
                let pid = pid
                    .parse::<u32>()
                    .map_err(|_| "invalid provider process_id")?;
                if Some(pid) != self.pid {
                    return Err("provider process_id differs from launched process; launch the engine executable directly, not a wrapper".into());
                }
            }
            self.provider_info = Some(values);
        }
        if let Some(metadata) = hello.get("metric_metadata") {
            let entries = metadata
                .as_object()
                .ok_or("metric_metadata must be an object")?;
            if entries.len() > 64 {
                return Err("too many metric descriptions".into());
            }
            let mut descriptions = std::collections::BTreeMap::new();
            for (key, value) in entries {
                let description = value
                    .as_str()
                    .ok_or("metric description must be a string")?;
                if key.is_empty() || key.len() > 128 || description.len() > 512 {
                    return Err("invalid metric description".into());
                }
                descriptions.insert(key.clone(), description.to_string());
            }
            self.metric_metadata = Some(descriptions);
        }
        self.request("initialize", json!({"scenario": scenario.id, "parameters": scenario.parameters, "maximum_load": scenario.maximum_load}))?;
        Ok(())
    }
    fn reset(&mut self) -> Result<(), String> {
        if self.supports_reset {
            self.request("reset", json!({}))?;
        }
        Ok(())
    }
    fn provider_info(&self) -> Option<std::collections::BTreeMap<String, String>> {
        self.provider_info.clone()
    }
    fn metric_metadata(&self) -> Option<std::collections::BTreeMap<String, String>> {
        self.metric_metadata.clone()
    }
    fn metrics(&mut self) -> Result<Option<std::collections::BTreeMap<String, f64>>, String> {
        if !self.supports_metrics {
            return Ok(None);
        }
        let response = self.request("metrics", json!({}))?;
        let values = response["metrics"]
            .as_object()
            .ok_or("protocol metrics must be an object")?;
        if values.len() > 64 {
            return Err("too many provider metrics".into());
        }
        let mut result = std::collections::BTreeMap::new();
        for (key, value) in values {
            let number = value
                .as_f64()
                .filter(|n| n.is_finite())
                .ok_or("provider metric must be finite")?;
            result.insert(key.clone(), number);
        }
        Ok(Some(result))
    }
    fn set_load(&mut self, load: u64) -> Result<(), String> {
        if !self.controlled {
            return Ok(());
        }
        let response = self.request("set_load", json!({"load": load}))?;
        if protocol_integer(&response["load"]) != Some(load)
            || response["ready"] != true
            || response
                .get("observed_load")
                .is_some_and(|value| protocol_integer(value) != Some(load))
        {
            return Err("workload was not acknowledged as ready".into());
        }
        Ok(())
    }
    fn target_pid(&self) -> Option<u32> {
        self.pid
    }
    fn check_health(&mut self) -> Result<(), String> {
        if let Some(child) = &mut self.child
            && let Some(status) = child.try_wait().map_err(|e| e.to_string())?
        {
            return Err(format!("target exited: {status}"));
        }
        if self.controlled {
            match self
                .responses
                .as_ref()
                .ok_or("protocol response unavailable")?
                .try_recv()
            {
                Ok(Ok(_)) => return Err("unexpected unsolicited protocol message".into()),
                Ok(Err(error)) => return Err(error),
                Err(mpsc::TryRecvError::Disconnected) => return Err("protocol disconnected".into()),
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        Ok(())
    }
    fn load_is_acknowledged(&self) -> bool {
        self.controlled
    }
    fn cleanup(&mut self) -> Result<(), String> {
        if self.child.is_none() {
            return Ok(());
        }
        let mut error = None;
        if self.controlled
            && let Err(e) = self.request("shutdown", json!({}))
        {
            error = Some(e);
        }
        let child = self.child.as_mut().unwrap();
        if self.controlled {
            let start = Instant::now();
            while start.elapsed() < self.timeout {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        if !status.success() && error.is_none() {
                            error = Some(format!("target exited: {status}"));
                        }
                        break;
                    }
                    Ok(None) => thread::sleep(Duration::from_millis(25)),
                    Err(e) => {
                        error = Some(format!("target wait failed: {e}"));
                        break;
                    }
                }
            }
        }
        let reaped = match child.try_wait() {
            Ok(Some(_)) => true,
            Ok(None) => {
                if let Err(e) = child.kill().and_then(|()| child.wait().map(|_| ())) {
                    let previous = error.take().map(|v| format!("{v}; ")).unwrap_or_default();
                    error = Some(format!("{previous}owned target termination failed: {e}"));
                    false
                } else {
                    if self.controlled && error.is_none() {
                        error = Some("graceful shutdown timed out; target terminated".into());
                    }
                    true
                }
            }
            Err(e) => {
                let previous = error.take().map(|v| format!("{v}; ")).unwrap_or_default();
                error = Some(format!("{previous}target wait failed: {e}"));
                false
            }
        };
        self.writer = None;
        self.write_results = None;
        self.responses = None;
        if reaped {
            self.child = None;
        }
        if let Some(error) = error {
            return Err(error);
        }
        Ok(())
    }
}

impl Drop for External {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod protocol_tests {
    use super::*;
    #[test]
    fn accepts_exact_json_integers_but_not_fractional_or_inexact_values() {
        assert_eq!(protocol_integer(&json!(1.0)), Some(1));
        assert_eq!(protocol_integer(&json!(1.5)), None);
        assert_eq!(protocol_integer(&json!(-1.0)), None);
        assert_eq!(protocol_integer(&json!(9_007_199_254_740_994.0)), None);
        assert_eq!(protocol_integer(&json!(u64::MAX)), Some(u64::MAX));
    }
}
