//! The frames every UI-RPC request is answered with, before any method or
//! contract error is named.

use crate::rpc::metrics as rpc_metrics;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::fmt;

/// A request id, in the JSON form it arrived as and the text this protocol
/// correlates on.
///
/// The client keys its response map by the exact value it sent; echoing `"0"`
/// back for a request sent as `0` matches no caller and silently times out.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RequestId {
    pub(crate) text: String,
    pub(crate) wire: Value,
}

impl RequestId {
    pub(crate) fn as_str(&self) -> &str {
        &self.text
    }

    pub(crate) fn wire(&self) -> Value {
        self.wire.clone()
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text)
    }
}

impl<'de> Deserialize<'de> for RequestId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct RequestIdVisitor;

        impl serde::de::Visitor<'_> for RequestIdVisitor {
            type Value = RequestId;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a request id as a string or an integer")
            }

            fn visit_str<E: serde::de::Error>(self, id: &str) -> Result<RequestId, E> {
                Ok(RequestId {
                    text: id.to_string(),
                    wire: Value::String(id.to_string()),
                })
            }

            fn visit_u64<E: serde::de::Error>(self, id: u64) -> Result<RequestId, E> {
                Ok(RequestId {
                    text: id.to_string(),
                    wire: Value::from(id),
                })
            }

            fn visit_i64<E: serde::de::Error>(self, id: i64) -> Result<RequestId, E> {
                Ok(RequestId {
                    text: id.to_string(),
                    wire: Value::from(id),
                })
            }
        }

        // A fractional id reaches no visitor here and is refused, which is what
        // `valid_request_id` would decide about it anyway.
        deserializer.deserialize_any(RequestIdVisitor)
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "_tag")]
pub(crate) enum ClientFrame {
    Request {
        id: RequestId,
        tag: String,
        #[serde(default)]
        payload: Value,
        #[serde(default)]
        headers: Vec<(String, String)>,
        #[serde(rename = "traceId")]
        #[serde(default)]
        trace_id: Option<String>,
        #[serde(rename = "spanId")]
        #[serde(default)]
        span_id: Option<String>,
        #[serde(default)]
        sampled: Option<bool>,
    },
    Ack {
        #[serde(rename = "requestId")]
        request_id: RequestId,
    },
    Interrupt {
        #[serde(rename = "requestId")]
        request_id: RequestId,
    },
    Ping,
    Eof,
}

pub(crate) fn success_exit(request_id: &RequestId, value: Value) -> Value {
    json!({
        "_tag": "Exit",
        "requestId": request_id.wire(),
        "exit": {
            "_tag": "Success",
            "value": value
        }
    })
}

pub(crate) fn failure_exit(request_id: &RequestId, defect: impl Into<String>) -> Value {
    json!({
        "_tag": "Exit",
        "requestId": request_id.wire(),
        "exit": {
            "_tag": "Failure",
            "cause": [{
                "_tag": "Die",
                "defect": defect.into()
            }]
        }
    })
}

/// Fail a request with an error the client's schema knows how to decode.
///
/// Unlike [`failure_exit`]'s string defect, this carries a tagged error the
/// client matches against the method's declared error type and handles as an
/// ordinary outcome, not a server defect.
pub(crate) fn typed_failure_exit(request_id: &RequestId, error: Value) -> Value {
    json!({
        "_tag": "Exit",
        "requestId": request_id.wire(),
        "exit": {
            "_tag": "Failure",
            "cause": [{
                "_tag": "Fail",
                "error": error
            }]
        }
    })
}

pub(crate) fn interrupted_exit(request_id: &RequestId) -> Value {
    json!({
        "_tag": "Exit",
        "requestId": request_id.wire(),
        "exit": {
            "_tag": "Failure",
            "cause": [{
                "_tag": "Interrupt"
            }]
        }
    })
}

/// Refuse a frame the socket could not act on at all.
///
/// A `Defect` answers the connection rather than a request. `reason` is a fixed
/// word, not `defect`'s message, which carries a serde error or request id and
/// would make the counter's label set unbounded.
pub(crate) fn connection_defect(reason: &'static str, defect: impl Into<String>) -> Value {
    metrics::counter!(rpc_metrics::CONNECTION_DEFECTS, "reason" => reason).increment(1);
    json!({
        "_tag": "Defect",
        "defect": defect.into()
    })
}
pub(crate) fn valid_request_id(id: &str) -> bool {
    let digits = id.strip_prefix('-').unwrap_or(id);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}
