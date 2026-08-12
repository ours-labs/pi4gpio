//! Newline-delimited JSON protocol shared by the daemon and its clients.
//!
//! Requests identify a bus and an operation. Validation runs before hardware is
//! acquired so malformed or oversized operations fail without side effects.

use crate::lock::BusId;
use serde::{Deserialize, Serialize};

pub const NATIVE_PROTOCOL_V1: u16 = 1;
pub const SUPPORTED_PROTOCOL_VERSIONS: &[u16] = &[NATIVE_PROTOCOL_V1];
pub const CAPABILITIES: &[&str] = &[
    "protocol.capabilities",
    "gpio.read",
    "gpio.write",
    "gpio.watch_edges",
    "gpio.watch_edges_polled",
    "i2c.read",
    "i2c.write",
    "i2c.write_read",
    "spi.transfer",
    "uart.read",
    "uart.write",
    "resource.release",
];

pub const DEFAULT_POLL_IDLE_TIMEOUT_US: u64 = 300;
const MAX_TRANSFER_BYTES: usize = 1_048_576;
const MAX_EDGE_EVENTS: usize = 1_000_000;
const MAX_OPERATION_MS: u64 = 60_000;
const MAX_POLL_IDLE_TIMEOUT_US: u64 = 60_000_000;

fn default_poll_idle_timeout_us() -> u64 {
    DEFAULT_POLL_IDLE_TIMEOUT_US
}

fn default_protocol_versions() -> Vec<u16> {
    vec![NATIVE_PROTOCOL_V1]
}

#[derive(Debug, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub bus: Option<BusRef>,
    pub op: Operation,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BusRef {
    Gpio { pin: u32 },
    I2c { bus: u8, addr: u8 },
    Spi { bus: u8, chip_select: u8 },
    Uart { port: u8, baud_rate: u32 },
}

impl From<&BusRef> for BusId {
    fn from(bus: &BusRef) -> Self {
        match *bus {
            BusRef::Gpio { pin } => BusId::Gpio(pin),
            BusRef::I2c { bus, .. } => BusId::I2c(bus),
            BusRef::Spi { bus, chip_select } => BusId::Spi(bus, chip_select),
            BusRef::Uart { port, .. } => BusId::Uart(port),
        }
    }
}

#[derive(Debug, Deserialize, Clone, Copy, Default)]
#[serde(rename_all = "snake_case")]
pub enum PullWire {
    #[default]
    None,
    Up,
    Down,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    /// Read-only protocol negotiation. This operation is bus-free and never
    /// acquires a hardware resource lock.
    Hello {
        #[serde(default = "default_protocol_versions")]
        protocol_versions: Vec<u16>,
        #[serde(default)]
        requested_capabilities: Vec<String>,
    },
    Read {
        #[serde(default)]
        pull: PullWire,
    },
    Write {
        value: bool,
    },
    ReadBytes {
        length: usize,
    },
    WriteBytes {
        data: Vec<u8>,
    },
    WriteReadBytes {
        data: Vec<u8>,
        length: usize,
    },
    Transfer {
        data: Vec<u8>,
    },
    WatchEdges {
        pre_pulse_low_ms: Option<u64>,
        max_events: usize,
        timeout_ms: u64,
        #[serde(default)]
        pull: PullWire,
    },
    WatchEdgesPolled {
        pre_pulse_low_ms: Option<u64>,
        budget_ms: u64,
        #[serde(default = "default_poll_idle_timeout_us")]
        idle_timeout_us: u64,
        #[serde(default)]
        glitch_filter_us: u64,
        #[serde(default)]
        pull: PullWire,
    },
    Release,
}

