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
