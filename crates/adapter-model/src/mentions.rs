//! The canonical mention the agent writes, and how an Adapter renders it.
//!
//! The agent addresses people in Moatless terms — `<@login>` — because that is
//! the vocabulary the sender label already puts in front of it, and because the
//! `<…>` delimiter makes the ping deliberate: a bare `@name` sweep would catch
//! `@moatless/api-client` and turn every passing reference into a notification.
//!
//! An Adapter's own mention passes through untouched, so a person the Adapter
//! can address but Moatless has never seen is still reachable by raw id.

use std::collections::HashMap;
use std::sync::LazyLock;

use moatless_ports::identity::ProviderIdentityLink;
use regex::Regex;
use tracing::warn;

/// `<@token>` — the shape both the canonical mention and every Adapter's own
/// mention share. Which one a token is, is decided by `is_native_id`.
// A constant pattern, so it cannot fail to compile.
#[allow(clippy::expect_used)]
static MENTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<@([A-Za-z0-9._\-]+)>").expect("valid regex"));

/// A run of `text` handed back by [`split_mentions`], either a mention's token
/// or the plain text between two of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MentionPiece<'a> {
    Literal(&'a str),
    Token(&'a str),
}

/// Split `text` into its mention tokens and the literals around them, in order.
/// Concatenating the pieces back with `<@…>` around each token rebuilds `text`.
///
/// For an Adapter whose surface is not one string: a rich-text leaf resolves
/// each token into an element of its own rather than splicing a rendered
/// mention back into the leaf.
pub fn split_mentions(text: &str) -> impl Iterator<Item = MentionPiece<'_>> {
    let mut pieces = Vec::new();
    let mut cut = 0;
    for found in MENTION.find_iter(text) {
        if found.start() > cut {
            pieces.push(MentionPiece::Literal(&text[cut..found.start()]));
        }
        // `<@` and `>` are one byte each, so the token is the span inside them.
        pieces.push(MentionPiece::Token(
            &text[found.start() + 2..found.end() - 1],
        ));
        cut = found.end();
    }
    if cut < text.len() {
        pieces.push(MentionPiece::Literal(&text[cut..]));
    }
    pieces.into_iter()
}

/// What one token resolves to, before an Adapter renders it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mention {
    /// An id the Adapter can address. Notifies.
    Native(String),
    /// A name to print, for someone the Adapter cannot address. Notifies
    /// nobody, and names who was meant.
    Plain(String),
}

impl Mention {
    /// The text this becomes on a surface that takes the Adapter's own mention
    /// syntax.
    pub fn render(&self, render_native: impl Fn(&str) -> String) -> String {
        match self {
            Self::Native(id) => render_native(id),
            Self::Plain(name) => format!("@{name}"),
        }
    }
}

/// Resolve one token against `provider`'s directory.
///
/// The Adapter's own id passes through unresolved; a Moatless login linked at
/// `provider` resolves to the linked id; anything else stays plain. A failed
/// lookup is treated as unlinked, since a reply must never be held up or ping
/// the wrong person.
pub async fn resolve_mention(
    users: &dyn ProviderIdentityLink,
    provider: &str,
    token: &str,
    is_native_id: impl Fn(&str) -> bool,
) -> Mention {
    if is_native_id(token) {
        return Mention::Native(token.to_string());
    }
    match users.provider_id_for_login(provider, token).await {
        Ok(Some(provider_user_id)) => Mention::Native(provider_user_id),
        Ok(None) => {
            warn!(
                provider,
                login = %token,
                "Mention left as plain text and notified nobody: the token is not \
                 a Moatless login with an account linked at this provider"
            );
            Mention::Plain(token.to_string())
        }
        Err(e) => {
            warn!(
                provider,
                login = %token,
                error = %e,
                "Mention left as plain text: the provider-id lookup failed"
            );
            Mention::Plain(token.to_string())
        }
    }
}

