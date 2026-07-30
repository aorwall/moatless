/// `slack:ts:<ts>` - Slack message timestamp.
pub fn message_ts(ts: &str) -> String {
    format!("slack:ts:{ts}")
}

/// Extract the bare Slack `ts` from a namespaced `slack:ts:<ts>` external id,
/// or pass through a bare `ts` already in unwrapped form.
pub fn ts_from_message_id(external_id: &str) -> Option<&str> {
    if let Some(ts) = external_id.strip_prefix("slack:ts:") {
        if ts.is_empty() {
            return None;
        }
        return Some(ts);
    }
    // Legacy bare ts: must be `<int>.<int>`.
    let (head, tail) = external_id.split_once('.')?;
    if !head.is_empty()
        && !tail.is_empty()
        && head.chars().all(|c| c.is_ascii_digit())
        && tail.chars().all(|c| c.is_ascii_digit())
    {
        Some(external_id)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_ts_format() {
        assert_eq!(
            message_ts("1700000000.000100"),
            "slack:ts:1700000000.000100"
        );
    }

    #[test]
    fn ts_from_message_id_strips_namespace() {
        assert_eq!(
            ts_from_message_id("slack:ts:1700000000.000100"),
            Some("1700000000.000100")
        );
    }

    #[test]
    fn ts_from_message_id_accepts_legacy_bare_ts() {
        assert_eq!(
            ts_from_message_id("1700000000.000100"),
            Some("1700000000.000100")
        );
    }

    #[test]
    fn ts_from_message_id_rejects_other_namespaces() {
        assert_eq!(ts_from_message_id("github:ic:42"), None);
        assert_eq!(ts_from_message_id("slack:ts:"), None);
        assert_eq!(ts_from_message_id("nonsense"), None);
    }
}
