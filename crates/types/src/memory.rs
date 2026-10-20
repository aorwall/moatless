//! Memory vocabulary: the scopes an entry can belong to and what was done to it.

crate::string_enum!(
    /// Which Tasks share a memory entry. The scope's id is never named by a
    /// caller: it comes from the Task the request is made for.
    MemoryScope [Hash] {
        /// The calling Task only.
        Task => "task",
        /// Every Task the calling Task's Loop fires.
        Loop => "loop",
        /// Every Task in the calling Task's Workspace.
        Workspace => "workspace",
        /// Every Task against the calling Task's repository.
        Repository => "repository",
        /// Every Task of the calling Task's owner.
        User => "user",
        /// Every Task.
        Global => "global",
    }
);

crate::string_enum!(
    /// A write recorded in a memory entry's history.
    MemoryOp {
        Put => "put",
        Append => "append",
        Verify => "verify",
        Claim => "claim",
        Release => "release",
    }
);

/// Longest memory key, in bytes.
pub const MAX_MEMORY_KEY_LEN: usize = 200;

/// Whether `key` is a memory key: ASCII letters, digits and `:._/-`, starting
/// with a letter or digit, at most [`MAX_MEMORY_KEY_LEN`] bytes.
pub fn is_valid_key(key: &str) -> bool {
    key.len() <= MAX_MEMORY_KEY_LEN
        && key
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '.' | '_' | '/' | '-'))
}

/// Why a LOOP.md `memory.preload` pattern is unusable, if it is: an exact key,
/// or a key prefix ending in `*`. A bare `*` is refused because it would
/// preload every visible entry.
pub fn preload_pattern_error(pattern: &str) -> Option<String> {
    let key = pattern.strip_suffix('*').unwrap_or(pattern);
    (!is_valid_key(key)).then(|| {
        format!(
            "memory.preload pattern {pattern:?} must be a key or a key prefix ending in `*`: \
             ASCII letters, digits and ':._/-', starting with a letter or digit"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_ascii_words_and_separators() {
        assert!(is_valid_key("triage:SP-123"));
        assert!(is_valid_key("repo/ci_flaky.v2"));
        assert!(!is_valid_key(""));
        assert!(!is_valid_key(":leading"));
        assert!(!is_valid_key("has space"));
        assert!(!is_valid_key(&"k".repeat(MAX_MEMORY_KEY_LEN + 1)));
    }

    #[test]
    fn a_preload_pattern_is_a_key_or_a_prefix_but_never_everything() {
        assert!(preload_pattern_error("ci:flaky").is_none());
        assert!(preload_pattern_error("triage:*").is_none());
        assert!(preload_pattern_error("*").is_some());
        assert!(preload_pattern_error("").is_some());
        assert!(preload_pattern_error("bad key*").is_some());
    }
}
