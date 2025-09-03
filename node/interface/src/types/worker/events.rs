use alloy_primitives::{Address, B256};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::super::{error::ConversionError, secrets::ChainIdentifier};
use crate::proto::interface;

/// An event that can trigger a worker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerEvent {
    /// The name of the function in the worker to handle this event.
    pub handler: String,
    #[serde(flatten)]
    pub kind: WorkerEventKind,
}

/// The kind of an event that can trigger a worker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
#[non_exhaustive]
pub enum WorkerEventKind {
    /// Runs whenever the worker is freshly started.
    Launch,
    /// Describes a Web3 event subscription.
    Web3Event(Web3Event),
    /// Indicates the worker can handle HTTP requests.
    HttpRequest,
    /// Describes a scheduled event with timing configuration.
    Scheduled(Schedule),
}

/// Top-level schedule config using "first-match" deserialization.
/// Put `Schedule::Rate` first so missing/unknown `mode` falls back to rate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Schedule {
    Rate(RateMode),
    // Add other modes later (e.g., Calendar) after Rate to keep Rate the default.
}

/// Mode discriminator.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Rate,
}

/// Catch-up strategy for late ticks.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum CatchUp {
    /// Drop overdue ticks and schedule only the next on-time one.
    #[default]
    Skip,
    /// Merge all missed ticks into a single immediate tick.
    Coalesce,
    /// Enqueue missed ticks for the handler to process.
    Queue,
}

/// Optional policy tuning. All fields optional.
/// Omitted => sane best-effort.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Policy {
    /// What to do if a tick is late. Default: `skip`.
    #[serde(default)]
    pub catch_up: CatchUp,
    /// Drop a tick if it fires later than this many ms. Omit to disable.
    #[serde(default)]
    pub max_lateness_ms: Option<u64>,
    /// Used for monitoring/SLOs only. Omit if not needed.
    #[serde(default)]
    pub jitter_budget_ms: Option<u64>,
}

/// High-resolution, monotonic, rate-based schedule.
///
/// Required:
/// - `period_ms`
///
/// Defaults:
/// - `mode` = "rate"
/// - `phase_ms` = 0
/// - `start_at` = immediate (None)
/// - `end_at` = never (None)
/// - `max_occurrences` = infinite (None)
/// - `policy` = best-effort (None)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RateMode {
    /// Optional discriminator. Defaults to `"rate"`. May be omitted.
    #[serde(default)]
    pub mode: Mode,

    /// Period between ticks in milliseconds. Required.
    pub period_ms: u64,

    /// Phase offset from the start boundary in milliseconds. Default 0.
    #[serde(default)]
    pub phase_ms: u64,

    /// When to start. `None` means start immediately. Default None.
    #[serde(default)]
    pub start_at: Option<DateTime<Utc>>,

    /// When to stop. `None` means never. Default None.
    #[serde(default)]
    pub end_at: Option<DateTime<Utc>>,

    /// Max number of ticks. `None` means infinite. Default None.
    #[serde(default)]
    pub max_occurrences: Option<u64>,

    /// Optional policy. Omit for best-effort defaults.
    #[serde(default)]
    pub policy: Option<Policy>,
}

impl RateMode {
    /// Helper: minimal constructor with only the required field.
    pub fn new(period_ms: u64) -> Self {
        Self {
            mode: Mode::Rate,
            period_ms,
            phase_ms: 0,
            start_at: None,
            end_at: None,
            max_occurrences: None,
            policy: None,
        }
    }
}

