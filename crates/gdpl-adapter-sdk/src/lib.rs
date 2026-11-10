use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_MESSAGE_BYTES: usize = 8192;

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Request {
    pub version: u32,
    pub id: u64,
    pub command: String,
    pub data: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ClockDomain {
    HostMonotonic,
    EngineMonotonic,
    ExternalCapture,
    OsTelemetry,
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Available,
    Conditional,
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MetricDefinition {
    pub name: String,
    pub unit: String,
    pub data_type: String,
    pub source: String,
    pub sampling_method: String,
    pub timestamp_domain: ClockDomain,
    pub availability: Availability,
    pub diagnostic: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    UnsupportedVersion,
    InvalidRequest,
    UnknownScenario,
    UnsupportedCapability,
    InvalidWorkload,
    UnknownMetric,
    InvalidState,
    InitializationFailed,
    Timeout,
    Cancelled,
    Disconnected,
    MeasurementFailed,
    CleanupFailed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProtocolError {
    pub code: ErrorCode,
    pub request_id: u64,
    pub details: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Connected,
    Negotiated,
    Initialized,
    Running,
    Closed,
}

#[derive(Debug, PartialEq, Eq)]
pub struct TransitionError {
    pub state: State,
    pub command: String,
}

impl State {
    pub fn transition(self, command: &str) -> Result<Self, TransitionError> {
        let next = match (self, command) {
            (Self::Connected, "hello") => Self::Negotiated,
            (Self::Negotiated, "initialize") => Self::Initialized,
            (Self::Initialized | Self::Running, "set_load") => Self::Running,
            (Self::Initialized | Self::Running, "metrics") => self,
            (Self::Initialized | Self::Running, "reset" | "cancel") => Self::Initialized,
            (Self::Negotiated | Self::Initialized | Self::Running, "clock_sync") => self,
            (Self::Negotiated, "list_scenarios" | "capabilities") => self,
            (
                Self::Connected | Self::Negotiated | Self::Initialized | Self::Running,
                "shutdown",
            ) => Self::Closed,
            _ => {
                return Err(TransitionError {
                    state: self,
                    command: command.into(),
                });
            }
        };
        Ok(next)
    }
}

/// Four timestamps in microseconds. Host and adapter clocks have independent origins.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClockExchange {
    pub host_send_us: u64,
    pub adapter_receive_us: u64,
    pub adapter_send_us: u64,
    pub host_receive_us: u64,
    pub round_trip_us: u64,
    pub offset_us: f64,
}

impl ClockExchange {
    pub fn new(
        host_send_us: u64,
        adapter_receive_us: u64,
        adapter_send_us: u64,
        host_receive_us: u64,
    ) -> Option<Self> {
        let host_elapsed = host_receive_us.checked_sub(host_send_us)?;
        let adapter_elapsed = adapter_send_us.checked_sub(adapter_receive_us)?;
        let round_trip_us = host_elapsed.checked_sub(adapter_elapsed)?;
        if round_trip_us > 250_000 {
            return None;
        }
        let offset_us = ((adapter_receive_us as f64 - host_send_us as f64)
            + (adapter_send_us as f64 - host_receive_us as f64))
            / 2.0;
        Some(Self {
            host_send_us,
            adapter_receive_us,
            adapter_send_us,
            host_receive_us,
            round_trip_us,
            offset_us,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClockCorrelation {
    pub exchanges: Vec<ClockExchange>,
    pub estimated_offset_us: f64,
    pub uncertainty_estimate_us: f64,
    pub offset_spread_us: f64,
    pub age_us: u64,
    pub quality: String,
    pub assumptions: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serialization_and_lifecycle() {
        let request = Request {
            version: 1,
            id: 7,
            command: "hello".into(),
            data: serde_json::json!({"versions":[1]}),
        };
        assert_eq!(
            serde_json::from_str::<Request>(&serde_json::to_string(&request).unwrap()).unwrap(),
            request
        );
        assert!(State::Connected.transition("set_load").is_err());
        let state = State::Connected
            .transition("hello")
            .unwrap()
            .transition("initialize")
            .unwrap()
            .transition("set_load")
            .unwrap();
        assert_eq!(state.transition("reset").unwrap(), State::Initialized);
        assert!(State::Closed.transition("hello").is_err());
    }
    #[test]
    fn clock_exchange_checks_order_and_latency() {
        let sample = ClockExchange::new(1000, 1250, 1300, 1200).unwrap();
        assert_eq!(sample.round_trip_us, 150);
        assert_eq!(sample.offset_us, 175.0);
        assert!(ClockExchange::new(1000, 1300, 1200, 1400).is_none());
        assert!(ClockExchange::new(1000, 1200, 1201, 999).is_none());
        assert!(ClockExchange::new(1000, 1200, 1201, 300_000).is_none());
    }
    #[test]
    fn clock_domains_are_explicit() {
        assert_ne!(ClockDomain::EngineMonotonic, ClockDomain::HostMonotonic);
        assert_eq!(
            serde_json::to_string(&ClockDomain::ExternalCapture).unwrap(),
            "\"external_capture\""
        );
    }
}
