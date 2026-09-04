//! Token → cost pricing for runs whose transport reports no cost.
//!
//! Claude prices its own run and reports `total_cost_usd`; Codex and the tmux
//! backend report none, so the backend prices those from the run's `model` and
//! its per-tier token counts. Prices are USD per 1M tokens.

use super::models::UsageInfo;

/// Per-1M-token USD prices for one model's tiers. `cache_write` prices the
/// cache-creation input; `cache_read` prices the cached-prefix reads.
struct ModelPrice {
    input: f64,
    cache_write: f64,
    cache_read: f64,
    output: f64,
}

/// Prices for the model, matched by the substrings its published id carries.
/// `None` for an unrecognised model, which leaves the run unpriced rather than
/// billed at a guessed rate.
fn price_for(model: &str) -> Option<ModelPrice> {
    let m = model.to_ascii_lowercase();
    // Codex ids are `gpt-5*` / `*codex*`; cached input is priced, cache-write is
    // not a Codex tier (its cache-creation count is always 0) so it rides the
    // input rate.
    if m.contains("codex") || m.contains("gpt-5") {
        Some(ModelPrice {
            input: 1.25,
            cache_write: 1.25,
            cache_read: 0.125,
            output: 10.0,
        })
    } else if m.contains("opus") || m.contains("fable") {
        Some(ModelPrice {
            input: 15.0,
            cache_write: 18.75,
            cache_read: 1.5,
            output: 75.0,
        })
    } else if m.contains("sonnet") {
        Some(ModelPrice {
            input: 3.0,
            cache_write: 3.75,
            cache_read: 0.3,
            output: 15.0,
        })
    } else if m.contains("haiku") {
        Some(ModelPrice {
            input: 1.0,
            cache_write: 1.25,
            cache_read: 0.1,
            output: 5.0,
        })
    } else {
        None
    }
}

/// Estimated USD cost of a run from its `model` and per-tier token counts.
///
/// `usage.input_tokens` is the non-cached input (both harnesses normalize to
/// this at the seam), so the cached-prefix reads are priced separately at the
/// cache-read rate. `output_tokens` already includes any reasoning tokens, so
/// reasoning is not priced again. `None` when the model is unrecognised.
pub fn estimate_cost_usd(model: &str, usage: &UsageInfo) -> Option<f64> {
    let price = price_for(model)?;
    let tok = |v: Option<i64>| f64::from(i32::try_from(v.unwrap_or(0).max(0)).unwrap_or(i32::MAX));
    Some(
        (tok(usage.input_tokens) * price.input
            + tok(usage.cache_creation_input_tokens) * price.cache_write
            + tok(usage.cache_read_input_tokens) * price.cache_read
            + tok(usage.output_tokens) * price.output)
            / 1_000_000.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: i64, cache_write: i64, cache_read: i64, output: i64) -> UsageInfo {
        UsageInfo {
            input_tokens: Some(input),
            output_tokens: Some(output),
            cache_creation_input_tokens: Some(cache_write),
            cache_read_input_tokens: Some(cache_read),
        }
    }

    #[test]
    fn prices_codex_from_net_input_cache_read_and_output() {
        // gpt-5-codex: 1M net input @1.25 + 1M cache-read @0.125 + 1M output @10.
        let cost = estimate_cost_usd("gpt-5-codex", &usage(1_000_000, 0, 1_000_000, 1_000_000));
        assert_eq!(cost, Some(1.25 + 0.125 + 10.0));
    }

    #[test]
    fn prices_claude_opus_tiers() {
        let cost = estimate_cost_usd(
            "claude-opus-4-8",
            &usage(1_000_000, 1_000_000, 1_000_000, 1_000_000),
        );
        assert_eq!(cost, Some(15.0 + 18.75 + 1.5 + 75.0));
    }

    #[test]
    fn unrecognised_model_is_unpriced() {
        assert!(estimate_cost_usd("some-other-model", &usage(1, 1, 1, 1)).is_none());
    }

    #[test]
    fn missing_token_fields_count_as_zero() {
        let u = UsageInfo {
            input_tokens: None,
            output_tokens: Some(1_000_000),
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        };
        assert_eq!(estimate_cost_usd("gpt-5-codex", &u), Some(10.0));
    }
}
