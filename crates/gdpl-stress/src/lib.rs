use gdpl_core::{StressScenario, StressScenarioProvider};
use serde::{Deserialize, Serialize};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use sysinfo::{Pid, ProcessesToUpdate, System};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub scenario: StressScenario,
    pub strategy: Strategy,
    pub warmup_seconds: u64,
    pub repetitions: u32,
    pub maximum_cpu_percent: Option<f32>,
    pub sustained_samples: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    Fixed,
    Linear,
}

#[derive(Debug, Serialize)]
pub struct Sample {
    pub elapsed_ms: u64,
    pub process_memory_bytes: u64,
    pub process_cpu_percent: Option<f32>,
}
#[derive(Debug, Serialize)]
pub struct Stage {
    pub load: u64,
    pub repetition: u32,
    pub status: String,
    pub samples: Vec<Sample>,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub started_unix_ms: u128,
    pub config: Config,
    pub scenario_kind: String,
    pub stages: Vec<Stage>,
    pub termination: String,
    pub diagnostic: Option<String>,
    pub maximum_attempted_load: Option<u64>,
    pub maximum_completed_load: Option<u64>,
    pub maximum_validated_load: Option<u64>,
    pub peak_process_memory_bytes: Option<u64>,
    pub cleanup: String,
    pub limitations: Vec<&'static str>,
}

/// The built-in reference allocates KiB of memory; it is not representative of game load.
#[derive(Default)]
pub struct SyntheticMemory {
    bytes: Vec<u8>,
}
impl StressScenarioProvider for SyntheticMemory {
    type Error = String;
    fn initialize(&mut self, scenario: &StressScenario) -> Result<(), String> {
        if scenario.id != "synthetic_memory" || !scenario.parameters.is_empty() {
            return Err("only synthetic_memory with empty parameters is supported".into());
        }
        if scenario.maximum_load > 32768 {
            return Err("synthetic_memory is capped at 32768 KiB".into());
        }
        Ok(())
    }
    fn set_load(&mut self, load: u64) -> Result<(), String> {
        let bytes = usize::try_from(load.checked_mul(1024).ok_or("load overflow")?)
            .map_err(|_| "load overflow")?;
        if bytes > self.bytes.len() {
            self.bytes
                .try_reserve_exact(bytes - self.bytes.len())
                .map_err(|e| e.to_string())?;
        }
        self.bytes.resize(bytes, 1);
        Ok(())
    }
    fn cleanup(&mut self) -> Result<(), String> {
        self.bytes = Vec::new();
        Ok(())
    }
}

