/// `tg:m:<id>` - Telegram message id.
pub fn message(id: i64) -> String {
    format!("tg:m:{id}")
}

/// Extract the bare Telegram `message_id` from a `tg:m:<id>` external id.
pub fn message_id_from(external_id: &str) -> Option<i64> {
    external_id.strip_prefix("tg:m:")?.parse::<i64>().ok()
}

pub fn aliases(external_id: &str) -> Vec<String> {
    let Some(tail) = external_id.strip_prefix("tg:m:") else {
        return vec![external_id.to_string()];
    };
    if tail.is_empty() || !tail.chars().all(|c| c.is_ascii_digit()) {
        return vec![external_id.to_string()];
    }
    vec![external_id.to_string(), tail.to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_format() {
        assert_eq!(message(789), "tg:m:789");
    }

    #[test]
    fn message_id_from_extracts_int() {
        assert_eq!(message_id_from("tg:m:789"), Some(789));
        assert_eq!(message_id_from("tg:m:"), None);
        assert_eq!(message_id_from("789"), None);
    }

    #[test]
    fn aliases_include_legacy_bare_id() {
        assert_eq!(
            aliases("tg:m:789"),
            vec!["tg:m:789".to_string(), "789".to_string()]
        );
    }
}