impl Request {
    pub fn validate(&self) -> Result<(), String> {
        let compatible = matches!(
            (self.bus.as_ref(), &self.op),
            (None, Operation::Hello { .. })
                | (Some(_), Operation::Release)
                | (Some(BusRef::Gpio { .. }), Operation::Read { .. })
                | (Some(BusRef::Gpio { .. }), Operation::Write { .. })
                | (Some(BusRef::Gpio { .. }), Operation::WatchEdges { .. })
                | (
                    Some(BusRef::Gpio { .. }),
                    Operation::WatchEdgesPolled { .. }
                )
                | (Some(BusRef::I2c { .. }), Operation::ReadBytes { .. })
                | (Some(BusRef::I2c { .. }), Operation::WriteBytes { .. })
                | (Some(BusRef::I2c { .. }), Operation::WriteReadBytes { .. })
                | (Some(BusRef::Spi { .. }), Operation::Transfer { .. })
                | (Some(BusRef::Uart { .. }), Operation::ReadBytes { .. })
                | (Some(BusRef::Uart { .. }), Operation::WriteBytes { .. })
        );
        if !compatible {
            return Err("this operation is not supported for the selected bus".to_string());
        }

        let validate_length = |name: &str, length: usize| {
            if length == 0 || length > MAX_TRANSFER_BYTES {
                Err(format!(
                    "{name} must contain 1..={MAX_TRANSFER_BYTES} bytes"
                ))
            } else {
                Ok(())
            }
        };
        let validate_pre_pulse = |value: Option<u64>| {
            if value.is_some_and(|ms| ms > MAX_OPERATION_MS) {
                Err(format!(
                    "pre_pulse_low_ms must be in 0..={MAX_OPERATION_MS}"
                ))
            } else {
                Ok(())
            }
        };

        match &self.op {
            Operation::Hello {
                protocol_versions,
                requested_capabilities,
            } => {
                if protocol_versions.is_empty() || protocol_versions.len() > 16 {
                    return Err("protocol_versions must contain 1..=16 entries".to_string());
                }
                if requested_capabilities.len() > 256 {
                    return Err("requested_capabilities exceeds 256 entries".to_string());
                }
                Ok(())
            }
            Operation::ReadBytes { length } => validate_length("length", *length),
            Operation::WriteBytes { data } | Operation::Transfer { data } => {
                validate_length("data length", data.len())
            }
            Operation::WriteReadBytes { data, length } => {
                validate_length("data length", data.len())?;
                validate_length("length", *length)
            }
            Operation::WatchEdges {
                pre_pulse_low_ms,
                max_events,
                timeout_ms,
                ..
            } => {
                validate_pre_pulse(*pre_pulse_low_ms)?;
                if *max_events == 0 || *max_events > MAX_EDGE_EVENTS {
                    return Err(format!("max_events must be in 1..={MAX_EDGE_EVENTS}"));
                }
                if *timeout_ms == 0 || *timeout_ms > MAX_OPERATION_MS {
                    return Err(format!("timeout_ms must be in 1..={MAX_OPERATION_MS}"));
                }
                Ok(())
            }
            Operation::WatchEdgesPolled {
                pre_pulse_low_ms,
                budget_ms,
                idle_timeout_us,
                glitch_filter_us,
                ..
            } => {
                validate_pre_pulse(*pre_pulse_low_ms)?;
                if *budget_ms == 0 || *budget_ms > MAX_OPERATION_MS {
                    return Err(format!("budget_ms must be in 1..={MAX_OPERATION_MS}"));
                }
                if *idle_timeout_us == 0 || *idle_timeout_us > MAX_POLL_IDLE_TIMEOUT_US {
                    return Err(format!(
                        "idle_timeout_us must be in 1..={MAX_POLL_IDLE_TIMEOUT_US}"
                    ));
                }
                if *glitch_filter_us > *idle_timeout_us {
                    return Err("glitch_filter_us must not exceed idle_timeout_us".to_string());
                }
                Ok(())
            }
            Operation::Read { .. } | Operation::Write { .. } | Operation::Release => Ok(()),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Response {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edges: Option<Vec<EdgeEventWire>>,
    /// Present only for the bus-free `hello` operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<ProtocolInfo>,
}

#[derive(Debug, Serialize)]
pub struct ProtocolInfo {
    pub selected_version: Option<u16>,
    pub supported_versions: &'static [u16],
    pub available_capabilities: &'static [&'static str],
    pub enabled_capabilities: Vec<&'static str>,
    pub unavailable_capabilities: Vec<String>,
    pub clock: ClockInfo,
}

#[derive(Debug, Serialize)]
pub struct ClockInfo {
    pub source: &'static str,
    pub unit: &'static str,
    pub epoch: &'static str,
}

#[derive(Debug, Serialize)]
pub struct EdgeEventWire {
    pub timestamp_ns: u64,
    pub rising: bool,
}

impl Response {
    pub fn ok() -> Self {
        Self {
            ok: true,
            error: None,
            value: None,
            bytes: None,
            edges: None,
            protocol: None,
        }
    }

    pub fn value(value: bool) -> Self {
        Self {
            ok: true,
            error: None,
            value: Some(value),
            bytes: None,
            edges: None,
            protocol: None,
        }
    }

    pub fn bytes(data: Vec<u8>) -> Self {
        Self {
            ok: true,
            error: None,
            value: None,
            bytes: Some(data),
            edges: None,
            protocol: None,
        }
    }

    pub fn edges(events: Vec<EdgeEventWire>) -> Self {
        Self {
            ok: true,
            error: None,
            value: None,
            bytes: None,
            edges: Some(events),
            protocol: None,
        }
    }

    pub fn locked_by(holder: &str) -> Self {
        Self {
            ok: false,
            error: Some(format!("locked_by:{holder}")),
            value: None,
            bytes: None,
            edges: None,
            protocol: None,
        }
    }

    pub fn malformed(msg: &str) -> Self {
        Self {
            ok: false,
            error: Some(format!("malformed_request:{msg}")),
            value: None,
            bytes: None,
            edges: None,
            protocol: None,
        }
    }

    pub fn hw_error(msg: &str) -> Self {
        Self {
            ok: false,
            error: Some(format!("hw_error:{msg}")),
            value: None,
            bytes: None,
            edges: None,
            protocol: None,
        }
    }

    pub fn hello(protocol_versions: &[u16], requested_capabilities: &[String]) -> Self {
        let selected_version = protocol_versions
            .iter()
            .filter(|version| SUPPORTED_PROTOCOL_VERSIONS.contains(version))
            .max()
            .copied();
        let enabled_capabilities = if requested_capabilities.is_empty() {
            CAPABILITIES.to_vec()
        } else {
            CAPABILITIES
                .iter()
                .copied()
                .filter(|capability| {
                    requested_capabilities
                        .iter()
                        .any(|requested| requested == capability)
                })
                .collect()
        };
        let unavailable_capabilities = requested_capabilities
            .iter()
            .filter(|requested| !CAPABILITIES.contains(&requested.as_str()))
            .cloned()
            .collect();

        Self {
            ok: selected_version.is_some(),
            error: selected_version
                .is_none()
                .then(|| "unsupported_protocol_version".to_string()),
            value: None,
            bytes: None,
            edges: None,
            protocol: Some(ProtocolInfo {
                selected_version,
                supported_versions: SUPPORTED_PROTOCOL_VERSIONS,
                available_capabilities: CAPABILITIES,
                enabled_capabilities,
                unavailable_capabilities,
                clock: ClockInfo {
                    source: "CLOCK_MONOTONIC",
                    unit: "nanoseconds",
                    epoch: "unspecified_per_boot",
                },
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(json: &str) -> Request {
        serde_json::from_str(json).expect("request fixture must parse")
    }

    #[test]
    fn hello_is_bus_free_and_negotiates_capabilities() {
        let parsed = request(
            r#"{"op":{"hello":{"protocol_versions":[1,2],"requested_capabilities":["gpio.read","wave.start"]}}}"#,
        );
        assert!(parsed.bus.is_none());
        assert!(parsed.validate().is_ok());

        let Operation::Hello {
            protocol_versions,
            requested_capabilities,
        } = parsed.op
        else {
            panic!("wrong operation");
        };
        let encoded =
            serde_json::to_value(Response::hello(&protocol_versions, &requested_capabilities))
                .unwrap();
        assert_eq!(encoded["protocol"]["selected_version"], 1);
        assert_eq!(encoded["protocol"]["enabled_capabilities"][0], "gpio.read");
        assert_eq!(
            encoded["protocol"]["unavailable_capabilities"][0],
            "wave.start"
        );
    }

    #[test]
    fn legacy_v1_request_shape_is_unchanged() {
        let parsed = request(r#"{"bus":{"type":"gpio","pin":17},"op":{"read":{"pull":"none"}}}"#);
        assert!(matches!(
            parsed.bus.as_ref(),
            Some(BusRef::Gpio { pin: 17 })
        ));
        assert!(parsed.validate().is_ok());
    }

    #[test]
    fn polled_watch_uses_backward_compatible_idle_timeout() {
        let parsed = request(
            r#"{
                "bus":{"type":"gpio","pin":17},
                "op":{"watch_edges_polled":{
                    "pre_pulse_low_ms":null,
                    "budget_ms":15,
                    "pull":"up"
                }}
            }"#,
        );
        match parsed.op {
            Operation::WatchEdgesPolled {
                idle_timeout_us,
                glitch_filter_us,
                ..
            } => {
                assert_eq!(idle_timeout_us, DEFAULT_POLL_IDLE_TIMEOUT_US);
                assert_eq!(glitch_filter_us, 0);
            }
            _ => panic!("wrong operation"),
        }
        assert!(parsed.validate().is_ok());
    }

    #[test]
    fn client_can_override_polled_idle_timeout() {
        let parsed = request(
            r#"{
                "bus":{"type":"gpio","pin":17},
                "op":{"watch_edges_polled":{
                    "pre_pulse_low_ms":null,
                    "budget_ms":15,
                    "idle_timeout_us":2500,
                    "glitch_filter_us":10
                }}
            }"#,
        );
        assert!(parsed.validate().is_ok());
    }

    #[test]
    fn incompatible_operation_is_rejected_before_dispatch() {
        let parsed = request(
            r#"{
                "bus":{"type":"i2c","bus":1,"addr":118},
                "op":{"write":{"value":true}}
            }"#,
        );
        assert!(parsed.validate().is_err());
    }

    #[test]
    fn unbounded_requests_are_rejected() {
        let huge_read = request(
            r#"{
                "bus":{"type":"uart","port":0,"baud_rate":9600},
                "op":{"read_bytes":{"length":1048577}}
            }"#,
        );
        assert!(huge_read.validate().is_err());

        let huge_wait = request(
            r#"{
                "bus":{"type":"gpio","pin":17},
                "op":{"watch_edges":{
                    "pre_pulse_low_ms":null,
                    "max_events":10,
                    "timeout_ms":60001
                }}
            }"#,
        );
        assert!(huge_wait.validate().is_err());
    }
}