/// Configuration for a Web3 event listener, mirroring Alloy's Filter structure.
/// This is the Rust representation of the JSON `Web3Event` type in the work order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Web3Event {
    pub chain: ChainIdentifier,
    /// Contract addresses to filter for.
    /// - `None` or empty `Vec` typically means wildcard (any address), depending on RPC interpretation.
    ///   Our interpretation: empty Vec means wildcard.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub address: Vec<Address>,
    /// Topic filters. A `Vec<Vec<B256>>`.
    /// Outer Vec corresponds to topic0, topic1, etc. Max 4.
    /// Inner Vec contains alternative values for that topic.
    /// - `topics: []` (empty outer Vec) -> wildcard for all topic positions.
    /// - `topics: [vec![]]` -> topic0 must be empty (FilterSet::Values([])), rest wildcard.
    /// - `topics: [vec!["0x...".parse().unwrap()]]` -> topic0 specific, rest wildcard.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topics: Vec<Vec<B256>>,
    /// Explicit WebSocket gateways to use instead of the default for this chain.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gateways: Vec<String>,
}

impl TryFrom<interface::Web3EventConfig> for Web3Event {
    type Error = ConversionError;
    fn try_from(p: interface::Web3EventConfig) -> Result<Self, Self::Error> {
        Ok(Self {
            chain: p
                .chain
                .ok_or(ConversionError::MissingField("chain".to_string()))?
                .try_into()?,
            address: p
                .address
                .into_iter()
                .map(|s| {
                    s.parse().map_err(|e| ConversionError::InvalidValue {
                        field: "address".to_string(),
                        message: format!("failed to parse address '{}': {}", s, e),
                    })
                })
                .collect::<Result<_, _>>()?,
            topics: p
                .topics
                .into_iter()
                .map(|topic_filter| {
                    topic_filter
                        .values
                        .into_iter()
                        .map(|s| {
                            s.parse().map_err(|e| ConversionError::InvalidValue {
                                field: "topics".to_string(),
                                message: format!("failed to parse topic '{}': {}", s, e),
                            })
                        })
                        .collect()
                })
                .collect::<Result<_, _>>()?,
            gateways: p.gateways,
        })
    }
}

impl From<Web3Event> for interface::Web3EventConfig {
    fn from(value: Web3Event) -> Self {
        interface::Web3EventConfig {
            chain: Some(value.chain.into()),
            address: value.address.iter().map(|a| format!("{a:#x}")).collect(),
            topics: value
                .topics
                .iter()
                .map(|topic_values| interface::ProtoTopicFilter {
                    values: topic_values.iter().map(|t| format!("{t:#x}")).collect(),
                })
                .collect(),
            gateways: value.gateways,
        }
    }
}

// --- Event Delivery Types ---

/// Represents a Web3 log event, mirroring `alloy_rpc_types::Log`.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Web3Log {
    pub address: Address,
    pub topics: Vec<B256>,
    pub data: alloy_primitives::Bytes,
    pub block_hash: Option<B256>,
    pub block_number: Option<u64>,
    pub transaction_hash: Option<B256>,
    pub transaction_index: Option<u64>, // usize in alloy, u64 is fine for proto
    pub log_index: Option<u64>,         // usize in alloy, u64 is fine for proto
    pub removed: bool,
}

impl From<alloy_rpc_types::Log> for Web3Log {
    fn from(log: alloy_rpc_types::Log) -> Self {
        Self {
            address: log.inner.address,
            topics: log.inner.topics().to_vec(), // Access topics through TopicList
            data: log.inner.data.data,
            block_hash: log.block_hash,
            block_number: log.block_number,
            transaction_hash: log.transaction_hash,
            transaction_index: log.transaction_index,
            log_index: log.log_index,
            removed: log.removed,
        }
    }
}

impl From<Web3Log> for interface::Web3Log {
    fn from(log: Web3Log) -> Self {
        Self {
            address: log.address.to_vec(),
            topics: log.topics.iter().map(|t| t.to_vec()).collect(),
            data: log.data.to_vec(),
            block_hash: log.block_hash.map_or_else(Vec::new, |h| h.to_vec()),
            block_number: log.block_number.unwrap_or(0),
            transaction_hash: log.transaction_hash.map_or_else(Vec::new, |h| h.to_vec()),
            transaction_index: log.transaction_index.unwrap_or(0),
            log_index: log.log_index.unwrap_or(0),
            removed: log.removed,
        }
    }
}

