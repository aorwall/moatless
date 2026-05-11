/// `tg:m:<id>` - Telegram message id.
pub fn message(id: i64) -> String {
    format!("tg:m:{id}")
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
    fn aliases_include_legacy_bare_id() {
        assert_eq!(
            aliases("tg:m:789"),
            vec!["tg:m:789".to_string(), "789".to_string()]
        );
    }
}