/// Executes a bounded plan against an explicitly provided workload provider.
pub fn run<P: StressScenarioProvider<Error = String>>(
    config: Config,
    provider: &mut P,
    cancelled: &AtomicBool,
) -> Result<Report, String> {
    config.scenario.validate().map_err(str::to_owned)?;
    if config.scenario.maximum_duration_seconds > 3600
        || config.warmup_seconds > 3600
        || config.repetitions == 0
        || config.repetitions > 100
        || config.sustained_samples == 0
        || config.sustained_samples > 100
    {
        return Err("invalid duration, repetitions or sustained_samples".into());
    }
    if config.scenario.minimum_fps.is_some() {
        return Err("FPS threshold unavailable for this runner".into());
    }
    if config
        .maximum_cpu_percent
        .is_some_and(|v| !v.is_finite() || v <= 0.0)
    {
        return Err("maximum_cpu_percent must be finite and positive".into());
    }
    let started_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis();
    let start = Instant::now();
    let mut stages = Vec::new();
    let mut termination = "completed".to_owned();
    let mut diagnostic = None;
    let mut maximum_attempted_load = None;
    let mut maximum_completed_load = None;
    let mut peak_memory: Option<u64> = None;
    let mut system = System::new();
    let pid = Pid::from_u32(std::process::id());
    let mut prior_cpu_sample = false;
    let mut cpu_violations = 0;
    let outcome = (|| -> Result<(), String> {
        provider
            .initialize(&config.scenario)
            .map_err(|e| format!("initialization failed: {e}"))?;
        let mut load = config.scenario.initial_load;
        'loads: loop {
            for repetition in 1..=config.repetitions {
                if cancelled.load(Ordering::SeqCst) {
                    termination = "cancelled".into();
                    break 'loads;
                }
                if start.elapsed() >= Duration::from_secs(config.scenario.maximum_duration_seconds)
                {
                    termination = "duration_limit".into();
                    break 'loads;
                }
                maximum_attempted_load =
                    Some(maximum_attempted_load.map_or(load, |previous: u64| previous.max(load)));
                provider
                    .set_load(load)
                    .map_err(|e| format!("setting load failed: {e}"))?;
                let mut stage = Stage {
                    load,
                    repetition,
                    status: "incomplete".into(),
                    samples: Vec::new(),
                };
                let stage_start = Instant::now();
                let total = Duration::from_secs(
                    config.warmup_seconds + config.scenario.measurement_seconds,
                );
                while stage_start.elapsed() < total {
                    if cancelled.load(Ordering::SeqCst) {
                        termination = "cancelled".into();
                        break;
                    }
                    if start.elapsed()
                        >= Duration::from_secs(config.scenario.maximum_duration_seconds)
                    {
                        termination = "duration_limit".into();
                        break;
                    }
                    system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
                    let process = system
                        .process(pid)
                        .ok_or("process resource telemetry unavailable")?;
                    let memory = process.memory();
                    if memory == 0 {
                        return Err("process memory telemetry unavailable".into());
                    }
                    peak_memory = Some(peak_memory.map_or(memory, |previous| previous.max(memory)));
                    let cpu = if prior_cpu_sample {
                        Some(process.cpu_usage())
                    } else {
                        None
                    };
                    prior_cpu_sample = true;
                    if stage_start.elapsed() >= Duration::from_secs(config.warmup_seconds) {
                        stage.samples.push(Sample {
                            elapsed_ms: start.elapsed().as_millis() as u64,
                            process_memory_bytes: memory,
                            process_cpu_percent: cpu,
                        });
                    }
                    if memory > config.scenario.maximum_memory_bytes {
                        termination = "memory_limit".into();
                        break;
                    }
                    if let (Some(limit), Some(cpu)) = (config.maximum_cpu_percent, cpu) {
                        cpu_violations = if cpu > limit { cpu_violations + 1 } else { 0 };
                        if cpu_violations >= config.sustained_samples {
                            termination = "cpu_limit".into();
                            break;
                        }
                    }
                    thread::sleep(Duration::from_millis(250));
                }
                if termination != "completed" {
                    stages.push(stage);
                    break 'loads;
                }
                if stage.samples.is_empty()
                    || (config.maximum_cpu_percent.is_some()
                        && !stage
                            .samples
                            .iter()
                            .any(|sample| sample.process_cpu_percent.is_some()))
                {
                    stages.push(stage);
                    return Err("insufficient process telemetry".into());
                }
                stage.status = "completed".into();
                stages.push(stage);
            }
            maximum_completed_load = Some(load);
            if matches!(config.strategy, Strategy::Fixed) || load == config.scenario.maximum_load {
                break;
            }
            load = load
                .saturating_add(config.scenario.load_increment)
                .min(config.scenario.maximum_load);
        }
        Ok(())
    })();
    if let Err(error) = outcome {
        termination = "provider_or_telemetry_error".into();
        diagnostic = Some(error);
    }
    let cleanup = match provider.cleanup() {
        Ok(()) => "completed".into(),
        Err(error) => {
            termination = "cleanup_failed".into();
            error
        }
    };
    let scenario_kind = config.scenario.id.clone();
    Ok(Report {
        schema_version: 1,
        started_unix_ms,
        config,
        scenario_kind,
        stages,
        termination,
        diagnostic,
        maximum_attempted_load,
        maximum_completed_load,
        maximum_validated_load: None,
        peak_process_memory_bytes: peak_memory,
        cleanup,
        limitations: vec![
            "Workload semantics are provider-defined; synthetic_memory does not represent gameplay",
            "Resource monitoring samples the GDPL process only; descendants and GPU are excluded",
            "Memory limit is sampled and can be exceeded between samples",
            "FPS and frame-time telemetry are unavailable",
            "Completed loads are not validated hardware requirements",
            "CPU is percent of one logical core; first observation unavailable",
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    #[derive(Default)]
    struct Fixture {
        loads: Vec<u64>,
        cleaned: bool,
        fail_init: bool,
    }
    impl StressScenarioProvider for Fixture {
        type Error = String;
        fn initialize(&mut self, _: &StressScenario) -> Result<(), String> {
            if self.fail_init {
                return Err("failure".into());
            }
            Ok(())
        }
        fn set_load(&mut self, load: u64) -> Result<(), String> {
            self.loads.push(load);
            Ok(())
        }
        fn cleanup(&mut self) -> Result<(), String> {
            self.cleaned = true;
            Ok(())
        }
    }
    fn config() -> Config {
        Config {
            scenario: StressScenario {
                id: "fixture".into(),
                parameters: Default::default(),
                initial_load: 1,
                load_increment: 1,
                maximum_load: 2,
                measurement_seconds: 1,
                maximum_duration_seconds: 10,
                maximum_memory_bytes: u64::MAX,
                minimum_fps: None,
            },
            strategy: Strategy::Linear,
            warmup_seconds: 0,
            repetitions: 1,
            maximum_cpu_percent: None,
            sustained_samples: 2,
        }
    }
    #[test]
    fn ramps_and_always_cleans_up() {
        let mut provider = Fixture::default();
        let report = run(config(), &mut provider, &AtomicBool::new(false)).unwrap();
        assert_eq!(provider.loads, vec![1, 2]);
        assert!(provider.cleaned);
        assert_eq!(report.termination, "completed");
        assert_eq!(report.maximum_completed_load, Some(2));
        assert!(report.maximum_validated_load.is_none());
        assert_eq!(report.stages.len(), 2);
        assert!(serde_json::to_string(&report).is_ok());
    }
    #[test]
    fn memory_limit_and_cancellation_stop_and_clean() {
        let mut config = config();
        config.scenario.maximum_memory_bytes = 1;
        let mut provider = Fixture::default();
        let report = run(config, &mut provider, &AtomicBool::new(false)).unwrap();
        assert_eq!(report.termination, "memory_limit");
        assert!(provider.cleaned);
        assert!(report.maximum_completed_load.is_none());
        let mut provider = Fixture::default();
        let report = run(self::config(), &mut provider, &AtomicBool::new(true)).unwrap();
        assert_eq!(report.termination, "cancelled");
        assert!(provider.cleaned);
        assert!(provider.loads.is_empty());
    }
    #[test]
    fn duration_limit_stops_before_next_stage_and_cleans_up() {
        let mut config = config();
        config.scenario.maximum_duration_seconds = 1;
        let mut provider = Fixture::default();
        let report = run(config, &mut provider, &AtomicBool::new(false)).unwrap();
        assert_eq!(report.termination, "duration_limit");
        assert!(provider.cleaned);
        assert!(provider.loads.len() <= 1);
    }
    #[test]
    fn failure_cleans_up_and_unavailable_fps_is_rejected() {
        let mut provider = Fixture {
            fail_init: true,
            ..Default::default()
        };
        let report = run(config(), &mut provider, &AtomicBool::new(false)).unwrap();
        assert_eq!(report.termination, "provider_or_telemetry_error");
        assert!(provider.cleaned);
        let mut config = config();
        config.scenario.minimum_fps = Some(60.);
        assert!(run(config, &mut provider, &AtomicBool::new(false)).is_err());
    }
}
