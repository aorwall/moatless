//! `server.getUsageSummary` — token and cost totals for a window.
//!
//! T3's contract is `packages/contracts/src/usage.ts`; this transcribes it by
//! hand, as the rest of this crate does. Two of its fields describe a local
//! transcript scan that has no counterpart here, and [`source_for`] documents
//! what they report instead.

use moatless_types::model::{ClaudeModel, CodexModel, ModelPrice, price_for};
use serde_json::{Value, json};

use crate::ports::{UsageBucket, UsageWindow};

/// Bumped by T3 whenever [`UsageSummary`]'s shape changes incompatibly.
/// `packages/contracts/src/usage.ts` is the authority; a client merging an
/// older version renders partial coverage rather than failing the page.
const CONTRACT_VERSION: u32 = 5;

/// What the cached input would have cost at full input rates, minus what it did
/// cost. Zero for a model with no known rate, which keeps an unpriced model out
/// of the savings total rather than guessing at it.
fn cache_savings_usd(model: &str, cached_input_tokens: i64) -> f64 {
    if cached_input_tokens <= 0 {
        return 0.0;
    }
    let Some(ModelPrice {
        input, cache_read, ..
    }) = price_for(model)
    else {
        return 0.0;
    };
    #[allow(clippy::cast_precision_loss)]
    let tokens = cached_input_tokens as f64;
    tokens * (input - cache_read) / 1_000_000.0
}

/// Why a bucket's cost is what it is.
///
/// Never `modelPriced`: an unpriced turn stays unpriced rather than being
/// valued here, since a second rate table would drift from what the backend
/// already derives.
fn cost_source(row: &UsageBucket) -> &'static str {
    if row.unpriced_records >= row.records {
        "unpriced"
    } else {
        "providerReported"
    }
}

fn bucket(row: &UsageBucket) -> Value {
    let mut out = json!({
        "day": row.day,
        "provider": row.provider,
        "model": row.model,
        "totals": {
            "uncachedInputTokens": row.uncached_input_tokens.max(0),
            "cachedInputTokens": row.cached_input_tokens.max(0),
            "cacheCreationTokens": row.cache_creation_tokens.max(0),
            "outputTokens": row.output_tokens.max(0),
            "reasoningTokens": row.reasoning_tokens.max(0),
        },
        "costUsd": row.cost_usd,
        "cacheSavingsUsd": cache_savings_usd(&row.model, row.cached_input_tokens),
        "costSource": cost_source(row),
        "records": row.records.max(0),
        "unpricedRecords": row.unpriced_records.max(0),
        "sessions": row.sessions.max(0),
    });
    // Absent rather than null on a daily read: the field is optional in the
    // contract and `Schema.optional` rejects an explicit null.
    if let Some(hour) = &row.hour_start {
        out["hourStart"] = json!(hour);
    }
    out
}

/// The one source this deployment reports.
///
/// `UsageSourceFingerprint` normally dedupes two T3 servers reading one
/// transcript directory by `device:inode`; here nothing reads a directory, so
/// it names the deployment instead and leaves `volumeId` empty.
fn source_for(provider: &str, sessions: i64, records: i64) -> Value {
    json!({
        "fingerprint": {
            "hostId": "moatless",
            "provider": provider,
            "resolvedHomePath": format!("moatless://task_turns/{provider}"),
            "volumeId": "",
        },
        "status": "ok",
        "scannedFiles": records.max(0),
        "skippedFiles": 0,
        "malformedRecords": 0,
        "distinctSessions": sessions.max(0),
        "message": Value::Null,
    })
}