/// Rewrite every `<@…>` in `text` into what `provider` can address, per
/// [`resolve_mention`].
pub async fn render_mentions(
    users: &dyn ProviderIdentityLink,
    provider: &str,
    text: &str,
    is_native_id: impl Fn(&str) -> bool,
    render_native: impl Fn(&str) -> String,
) -> String {
    // Resolve first, then splice: the rewrite itself is sync, and the lookups
    // are per distinct token rather than per occurrence.
    let pieces: Vec<MentionPiece<'_>> = split_mentions(text).collect();
    let mut tokens: Vec<&str> = pieces
        .iter()
        .filter_map(|piece| match piece {
            MentionPiece::Token(token) => Some(*token),
            MentionPiece::Literal(_) => None,
        })
        .collect();
    tokens.sort_unstable();
    tokens.dedup();

    let mut resolved: HashMap<&str, Mention> = HashMap::with_capacity(tokens.len());
    for token in tokens {
        resolved.insert(
            token,
            resolve_mention(users, provider, token, &is_native_id).await,
        );
    }

    let mut out = String::with_capacity(text.len());
    for piece in pieces {
        match piece {
            MentionPiece::Literal(literal) => out.push_str(literal),
            MentionPiece::Token(token) => out.push_str(
                &resolved
                    .get(token)
                    .map_or_else(|| format!("@{token}"), |m| m.render(&render_native)),
            ),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use moatless_ports::identity::ProviderIdentityLink;
    use moatless_types::store::StoreError;

    use super::{Mention, MentionPiece, render_mentions, resolve_mention, split_mentions};

    /// Knows one link: `aorwall` is `U0000000001` on Slack.
    struct OneLink {
        fail: bool,
    }

    #[async_trait]
    impl ProviderIdentityLink for OneLink {
        async fn provider_id_for_login(
            &self,
            provider: &str,
            login: &str,
        ) -> Result<Option<String>, StoreError> {
            if self.fail {
                return Err(StoreError::new("the directory is down"));
            }
            Ok((provider == "slack" && login == "aorwall").then(|| "U0000000001".to_string()))
        }
    }

    fn slack_id(token: &str) -> bool {
        token.len() >= 8
            && token.starts_with(['U', 'W'])
            && token[1..]
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    }

    async fn render(text: &str, fail: bool) -> String {
        render_mentions(&OneLink { fail }, "slack", text, slack_id, |id| {
            format!("<@{id}>")
        })
        .await
    }

    #[tokio::test]
    async fn a_linked_login_becomes_the_providers_own_mention() {
        assert_eq!(
            render("ping <@aorwall> about it", false).await,
            "ping <@U0000000001> about it"
        );
    }

    /// The escape hatch: an id the Adapter can already address is never looked
    /// up, so a person Moatless has never seen stays reachable.
    #[tokio::test]
    async fn a_raw_provider_id_passes_through() {
        assert_eq!(
            render("ping <@U0NEVERSEEN> about it", false).await,
            "ping <@U0NEVERSEEN> about it"
        );
    }

    #[tokio::test]
    async fn an_unlinked_login_is_named_in_plain_text_and_pings_nobody() {
        assert_eq!(
            render("ping <@martin> about it", false).await,
            "ping @martin about it"
        );
    }

    /// A mention is never worth failing a reply over, and plain text cannot
    /// name the wrong person.
    #[tokio::test]
    async fn a_failed_lookup_degrades_to_plain_text() {
        assert_eq!(render("ping <@aorwall>", true).await, "ping @aorwall");
    }

    #[tokio::test]
    async fn text_that_only_looks_like_a_mention_is_untouched() {
        let text = "install @moatless/api-client, mail ops@host, see <https://x.test|docs>";
        assert_eq!(render(text, false).await, text);
    }

    #[tokio::test]
    async fn every_occurrence_of_a_token_is_rewritten_once_resolved() {
        assert_eq!(
            render("<@aorwall> and <@aorwall> and <@martin>", false).await,
            "<@U0000000001> and <@U0000000001> and @martin"
        );
    }

    #[test]
    fn a_split_alternates_literals_and_tokens_and_drops_nothing() {
        let pieces: Vec<_> = split_mentions("hi <@aorwall>, ping <@martin> now").collect();
        assert_eq!(
            pieces,
            [
                MentionPiece::Literal("hi "),
                MentionPiece::Token("aorwall"),
                MentionPiece::Literal(", ping "),
                MentionPiece::Token("martin"),
                MentionPiece::Literal(" now"),
            ]
        );
    }

    /// A leaf that is nothing but a mention yields no empty literal on either
    /// side, so an Adapter building one element per piece emits exactly one.
    #[test]
    fn a_token_alone_splits_into_one_piece() {
        let pieces: Vec<_> = split_mentions("<@aorwall>").collect();
        assert_eq!(pieces, [MentionPiece::Token("aorwall")]);
    }

    #[test]
    fn text_with_no_mention_splits_into_one_literal() {
        let pieces: Vec<_> = split_mentions("mail ops@host").collect();
        assert_eq!(pieces, [MentionPiece::Literal("mail ops@host")]);
    }

    #[tokio::test]
    async fn resolving_one_token_names_the_three_outcomes() {
        let link = OneLink { fail: false };
        assert_eq!(
            resolve_mention(&link, "slack", "aorwall", slack_id).await,
            Mention::Native("U0000000001".to_string())
        );
        assert_eq!(
            resolve_mention(&link, "slack", "U0NEVERSEEN", slack_id).await,
            Mention::Native("U0NEVERSEEN".to_string())
        );
        assert_eq!(
            resolve_mention(&link, "slack", "martin", slack_id).await,
            Mention::Plain("martin".to_string())
        );
    }

    #[test]
    fn a_plain_mention_renders_without_consulting_the_adapter() {
        let rendered = Mention::Plain("martin".to_string()).render(|_| unreachable!());
        assert_eq!(rendered, "@martin");
    }
}
