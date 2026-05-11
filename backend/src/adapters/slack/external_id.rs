/// `slack:ts:<ts>` - Slack message timestamp.
pub fn message_ts(ts: &str) -> String {
    format!("slack:ts:{ts}")
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
}
