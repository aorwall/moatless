use crate::agent::harness::error::{
    ErrorClass, FailureStage, HarnessError, SessionImpact, is_provider_unavailable,
    is_safety_refusal, names_status,
};

pub(super) fn provider_error(text: &str) -> HarnessError {
    let lower = text.to_lowercase();
    let contains = |patterns: &[&str]| patterns.iter().any(|p| lower.contains(p));
    let class = if is_safety_refusal(&lower) {
        ErrorClass::InvalidInput
    } else if is_provider_unavailable(&lower) {
        ErrorClass::ProviderUnavailable
    } else if names_status(&lower, &["401", "403"])
        || contains(&["api key", "authentication", "unauthorized"])
    {
        ErrorClass::Authentication
    } else if contains(&["usage limit", "quota", "credit balance", "extra usage"]) {
        ErrorClass::UsageLimit
    } else if names_status(&lower, &["429"]) || contains(&["rate limit"]) {
        ErrorClass::RateLimit
    } else if names_status(&lower, &["503", "502", "500"]) || contains(&["overloaded"]) {
        ErrorClass::ProviderUnavailable
    } else if contains(&["fetch failed", "connection", "timeout", "enotfound"]) {
        ErrorClass::Connectivity
    } else {
        ErrorClass::InvalidInput
    };
    HarnessError::new(
        FailureStage::Execution,
        class,
        SessionImpact::Intact,
        text.chars().take(500).collect::<String>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_failures_retain_their_retry_class() {
        for (text, class) in [
            ("401 invalid API key", ErrorClass::Authentication),
            ("429 rate limit", ErrorClass::RateLimit),
            ("credit balance exhausted", ErrorClass::UsageLimit),
            ("fetch failed", ErrorClass::Connectivity),
            ("invalid model", ErrorClass::InvalidInput),
            (
                "HTTP 500 internal server error",
                ErrorClass::ProviderUnavailable,
            ),
            ("tool output was 1500 lines", ErrorClass::InvalidInput),
            (
                "invalid model claude-3-5-sonnet-20240401",
                ErrorClass::InvalidInput,
            ),
        ] {
            assert_eq!(provider_error(text).class, class, "{text}");
        }
    }
}
