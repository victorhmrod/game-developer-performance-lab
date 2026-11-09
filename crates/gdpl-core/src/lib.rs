use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub struct Sample {
    /// Elapsed monotonic time since the measurement window started.
    pub elapsed_ms: u64,
    /// Percentage of one logical CPU; can exceed 100% for multithreaded processes.
    pub process_cpu_percent: f32,
    pub process_memory_bytes: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Hardware {
    pub os: Option<String>,
    pub os_version: Option<String>,
    pub cpu: Option<String>,
    pub logical_cpus: usize,
    pub total_memory_bytes: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BenchmarkResult {
    pub schema_version: u32,
    pub started_unix_ms: u128,
    pub executable: String,
    pub arguments: Vec<String>,
    pub warmup_ms: u64,
    pub requested_duration_ms: u64,
    pub sample_interval_ms: u64,
    pub termination: String,
    pub valid: bool,
    pub validity_notes: Vec<String>,
    pub hardware: Hardware,
    pub samples: Vec<Sample>,
    pub unavailable_metrics: Vec<String>,
}

/// Adapter contracts are versioned independently from result schemas.
#[derive(Debug, Serialize, Deserialize)]
pub struct AdapterDescription {
    pub protocol_version: u32,
    pub engine: String,
    pub engine_version: String,
    pub adapter_version: String,
    pub capabilities: Vec<String>,
}

/// A bounded stress plan; workload semantics are defined by the scenario provider.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StressScenario {
    pub id: String,
    pub parameters: std::collections::BTreeMap<String, String>,
    pub initial_load: u64,
    pub load_increment: u64,
    pub maximum_load: u64,
    pub measurement_seconds: u64,
    pub maximum_duration_seconds: u64,
    pub maximum_memory_bytes: u64,
    pub minimum_fps: Option<f64>,
}

impl StressScenario {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.id.trim().is_empty() {
            return Err("scenario id is required");
        }
        if self.load_increment == 0 || self.initial_load > self.maximum_load {
            return Err("invalid load range");
        }
        if self.measurement_seconds == 0
            || self.maximum_duration_seconds < self.measurement_seconds
            || self.maximum_memory_bytes == 0
        {
            return Err("invalid duration or memory limit");
        }
        if self.minimum_fps.is_some_and(|v| !v.is_finite() || v <= 0.0) {
            return Err("minimum FPS must be finite and positive");
        }
        Ok(())
    }
}

/// Providers must stop the workload and release resources when cleanup is invoked.
pub trait StressScenarioProvider {
    type Error;
    fn initialize(&mut self, scenario: &StressScenario) -> Result<(), Self::Error>;
    fn set_load(&mut self, load: u64) -> Result<(), Self::Error>;
    fn cleanup(&mut self) -> Result<(), Self::Error>;
    /// None selects the runner process for in-process scenarios.
    fn target_pid(&self) -> Option<u32> {
        None
    }
    fn check_health(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
    fn reset(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
    /// Provider-reported values have no implicit unit or trust guarantee.
    fn metrics(&mut self) -> Result<Option<std::collections::BTreeMap<String, f64>>, Self::Error> {
        Ok(None)
    }
    /// Whether successful set_load confirms a workload change.
    fn load_is_acknowledged(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod stress_tests {
    use super::*;
    #[test]
    fn bounded_scenario_validation() {
        let mut s = StressScenario {
            id: "example".into(),
            parameters: Default::default(),
            initial_load: 1,
            load_increment: 1,
            maximum_load: 10,
            measurement_seconds: 1,
            maximum_duration_seconds: 10,
            maximum_memory_bytes: 1024,
            minimum_fps: None,
        };
        assert!(s.validate().is_ok());
        s.load_increment = 0;
        assert!(s.validate().is_err());
        s.load_increment = 1;
        s.minimum_fps = Some(f64::NAN);
        assert!(s.validate().is_err());
        s.minimum_fps = None;
        s.maximum_memory_bytes = 0;
        assert!(s.validate().is_err());
    }
}