/// Build the `UsageSummary` reply from the rows a read returned.
///
/// `read_at` and `scan_duration_ms` are passed in so the caller owns the clock,
/// keeping this function pure and testable.
pub(crate) fn summary(
    window: &UsageWindow,
    rows: &[UsageBucket],
    read_at: &str,
    scan_duration_ms: u64,
) -> Value {
    let mut sources: Vec<Value> = Vec::new();
    for provider in ["claude", "codex"] {
        // Sessions are summed per provider from the provider's own rows. A
        // Task spans models, so its bucket-level session counts overlap and
        // this reports the distinct-Task count the rows carry rather than a
        // sum across models.
        let provider_rows: Vec<&UsageBucket> =
            rows.iter().filter(|r| r.provider == provider).collect();
        if provider_rows.is_empty() {
            continue;
        }
        let sessions = provider_rows.iter().map(|r| r.sessions).max().unwrap_or(0);
        let records = provider_rows.iter().map(|r| r.records).sum();
        sources.push(source_for(provider, sessions, records));
    }

    json!({
        "contractVersion": CONTRACT_VERSION,
        "readAt": read_at,
        "timeZone": window.time_zone,
        "sinceDay": window.since_day,
        "untilDay": window.until_day,
        "buckets": rows.iter().map(bucket).collect::<Vec<_>>(),
        "sources": sources,
        "pricing": {
            // The rates behind `cacheSavingsUsd` are compiled in, not fetched,
            // so there is nothing to be stale and no fetch time to report.
            "status": "cached",
            "source": "moatless-builtin",
            "fetchedAt": Value::Null,
            "knownModels": ClaudeModel::ALL.len() + CodexModel::ALL.len(),
        },
        "scanDurationMs": scan_duration_ms,
    })
}

