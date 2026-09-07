/// Normalize a remote URL for identity matching without changing persisted spelling.
pub(crate) fn normalize_remote_url(remote_url: &str) -> String {
    remote_url
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}
