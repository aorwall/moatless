//! The inbound message buffer windows, parsed from `ADAPTER_MESSAGE_BUFFER_SECONDS`
//! and its `_<KIND>` variants.

use std::ffi::OsString;

use moatless_types::adapter::MessageBufferConfig;

const MESSAGE_BUFFER_ENV: &str = "ADAPTER_MESSAGE_BUFFER_SECONDS";

/// Parses the windows out of every env var `config` read.
pub(crate) fn parse(vars: impl Iterator<Item = (OsString, OsString)>) -> MessageBufferConfig {
    let seconds = |key: &str, raw: &str| match raw.trim().parse::<i32>() {
        Ok(seconds) if seconds >= 0 => Some(seconds),
        _ => {
            tracing::warn!(key, value = %raw, "Ignoring unparseable inbound buffer window");
            None
        }
    };
    let mut config = MessageBufferConfig::default();
    for (key, raw) in vars {
        let Some(key) = key.to_str().filter(|k| k.starts_with(MESSAGE_BUFFER_ENV)) else {
            continue;
        };
        let raw = raw.to_string_lossy();
        if key == MESSAGE_BUFFER_ENV {
            config.global_seconds = seconds(key, &raw);
        } else if let Some(kind) = key
            .strip_prefix(MESSAGE_BUFFER_ENV)
            .and_then(|k| k.strip_prefix('_'))
            && let Some(value) = seconds(key, &raw)
        {
            config.per_kind_seconds.insert(kind.to_string(), value);
        }
    }
    config
}