/// Read the requested window off the payload.
///
/// `Err` carries the `detail` of a `UsageReadError { reason: "invalidWindow" }`
/// — the client renders it, so it names the field at fault.
pub(crate) fn window_from(payload: &Value) -> Result<UsageWindow, String> {
    let day = |key: &str| -> Result<String, String> {
        let raw = payload
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{key} is required."))?;
        // The contract brands these `YYYY-MM-DD`; a value of another shape
        // would reach SQL as a date literal and fail there instead.
        if raw.len() != 10
            || !raw
                .as_bytes()
                .iter()
                .all(|b| b.is_ascii_digit() || *b == b'-')
        {
            return Err(format!("{key} must be formatted YYYY-MM-DD."));
        }
        Ok(raw.to_string())
    };
    let since_day = day("sinceDay")?;
    let until_day = day("untilDay")?;
    if since_day > until_day {
        return Err("sinceDay must not be after untilDay.".to_string());
    }
    let time_zone = payload
        .get("timeZone")
        .and_then(Value::as_str)
        .filter(|zone| !zone.trim().is_empty())
        .ok_or_else(|| "timeZone is required.".to_string())?
        .to_string();
    Ok(UsageWindow {
        since_day,
        until_day,
        time_zone,
        hourly: payload.get("resolution").and_then(Value::as_str) == Some("hour"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(model: &str, cached: i64) -> UsageBucket {
        UsageBucket {
            day: "2026-09-04".to_string(),
            hour_start: None,
            provider: "claude".to_string(),
            model: model.to_string(),
            uncached_input_tokens: 30,
            cached_input_tokens: cached,
            cache_creation_tokens: 6241,
            output_tokens: 4114,
            reasoning_tokens: 0,
            cost_usd: 1.72,
            records: 3,
            unpriced_records: 0,
            sessions: 2,
        }
    }

    fn window() -> UsageWindow {
        UsageWindow {
            since_day: "2026-09-01".to_string(),
            until_day: "2026-09-04".to_string(),
            time_zone: "UTC".to_string(),
            hourly: false,
        }
    }

    #[test]
    fn a_daily_bucket_carries_no_hour_start_key_at_all() {
        let value = bucket(&row("claude-opus-4-8", 1_000));
        assert!(
            value.get("hourStart").is_none(),
            "an explicit null fails the contract's Schema.optional"
        );
    }

    #[test]
    fn an_hourly_bucket_carries_the_hour_it_covers() {
        let mut source = row("claude-opus-4-8", 1_000);
        source.hour_start = Some("2026-09-04T11:00:00Z".to_string());
        assert_eq!(bucket(&source)["hourStart"], "2026-09-04T11:00:00Z");
    }

    /// Cache reads bill at a tenth of fresh input across the table, so the
    /// saving is the other nine tenths.
    #[test]
    fn cache_savings_is_the_gap_between_the_input_and_cache_read_rates() {
        assert!((cache_savings_usd("claude-opus-4-8", 1_000_000) - 4.5).abs() < 1e-9);
        assert!((cache_savings_usd("claude-fable-5", 1_000_000) - 9.0).abs() < 1e-9);
    }

    /// The same ordering trap the backend's price table carries: `sonnet-5`
    /// contains `sonnet`, and matching the family first bills it as the older
    /// model.
    #[test]
    fn sonnet_5_is_matched_before_the_family_name_it_contains() {
        assert!((cache_savings_usd("claude-sonnet-5", 1_000_000) - 1.8).abs() < 1e-9);
        assert!((cache_savings_usd("claude-sonnet-4-6", 1_000_000) - 2.7).abs() < 1e-9);
    }

    #[test]
    fn an_unknown_model_saves_nothing_rather_than_a_guessed_amount() {
        assert!((cache_savings_usd("some-other-model", 1_000_000)).abs() < 1e-9);
    }

    #[test]
    fn a_bucket_no_run_priced_reports_unpriced() {
        let mut source = row("claude-opus-4-8", 1_000);
        source.unpriced_records = source.records;
        assert_eq!(bucket(&source)["costSource"], "unpriced");
    }

    #[test]
    fn a_bucket_any_run_priced_reports_the_provider() {
        let mut source = row("claude-opus-4-8", 1_000);
        source.unpriced_records = source.records - 1;
        assert_eq!(bucket(&source)["costSource"], "providerReported");
    }

    #[test]
    fn a_summary_reports_one_source_per_provider_present() {
        let mut codex = row("gpt-5-codex", 500);
        codex.provider = "codex".to_string();
        let value = summary(&window(), &[row("claude-opus-4-8", 1_000), codex], "now", 7);
        let sources = value["sources"].as_array().expect("sources");
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0]["fingerprint"]["provider"], "claude");
        assert_eq!(sources[1]["fingerprint"]["provider"], "codex");
        assert_eq!(value["contractVersion"], CONTRACT_VERSION);
        assert_eq!(value["scanDurationMs"], 7);
    }

    #[test]
    fn a_provider_with_no_rows_reports_no_source() {
        let value = summary(&window(), &[row("claude-opus-4-8", 1_000)], "now", 1);
        assert_eq!(value["sources"].as_array().expect("sources").len(), 1);
    }

    #[test]
    fn a_window_needs_both_days_and_a_zone() {
        let ok = window_from(&json!({
            "sinceDay": "2026-09-01", "untilDay": "2026-09-04", "timeZone": "Europe/Stockholm"
        }))
        .expect("valid window");
        assert_eq!(ok.time_zone, "Europe/Stockholm");
        assert!(!ok.hourly);

        assert!(window_from(&json!({"untilDay": "2026-09-04", "timeZone": "UTC"})).is_err());
        assert!(window_from(&json!({"sinceDay": "2026-09-01", "untilDay": "2026-09-04"})).is_err());
    }

    /// A day that reaches SQL as a date literal must be rejected here, where
    /// the message can name the field.
    #[test]
    fn a_day_of_the_wrong_shape_is_refused_before_it_reaches_sql() {
        let bad = json!({"sinceDay": "01/09/2026", "untilDay": "2026-09-04", "timeZone": "UTC"});
        assert!(window_from(&bad).unwrap_err().contains("sinceDay"));
    }

    #[test]
    fn a_backwards_window_is_refused() {
        let bad = json!({"sinceDay": "2026-09-05", "untilDay": "2026-09-04", "timeZone": "UTC"});
        assert!(window_from(&bad).is_err());
    }

    #[test]
    fn an_hourly_resolution_is_read_off_the_payload() {
        let hourly = window_from(&json!({
            "sinceDay": "2026-09-04", "untilDay": "2026-09-04",
            "timeZone": "UTC", "resolution": "hour"
        }))
        .expect("valid window");
        assert!(hourly.hourly);
    }
}
