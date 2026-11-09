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
    fn clock_domains_are_explicit() {
        assert_ne!(ClockDomain::EngineMonotonic, ClockDomain::HostMonotonic);
        assert_eq!(
            serde_json::to_string(&ClockDomain::ExternalCapture).unwrap(),
            "\"external_capture\""
        );
    }
}
