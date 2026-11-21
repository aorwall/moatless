//! Token → cost pricing for runs whose transport reports no cost.
//!
//! Claude prices its own run and reports `total_cost_usd`; Codex, Antigravity
//! and the tmux backend report none, so the backend prices those from the
//! run's `model` and its per-tier token counts. Prices are USD per 1M tokens.

use super::models::UsageInfo;
use moatless_types::model::{ModelPrice, price_for};

/// Estimated USD cost of a run from its `model` and per-tier token counts.
///
/// `usage.input_tokens` is the non-cached input (both harnesses normalize to
/// this at the seam); `output_tokens` already includes reasoning tokens, so
/// reasoning isn't priced twice. `None` when the model is unrecognised.
pub fn estimate_cost_usd(model: &str, usage: &UsageInfo) -> Option<f64> {
    let Some(ModelPrice {
        input,
        cache_write,
        cache_read,
        output,
    }) = price_for(model)
    else {
        tracing::warn!(model, "No price for model; the run is stored unpriced");
        return None;
    };
    let tok = |v: Option<i64>| f64::from(i32::try_from(v.unwrap_or(0).max(0)).unwrap_or(i32::MAX));
    Some(
        (tok(usage.input_tokens) * input
            + tok(usage.cache_creation_input_tokens) * cache_write
            + tok(usage.cache_read_input_tokens) * cache_read
            + tok(usage.output_tokens) * output)
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
        assert_eq!(cost, Some(5.0 + 6.25 + 0.5 + 25.0));
    }

    #[test]
    fn fable_is_priced_above_opus_not_alongside_it() {
        let u = usage(1_000_000, 1_000_000, 1_000_000, 1_000_000);
        assert_eq!(estimate_cost_usd("claude-fable-5", &u), Some(73.5));
    }

    #[test]
    fn sonnet_5_is_priced_below_the_older_sonnet() {
        let u = usage(1_000_000, 1_000_000, 1_000_000, 1_000_000);
        assert_eq!(estimate_cost_usd("claude-sonnet-5", &u), Some(14.7));
        assert_eq!(estimate_cost_usd("claude-sonnet-4-6", &u), Some(22.05));
    }

    #[test]
    fn prices_gpt_6_and_gpt_5_6_ahead_of_the_gpt_5_arm() {
        let u = usage(1_000_000, 0, 1_000_000, 1_000_000);
        assert_eq!(
            estimate_cost_usd("gpt-6-astra", &u),
            Some(10.0 + 1.0 + 50.0)
        );
        assert_eq!(estimate_cost_usd("gpt-6-sol", &u), Some(2.0 + 0.2 + 10.0));
        assert_eq!(estimate_cost_usd("gpt-5.6-sol", &u), Some(4.0 + 0.4 + 20.0));
        assert!(estimate_cost_usd("gpt-6-nova", &u).is_none());
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