impl TryFrom<interface::Web3Log> for Web3Log {
    type Error = ConversionError;
    fn try_from(p_log: interface::Web3Log) -> Result<Self, Self::Error> {
        let address = Address::try_from(p_log.address.as_slice()).map_err(|e| {
            ConversionError::InvalidValue {
                field: "address".to_string(),
                message: e.to_string(),
            }
        })?;
        let topics = p_log
            .topics
            .into_iter()
            .map(|b| {
                B256::try_from(b.as_slice()).map_err(|e| ConversionError::InvalidValue {
                    field: "topics".to_string(),
                    message: e.to_string(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let block_hash = if p_log.block_hash.is_empty() {
            None
        } else {
            Some(B256::try_from(p_log.block_hash.as_slice()).map_err(|e| {
                ConversionError::InvalidValue {
                    field: "block_hash".to_string(),
                    message: e.to_string(),
                }
            })?)
        };
        let transaction_hash = if p_log.transaction_hash.is_empty() {
            None
        } else {
            Some(
                B256::try_from(p_log.transaction_hash.as_slice()).map_err(|e| {
                    ConversionError::InvalidValue {
                        field: "transaction_hash".to_string(),
                        message: e.to_string(),
                    }
                })?,
            )
        };

        Ok(Self {
            address,
            topics,
            data: p_log.data.into(),
            block_hash,
            block_number: if p_log.block_number == 0 && p_log.block_hash.is_empty() {
                None
            } else {
                Some(p_log.block_number)
            },
            transaction_hash,
            transaction_index: if p_log.transaction_index == 0 && p_log.transaction_hash.is_empty()
            {
                None
            } else {
                Some(p_log.transaction_index)
            },
            log_index: if p_log.log_index == 0 && p_log.transaction_hash.is_empty() {
                None
            } else {
                Some(p_log.log_index)
            },
            removed: p_log.removed,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum EventPayload<'a> {
    Web3Log(Web3Log),
    Launch,
    HttpRequest,
    Scheduled,
    #[serde(borrow)]
    _Phantom(std::marker::PhantomData<&'a ()>), // Future event types
}

impl TryFrom<interface::EventPayload> for EventPayload<'_> {
    type Error = ConversionError;
    fn try_from(p_payload: interface::EventPayload) -> Result<Self, Self::Error> {
        match p_payload.payload {
            Some(interface::event_payload::Payload::Web3Log(log)) => {
                Ok(EventPayload::Web3Log(Web3Log::try_from(log)?))
            }
            Some(interface::event_payload::Payload::LaunchEvent(_)) => Ok(EventPayload::Launch),
            Some(interface::event_payload::Payload::HttpRequest(_)) => {
                Ok(EventPayload::HttpRequest)
            }
            Some(interface::event_payload::Payload::ScheduledEvent(_)) => {
                Ok(EventPayload::Scheduled)
            }
            None => Err(ConversionError::MissingField("payload".to_string())),
        }
    }
}

impl From<EventPayload<'_>> for interface::EventPayload {
    fn from(payload: EventPayload) -> Self {
        match payload {
            EventPayload::Web3Log(log) => Self {
                payload: Some(interface::event_payload::Payload::Web3Log(log.into())),
            },
            EventPayload::Launch => Self {
                payload: Some(interface::event_payload::Payload::LaunchEvent(())),
            },
            EventPayload::HttpRequest => Self {
                payload: Some(interface::event_payload::Payload::HttpRequest(())),
            },
            EventPayload::Scheduled => Self {
                payload: Some(interface::event_payload::Payload::ScheduledEvent(())),
            },
            EventPayload::_Phantom(_) => panic!("Cannot convert _Phantom EventPayload"),
        }
    }
}
