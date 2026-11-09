pub mod external;
mod process;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<Vec<u64>>,
    pub warmup_seconds: u64,
    pub repetitions: u32,
    pub maximum_cpu_percent: Option<f32>,
    pub sustained_samples: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<external::ExternalConfig>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    Fixed,
    Linear,
    Steps,
}

#[derive(Debug, Serialize)]
pub struct Sample {
    pub elapsed_ms: u64,
    pub process_memory_bytes: u64,
    pub process_cpu_percent: Option<f32>,
    pub provider_metrics: Option<std::collections::BTreeMap<String, f64>>,
}
#[derive(Debug, Serialize)]
pub struct Stage {
    pub load: u64,
    pub repetition: u32,
    pub acknowledged_load: Option<u64>,
    pub warmup_seconds: u64,
    pub measurement_seconds: u64,
    pub status: String,
    pub diagnostic: Option<String>,
    pub sample_count: usize,
    pub mean_cpu_percent: Option<f32>,
    pub peak_memory_bytes: Option<u64>,
    /// Statistics over polled provider snapshots, not per-frame distributions.
    pub provider_metric_summaries: std::collections::BTreeMap<String, MetricSummary>,
    pub missing_provider_metrics: Vec<String>,
    /// Responses received after the measurement boundary are not included.
    pub late_provider_responses: u64,
    pub samples: Vec<Sample>,
}
#[derive(Debug, Serialize)]
pub struct MetricSummary {
    pub count: usize,
    pub mean: Option<f64>,
    pub median: Option<f64>,
    pub p95: Option<f64>,
}

