//! The canonical mention the agent writes, and how an Adapter renders it.
//!
//! The agent addresses people in Moatless terms — `<@login>` — because that is
//! the vocabulary the sender label already puts in front of it, and because the
//! `<…>` delimiter makes the ping deliberate: a bare `@name` sweep would catch
//! `@moatless/api-client` and turn every passing reference into a notification.
//!
//! An Adapter's own mention passes through untouched, so a person the Adapter
//! can address but Moatless has never seen is still reachable by raw id.

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

/// Rewrite every `<@…>` in `text` into what `provider` can address.
///
/// Three outcomes per token, in this order:
///
/// - `is_native_id` says it is already the Adapter's own id — rendered by
///   `render_native`, unresolved. This is what keeps a raw id usable.
/// - It is a Moatless login linked at `provider` — rendered by `render_native`
///   from the linked id, which is the ping the agent asked for.
/// - Neither — rendered as plain `@token`. It names who was meant without
///   claiming a link that does not exist, and pings nobody.
///
/// A lookup that fails is treated as unlinked: a reply must not be held up, and
/// plain text is never the wrong person.
pub async fn render_mentions(
    users: &dyn ProviderIdentityLink,
    provider: &str,
    text: &str,
    is_native_id: impl Fn(&str) -> bool,
    render_native: impl Fn(&str) -> String,
) -> String {
    // Resolve first, then splice: the rewrite itself is sync, and the lookups
    // are per distinct token rather than per occurrence.
    let mut rendered: Vec<(String, String)> = Vec::new();
    for capture in MENTION.captures_iter(text) {
        let token = capture[1].to_string();
        if rendered.iter().any(|(seen, _)| seen == &token) {
            continue;
        }

        let replacement = if is_native_id(&token) {
            render_native(&token)
        } else {
            match users.provider_id_for_login(provider, &token).await {
                Ok(Some(provider_user_id)) => render_native(&provider_user_id),
                Ok(None) => format!("@{token}"),
                Err(e) => {
                    warn!(
                        provider,
                        login = %token,
                        error = %e,
                        "Mention left as plain text: the provider-id lookup failed"
                    );
                    format!("@{token}")
                }
            }
        };
        rendered.push((token, replacement));
    }

    let mut out = text.to_string();
    for (token, replacement) in rendered {
        out = out.replace(&format!("<@{token}>"), &replacement);
    }
    out
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use moatless_ports::identity::{LinkedUser, ProviderIdentityLink};
    use moatless_types::store::StoreError;
    use uuid::Uuid;

    use super::render_mentions;

    /// Knows one link: `aorwall` is `U0000000001` on Slack.
    struct OneLink {
        fail: bool,
    }

    #[async_trait]
    impl ProviderIdentityLink for OneLink {
        async fn user_at(&self, _: &str, _: &str) -> Result<Option<LinkedUser>, StoreError> {
            Ok(None)
        }
        async fn user_by_email(&self, _: &str) -> Result<Option<LinkedUser>, StoreError> {
            Ok(None)
        }
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
        async fn user_who_connected(&self, _: &str, _: &str) -> Result<Option<String>, StoreError> {
            Ok(None)
        }
        async fn user_with_login(&self, _: &str, _: &str) -> Result<Option<String>, StoreError> {
            Ok(None)
        }
        async fn link(&self, _: &Uuid, _: &str, _: &str) -> Result<bool, StoreError> {
            Ok(false)
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
}
