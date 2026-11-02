use crate::agent::harness::error::{
    ErrorClass, FailureStage, HarnessError, SessionImpact, is_provider_unavailable,
    is_safety_refusal,
};

pub(super) fn provider_error(text: &str) -> HarnessError {
    let lower = text.to_lowercase();
    let contains = |patterns: &[&str]| patterns.iter().any(|p| lower.contains(p));
    let class = if is_safety_refusal(&lower) {
        ErrorClass::InvalidInput
    } else if is_provider_unavailable(&lower) {
        ErrorClass::ProviderUnavailable
    } else if contains(&["401", "403", "api key", "authentication", "unauthorized"]) {
        ErrorClass::Authentication
    } else if contains(&["usage limit", "quota", "credit balance", "extra usage"]) {
        ErrorClass::UsageLimit
    } else if contains(&["429", "rate limit"]) {
        ErrorClass::RateLimit
    } else if contains(&["overloaded", "503", "502", "500"]) {
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
        ] {
            assert_eq!(provider_error(text).class, class, "{text}");
        }
    }
}
