//! Git hosting providers and the accounts used to resolve credentials.

crate::string_enum!(
    /// Git hosting provider.
    RepositoryProvider {
        Github => "github",
        Gitlab => "gitlab",
        Forgejo => "forgejo",
    }
);

/// The account or instance whose git credential is requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHostAccount {
    /// `repository` is the name within `owner` the credential is used for;
    /// `None` asks for the whole account, which a capped identity is refused.
    Github {
        owner: String,
        repository: Option<String>,
    },
    Forgejo {
        host: String,
    },
    Gitlab,
}

impl GitHostAccount {
    pub fn provider(&self) -> RepositoryProvider {
        match self {
            Self::Github { .. } => RepositoryProvider::Github,
            Self::Forgejo { .. } => RepositoryProvider::Forgejo,
            Self::Gitlab => RepositoryProvider::Gitlab,
        }
    }
}

impl std::fmt::Display for GitHostAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Github { owner, .. } => write!(f, "github account {owner}"),
            Self::Forgejo { host } => write!(f, "forgejo instance {host}"),
            Self::Gitlab => f.write_str("gitlab"),
        }
    }
}