impl Stage {
    fn summarize(&mut self) {
        self.sample_count = self.samples.len();
        let cpus: Vec<f32> = self
            .samples
            .iter()
            .filter_map(|sample| sample.process_cpu_percent)
            .collect();
        self.mean_cpu_percent = if cpus.is_empty() {
            None
        } else {
            Some(cpus.iter().sum::<f32>() / cpus.len() as f32)
        };
        self.peak_memory_bytes = self
            .samples
            .iter()
            .map(|sample| sample.process_memory_bytes)
            .max();
        let mut values = std::collections::BTreeMap::<String, Vec<f64>>::new();
        for sample in &self.samples {
            if let Some(metrics) = &sample.provider_metrics {
                for (name, value) in metrics {
                    values.entry(name.clone()).or_default().push(*value);
                }
            }
        }
        for (name, mut observations) in values {
            observations.sort_by(f64::total_cmp);
            let mean = observations.iter().sum::<f64>() / observations.len() as f64;
            let percentile = |p: f64| {
                let index = (observations.len() - 1) as f64 * p;
                let low = observations[index.floor() as usize];
                let high = observations[index.ceil() as usize];
                let result = low + (high - low) * index.fract();
                result.is_finite().then_some(result)
            };
            self.provider_metric_summaries.insert(
                name,
                MetricSummary {
                    count: observations.len(),
                    mean: mean.is_finite().then_some(mean),
                    median: percentile(0.5),
                    p95: percentile(0.95),
                },
            );
        }
    }
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub started_unix_ms: u128,
    pub config: Config,
    pub scenario_kind: String,
    pub provider_info: Option<std::collections::BTreeMap<String, String>>,
    pub provider_metric_metadata: Option<std::collections::BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_metric_definitions:
        Option<std::collections::BTreeMap<String, gdpl_adapter_sdk::MetricDefinition>>,
    pub stages: Vec<Stage>,
    pub termination: String,
    pub diagnostic: Option<String>,
    pub target_pid: Option<u32>,
    pub target_start_time_unix_seconds: Option<u64>,
    pub target_creation_time_100ns_windows: Option<u64>,
    pub maximum_attempted_load: Option<u64>,
    pub maximum_acknowledged_load: Option<u64>,
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

// Counter is recreated for every repetition and includes that repetition's warmup.
fn sustained_limit(count: &mut u32, exceeded: bool, required: u32) -> bool {
    *count = if exceeded { *count + 1 } else { 0 };
    *count >= required
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
    match (&config.strategy, &config.steps) {
        (Strategy::Steps, Some(steps))
            if !steps.is_empty()
                && steps.first() == Some(&config.scenario.initial_load)
                && steps.last() == Some(&config.scenario.maximum_load)
                && steps.windows(2).all(|pair| pair[0] < pair[1]) => {}
        (Strategy::Steps, _) => {
            return Err("steps must increase strictly from initial_load to maximum_load".into());
        }
        (_, Some(_)) => return Err("steps are only allowed with strategy steps".into()),
        (_, None) => {}
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
    let mut maximum_acknowledged_load = None;
    let mut target_pid = None;
    let mut target_start_time = None;
    let mut target_creation_time = None;
    let mut provider_metric_metadata = None;
    let mut provider_metric_definitions = None;
    let mut provider_info = None;
    let mut maximum_completed_load = None;
    let mut peak_memory: Option<u64> = None;
    let mut system = System::new();

    let mut prior_cpu_sample = false;
    let outcome = (|| -> Result<(), String> {
        if cancelled.load(Ordering::SeqCst) {
            termination = "cancelled".into();
            return Ok(());
        }
        provider
            .initialize(&config.scenario)
            .map_err(|e| format!("initialization failed: {e}"))?;
        provider_info = provider.provider_info();
        provider_metric_metadata = provider.metric_metadata();
        provider_metric_definitions = provider.metric_definitions();
        target_pid = Some(provider.target_pid().unwrap_or(std::process::id()));
        let target_id = target_pid.unwrap();
        let identity = process::Identity::open(target_id)
            .map_err(|e| format!("target identity unavailable: {e}"))?;
        identity
            .verify(target_id)
            .map_err(|e| format!("target identity invalid: {e}"))?;
        target_creation_time = identity.creation_time();
        let pid = Pid::from_u32(target_id);
        system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
        target_start_time = Some(
            system
                .process(pid)
                .ok_or("target process unavailable")?
                .start_time(),
        );
        if target_start_time == Some(0) {
            return Err("target creation time unavailable".into());
        }
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
                let mut cpu_violations = 0;
                let mut cpu_valid_samples = 0;
                let mut measurement_cpu_primed = false;
                let mut stage = Stage {
                    load,
                    repetition,
                    acknowledged_load: None,
                    warmup_seconds: config.warmup_seconds,
                    measurement_seconds: config.scenario.measurement_seconds,
                    status: "incomplete".into(),
                    diagnostic: None,
                    sample_count: 0,
                    mean_cpu_percent: None,
                    peak_memory_bytes: None,
                    provider_metric_summaries: Default::default(),
                    missing_provider_metrics: Vec::new(),
                    late_provider_responses: 0,
                    samples: Vec::new(),
                };
                if let Err(error) = provider.reset() {
                    let error = format!("setting load failed: reset: {error}");
                    stage.status = "failed".into();
                    stage.diagnostic = Some(error.clone());
                    stages.push(stage);
                    return Err(error);
                }
                if let Err(error) = provider.set_load(load) {
                    let error = format!("setting load failed: {error}");
                    stage.status = "failed".into();
                    stage.diagnostic = Some(error.clone());
                    stages.push(stage);
                    return Err(error);
                }
                stage.acknowledged_load = provider.load_is_acknowledged().then_some(load);
                if stage.acknowledged_load.is_some() {
                    maximum_acknowledged_load = Some(
                        maximum_acknowledged_load.map_or(load, |previous: u64| previous.max(load)),
                    );
                }
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
                    if let Err(error) = identity.verify(target_id) {
                        let error = format!("target failure: {error}");
                        stage.status = "failed".into();
                        stage.diagnostic = Some(error.clone());
                        stage.summarize();
                        stages.push(stage);
                        return Err(error);
                    }
                    if let Err(error) = provider.check_health() {
                        let error = format!("target failure: {error}");
                        stage.status = "failed".into();
                        stage.diagnostic = Some(error.clone());
                        stage.summarize();
                        stages.push(stage);
                        return Err(error);
                    }
                    system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
                    let Some(process) = system.process(pid) else {
                        let error = "process resource telemetry unavailable";
                        stage.status = "failed".into();
                        stage.diagnostic = Some(error.into());
                        stage.summarize();
                        stages.push(stage);
                        return Err(error.into());
                    };
                    if Some(process.start_time()) != target_start_time {
                        stage.status = "failed".into();
                        stage.diagnostic = Some("target PID identity changed".into());
                        stage.summarize();
                        stages.push(stage);
                        return Err("target PID identity changed".into());
                    }
                    let memory = process.memory();
                    if memory == 0 {
                        stage.status = "failed".into();
                        stage.diagnostic = Some("process memory telemetry unavailable".into());
                        stage.summarize();
                        stages.push(stage);
                        return Err("process memory telemetry unavailable".into());
                    }
                    peak_memory = Some(peak_memory.map_or(memory, |previous| previous.max(memory)));
                    let cpu = if prior_cpu_sample {
                        let usage = process.cpu_usage();
                        if !usage.is_finite() || usage < 0.0 {
                            stage.status = "failed".into();
                            stage.diagnostic = Some("invalid process CPU telemetry".into());
                            stage.summarize();
                            stages.push(stage);
                            return Err("invalid process CPU telemetry".into());
                        }
                        cpu_valid_samples += 1;
                        Some(usage)
                    } else {
                        None
                    };
                    prior_cpu_sample = true;
                    let observed_at = start.elapsed();
                    let observed_in_stage = stage_start.elapsed();
                    if memory > config.scenario.maximum_memory_bytes {
                        termination = "memory_limit".into();
                    }
                    if termination == "completed"
                        && let (Some(limit), Some(cpu)) = (config.maximum_cpu_percent, cpu)
                        && sustained_limit(
                            &mut cpu_violations,
                            cpu > limit,
                            config.sustained_samples,
                        )
                    {
                        termination = "cpu_limit".into();
                    }
                    if observed_in_stage >= Duration::from_secs(config.warmup_seconds)
                        && observed_in_stage < total
                    {
                        let mut provider_metrics = if termination != "completed" {
                            None
                        } else {
                            match provider.metrics() {
                                Ok(metrics) => metrics,
                                Err(error) => {
                                    let error = format!("target failure: metrics: {error}");
                                    stage.status = "failed".into();
                                    stage.diagnostic = Some(error.clone());
                                    stage.summarize();
                                    stages.push(stage);
                                    return Err(error);
                                }
                            }
                        };
                        if provider_metrics.is_some()
                            && (stage_start.elapsed() >= total || cancelled.load(Ordering::SeqCst))
                        {
                            stage.late_provider_responses += 1;
                            provider_metrics = None;
                        }
                        // The preceding CPU interval can overlap warmup or a previous stage.
                        let measurement_cpu = if measurement_cpu_primed { cpu } else { None };
                        measurement_cpu_primed = true;
                        stage.samples.push(Sample {
                            elapsed_ms: observed_at.as_millis() as u64,
                            process_memory_bytes: memory,
                            process_cpu_percent: measurement_cpu,
                            provider_metrics,
                        });
                    }
                    if termination != "completed" {
                        break;
                    }
                    thread::sleep(Duration::from_millis(250));
                }
                if termination == "completed" && cancelled.load(Ordering::SeqCst) {
                    termination = "cancelled".into();
                }
                if termination == "completed"
                    && start.elapsed()
                        >= Duration::from_secs(config.scenario.maximum_duration_seconds)
                {
                    termination = "duration_limit".into();
                }
                if termination != "completed" {
                    stage.diagnostic = Some(termination.clone());
                    stage.summarize();
                    stages.push(stage);
                    break 'loads;
                }
                if stage.samples.is_empty()
                    || (config.maximum_cpu_percent.is_some()
                        && cpu_valid_samples < config.sustained_samples)
                {
                    stage.status = "failed".into();
                    stage.diagnostic = Some("insufficient process telemetry".into());
                    stage.summarize();
                    stages.push(stage);
                    return Err("insufficient process telemetry".into());
                }
                stage.status = "completed".into();
                stage.summarize();
                stages.push(stage);
            }
            maximum_completed_load = Some(load);
            if matches!(config.strategy, Strategy::Fixed) || load == config.scenario.maximum_load {
                break;
            }
            load = match config.strategy {
                Strategy::Linear => load
                    .saturating_add(config.scenario.load_increment)
                    .min(config.scenario.maximum_load),
                Strategy::Steps => *config
                    .steps
                    .as_ref()
                    .unwrap()
                    .iter()
                    .find(|value| **value > load)
                    .unwrap(),
                Strategy::Fixed => unreachable!(),
            };
        }
        Ok(())
    })();
    if let Err(error) = outcome {
        termination = if error.starts_with("initialization failed:") {
            "initialization_failed"
        } else if error.starts_with("setting load failed:") {
            "workload_failed"
        } else if error.starts_with("target failure:") {
            "target_failed"
        } else {
            "telemetry_failed"
        }
        .into();
        diagnostic = Some(error);
    }
    if target_pid.is_none() {
        target_pid = provider.target_pid();
    }
    let cleanup = match provider.cleanup() {
        Ok(()) => "completed".into(),
        Err(error) => {
            if termination == "completed" {
                termination = "cleanup_failed".into();
            }
            error
        }
    };
    if let Some(metadata) = &provider_metric_metadata {
        for stage in &mut stages {
            stage.missing_provider_metrics = metadata
                .keys()
                .filter(|key| !stage.provider_metric_summaries.contains_key(*key))
                .cloned()
                .collect();
        }
    }
    let scenario_kind = config.scenario.id.clone();
    Ok(Report {
        schema_version: 1,
        started_unix_ms,
        config,
        scenario_kind,
        provider_info,
        provider_metric_metadata,
        provider_metric_definitions,
        stages,
        termination,
        diagnostic,
        target_pid,
        target_start_time_unix_seconds: target_start_time,
        target_creation_time_100ns_windows: target_creation_time,
        maximum_attempted_load,
        maximum_acknowledged_load,
        maximum_completed_load,
        maximum_validated_load: None,
        peak_process_memory_bytes: peak_memory,
        cleanup,
        limitations: vec![
            "Workload semantics are provider-defined; synthetic_memory does not represent gameplay",
            "Resource monitoring samples the selected process only; descendants and GPU are excluded",
            "Memory limit is sampled and can be exceeded between samples",
            "Provider metrics and their polled summaries are unverified claims; polling does not capture every frame",
            "Host, engine, OS and graphics capture clocks are not synchronized; cross-source correlation is unavailable",
            "FPS/frame-time threshold evaluation and independent display timing are unavailable",
            "Completed loads are not validated hardware requirements",
            "CPU is percent of one logical core; first observation unavailable",
            "Outside Windows, PID reuse detection uses process start time with one-second precision",
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
        fail_cleanup: bool,
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
            if self.fail_cleanup {
                return Err("cleanup failure".into());
            }
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
            steps: None,
            warmup_seconds: 0,
            repetitions: 1,
            maximum_cpu_percent: None,
            sustained_samples: 2,
            external: None,
        }
    }
    #[test]
    fn cancellation_during_metric_request_does_not_complete_stage() {
        struct CancelProvider<'a>(&'a AtomicBool);
        impl StressScenarioProvider for CancelProvider<'_> {
            type Error = String;
            fn initialize(&mut self, _: &StressScenario) -> Result<(), String> {
                Ok(())
            }
            fn set_load(&mut self, _: u64) -> Result<(), String> {
                Ok(())
            }
            fn metrics(
                &mut self,
            ) -> Result<Option<std::collections::BTreeMap<String, f64>>, String> {
                self.0.store(true, Ordering::SeqCst);
                Ok(Some(std::collections::BTreeMap::from([(
                    "frame_ms".into(),
                    1.0,
                )])))
            }
            fn cleanup(&mut self) -> Result<(), String> {
                Ok(())
            }
        }
        let flag = AtomicBool::new(false);
        let mut plan = config();
        plan.strategy = Strategy::Fixed;
        plan.scenario.maximum_load = plan.scenario.initial_load;
        let report = run(plan, &mut CancelProvider(&flag), &flag).unwrap();
        assert_eq!(report.termination, "cancelled");
        assert_eq!(report.stages[0].late_provider_responses, 1);
        assert!(report.stages[0].provider_metric_summaries.is_empty());
        assert!(report.maximum_completed_load.is_none());
    }
    #[test]
    fn late_provider_metrics_are_excluded_from_measurement() {
        struct SlowProvider;
        impl StressScenarioProvider for SlowProvider {
            type Error = String;
            fn initialize(&mut self, _: &StressScenario) -> Result<(), String> {
                Ok(())
            }
            fn set_load(&mut self, _: u64) -> Result<(), String> {
                Ok(())
            }
            fn metrics(
                &mut self,
            ) -> Result<Option<std::collections::BTreeMap<String, f64>>, String> {
                std::thread::sleep(Duration::from_millis(1200));
                Ok(Some(std::collections::BTreeMap::from([(
                    "frame_ms".into(),
                    1.0,
                )])))
            }
            fn metric_metadata(&self) -> Option<std::collections::BTreeMap<String, String>> {
                Some(std::collections::BTreeMap::from([(
                    "frame_ms".into(),
                    "milliseconds; test provider".into(),
                )]))
            }
            fn cleanup(&mut self) -> Result<(), String> {
                Ok(())
            }
        }
        let mut plan = config();
        plan.strategy = Strategy::Fixed;
        plan.scenario.maximum_load = plan.scenario.initial_load;
        let report = run(plan, &mut SlowProvider, &AtomicBool::new(false)).unwrap();
        assert_eq!(report.stages[0].late_provider_responses, 1);
        assert!(report.stages[0].provider_metric_summaries.is_empty());
        assert_eq!(report.stages[0].missing_provider_metrics, vec!["frame_ms"]);
        assert!(report.maximum_validated_load.is_none());
    }
    #[test]
    fn provider_snapshot_summaries_use_polled_values() {
        let mut stage = Stage {
            load: 1,
            repetition: 1,
            acknowledged_load: Some(1),
            warmup_seconds: 0,
            measurement_seconds: 1,
            status: "completed".into(),
            diagnostic: None,
            sample_count: 0,
            mean_cpu_percent: None,
            peak_memory_bytes: None,
            provider_metric_summaries: Default::default(),
            missing_provider_metrics: Vec::new(),
            late_provider_responses: 0,
            samples: Vec::new(),
        };
        for value in [10.0, 20.0, 30.0] {
            stage.samples.push(Sample {
                elapsed_ms: 0,
                process_memory_bytes: 100,
                process_cpu_percent: None,
                provider_metrics: Some(std::collections::BTreeMap::from([(
                    "frame_ms".into(),
                    value,
                )])),
            });
        }
        stage.summarize();
        let summary = &stage.provider_metric_summaries["frame_ms"];
        assert_eq!(summary.count, 3);
        assert_eq!(summary.mean, Some(20.0));
        assert_eq!(summary.median, Some(20.0));
        assert_eq!(summary.p95, Some(29.0));
    }
    #[test]
    fn sustained_threshold_resets_on_recovery_and_new_repetition() {
        let mut count = 0;
        assert!(!sustained_limit(&mut count, true, 2));
        assert!(!sustained_limit(&mut count, false, 2));
        assert!(!sustained_limit(&mut count, true, 2));
        assert!(sustained_limit(&mut count, true, 2));
        let mut next_repetition = 0;
        assert!(!sustained_limit(&mut next_repetition, true, 2));
    }
    #[test]
    fn step_strategy_uses_explicit_loads() {
        let mut config = config();
        config.strategy = Strategy::Steps;
        config.scenario.maximum_load = 4;
        config.steps = Some(vec![1, 4]);
        let mut provider = Fixture::default();
        let report = run(config, &mut provider, &AtomicBool::new(false)).unwrap();
        assert_eq!(report.termination, "completed");
        assert_eq!(provider.loads, vec![1, 4]);
        assert!(provider.cleaned);
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
    fn preserves_primary_error_when_cleanup_also_fails() {
        let mut provider = Fixture {
            fail_init: true,
            fail_cleanup: true,
            ..Default::default()
        };
        let report = run(config(), &mut provider, &AtomicBool::new(false)).unwrap();
        assert_eq!(report.termination, "initialization_failed");
        assert!(report.diagnostic.unwrap().contains("initialization failed"));
        assert_eq!(report.cleanup, "cleanup failure");
    }
    #[test]
    fn failure_cleans_up_and_unavailable_fps_is_rejected() {
        let mut provider = Fixture {
            fail_init: true,
            ..Default::default()
        };
        let report = run(config(), &mut provider, &AtomicBool::new(false)).unwrap();
        assert_eq!(report.termination, "initialization_failed");
        assert!(provider.cleaned);
        let mut config = config();
        config.scenario.minimum_fps = Some(60.);
        assert!(run(config, &mut provider, &AtomicBool::new(false)).is_err());
    }
}
