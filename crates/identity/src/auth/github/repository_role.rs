//! What a token minted for a person's work may do, given their repository role.

use std::collections::BTreeMap;

/// A person's role on one repository, as far as it caps a token.
///
/// `admin` and `maintain` fold into [`Self::Write`]: no token minted for a
/// person's work carries `administration`, so a bot cannot change branch
/// protection or settings on their behalf.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RepositoryRole {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    Read,
    Write,
    Admin,
}

impl Level {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "read" => Some(Self::Read),
            "write" => Some(Self::Write),
            "admin" => Some(Self::Admin),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Admin => "admin",
        }
    }
}

/// The repository permissions a person's role reaches, with the highest level
/// each takes for a reader and for a writer. Anything absent is never granted.
const CAPS: &[(&str, Option<Level>, Level)] = &[
    ("metadata", Some(Level::Read), Level::Read),
    ("contents", Some(Level::Read), Level::Write),
    ("pull_requests", Some(Level::Read), Level::Write),
    ("issues", Some(Level::Read), Level::Write),
    ("checks", Some(Level::Read), Level::Write),
    ("statuses", Some(Level::Read), Level::Write),
    ("actions", Some(Level::Read), Level::Write),
    ("deployments", Some(Level::Read), Level::Write),
    ("discussions", Some(Level::Read), Level::Write),
    // GitHub offers `workflows` at `write` only.
    ("workflows", None, Level::Write),
];

impl RepositoryRole {
    /// The role a `permission` from GitHub's collaborator-permission endpoint
    /// names; `None` for `none` or a value this build does not know.
    pub(crate) fn from_permission(permission: &str) -> Option<Self> {
        match permission {
            "admin" | "maintain" | "write" => Some(Self::Write),
            "triage" | "read" => Some(Self::Read),
            _ => None,
        }
    }

    /// `granted`, an installation's permissions, narrowed to this role.
    pub(crate) fn cap(self, granted: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        CAPS.iter()
            .filter_map(|&(name, read_cap, write_cap)| {
                let cap = match self {
                    Self::Read => read_cap?,
                    Self::Write => write_cap,
                };
                let granted = Level::parse(granted.get(name)?)?;
                Some((name.to_string(), granted.min(cap).as_str().to_string()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn permissions(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn app() -> BTreeMap<String, String> {
        permissions(&[
            ("administration", "write"),
            ("contents", "write"),
            ("metadata", "read"),
            ("pull_requests", "write"),
            ("secrets", "write"),
            ("workflows", "write"),
        ])
    }

    #[test]
    fn an_admin_gets_what_a_writer_gets_and_never_administration() {
        assert_eq!(
            RepositoryRole::from_permission("admin"),
            Some(RepositoryRole::Write)
        );
        assert_eq!(
            RepositoryRole::Write.cap(&app()),
            permissions(&[
                ("contents", "write"),
                ("metadata", "read"),
                ("pull_requests", "write"),
                ("workflows", "write"),
            ])
        );
    }

    #[test]
    fn a_reader_reads_only_what_the_app_was_granted() {
        assert_eq!(
            RepositoryRole::Read.cap(&app()),
            permissions(&[
                ("contents", "read"),
                ("metadata", "read"),
                ("pull_requests", "read"),
            ])
        );
    }

    #[test]
    fn a_role_never_raises_what_the_app_was_granted() {
        let granted = permissions(&[("contents", "read"), ("metadata", "read")]);
        assert_eq!(RepositoryRole::Write.cap(&granted), granted);
    }

    #[test]
    fn no_role_is_no_access() {
        assert_eq!(RepositoryRole::from_permission("none"), None);
        assert_eq!(
            RepositoryRole::from_permission("triage"),
            Some(RepositoryRole::Read)
        );
    }
}
