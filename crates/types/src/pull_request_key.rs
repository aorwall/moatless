//! The `subject_key` of a GitHub pull-request binding: `owner/repo#N`.
//!
//! [`pull_request_key`] writes it, [`parse_pull_request_key`] reads it, and
//! [`PULL_REQUEST_KEY_PATTERN`] is the same rule for a query that filters on
//! it. The tests pin all three to one another.

/// The key a pull-request binding is stored under.
pub fn pull_request_key(repository: &str, number: u64) -> String {
    format!("{repository}#{number}")
}

/// The `owner/repo` and number a key names, or `None` for any key
/// [`pull_request_key`] could not have written from an `owner/repo`.
pub fn parse_pull_request_key(key: &str) -> Option<(&str, u64)> {
    let (repository, number) = key.split_once('#')?;
    let (owner, repo) = repository.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    if number.is_empty()
        || number.len() > MAX_DIGITS
        || !number.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    Some((repository, number.parse().ok()?))
}

/// The most digits a number may have, so it always fits a `bigint` and a `u64`
/// rather than failing the cast of every row in the query that reads it.
const MAX_DIGITS: usize = 18;

/// [`parse_pull_request_key`] as a POSIX regular expression, for a SQL `~`.
///
/// A row it matches splits with `split_part(key, '#', 1)` and `2`, and the
/// number casts to `bigint`. The `{1,18}` is [`MAX_DIGITS`].
pub const PULL_REQUEST_KEY_PATTERN: &str = "^[^/#]+/[^/#]+#[0-9]{1,18}$";

#[cfg(test)]
mod tests {
    use super::*;

    const KEYS: &[&str] = &[
        "octo/repo#42",
        "octo/repo#0",
        "octo/repo#999999999999999999",
        "octo/repo#9999999999999999999",
        "my-org/my.repo_1#7",
        "octo/repo#",
        "octo/repo#4a",
        "octo/repo#-1",
        "octo/repo#1#2",
        "octo#1",
        "/repo#1",
        "octo/#1",
        "octo/repo/extra#1",
        "a#b/c#1",
        "octo/repo",
        "",
    ];

    #[test]
    fn a_written_key_parses_back() {
        let key = pull_request_key("octo/repo", 42);
        assert_eq!(key, "octo/repo#42");
        assert_eq!(parse_pull_request_key(&key), Some(("octo/repo", 42)));
    }

    #[test]
    fn the_sql_pattern_accepts_exactly_what_the_parser_accepts() {
        let pattern = regex::Regex::new(PULL_REQUEST_KEY_PATTERN).unwrap();
        for key in KEYS {
            assert_eq!(
                pattern.is_match(key),
                parse_pull_request_key(key).is_some(),
                "{key:?}"
            );
        }
    }

    #[test]
    fn only_owner_repo_and_digits_parse() {
        let parsed: Vec<_> = KEYS
            .iter()
            .filter(|key| parse_pull_request_key(key).is_some())
            .collect();
        assert_eq!(
            parsed,
            [
                &"octo/repo#42",
                &"octo/repo#0",
                &"octo/repo#999999999999999999",
                &"my-org/my.repo_1#7"
            ]
        );
    }
}
