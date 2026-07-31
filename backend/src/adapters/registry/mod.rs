//! Adapter registry and application configuration wiring.

use std::collections::HashMap;
use std::sync::Arc;

use crate::adapters::capabilities::{
    Adapter, AdapterBindingReply, AdapterBindingStatus, AdapterDestinationBuilder,
    AdapterDirectDestination, AdapterDirectMessage, AdapterIdentity, AdapterInboundPresentation,
    AdapterMessageReply, AdapterNewThread, AdapterPostBindingFollowup, AdapterReactions,
    AdapterUserIdentity,
};
use crate::adapters::error::AdapterError;

use moatless_api_schemas::enums::AdapterKind;

mod bootstrap;

/// Registry of external adapters keyed by AdapterKind.
///
/// Alongside the trait-object map, the registry keeps a typed handle for each
/// adapter that has concrete-only methods, so code inside an adapter's own
/// module reaches it without downcasting.
///
/// For those four kinds the typed registrar is the only way in: it sets the
/// handle and inserts into the map in one call. The trait-object insert is
/// private to this module, so no caller can add one of them to the map and
/// leave its handle unset. Apart from those registrars, the only callers that
/// reach it — and so the only ones that insert without a handle — are
/// `register_webhook`, whose adapter has no concrete-only methods and hence no
/// handle, and the tests.
#[derive(Clone)]
pub struct Adapters {
    providers: HashMap<AdapterKind, Arc<dyn Adapter>>,
    identities: HashMap<AdapterKind, Arc<dyn AdapterIdentity>>,
    user_identities: HashMap<AdapterKind, Arc<dyn AdapterUserIdentity>>,
    destination_builders: HashMap<AdapterKind, Arc<dyn AdapterDestinationBuilder>>,
    binding_replies: HashMap<AdapterKind, Arc<dyn AdapterBindingReply>>,
    message_replies: HashMap<AdapterKind, Arc<dyn AdapterMessageReply>>,
    direct_destinations: HashMap<AdapterKind, Arc<dyn AdapterDirectDestination>>,
    direct_messages: HashMap<AdapterKind, Arc<dyn AdapterDirectMessage>>,
    new_threads: HashMap<AdapterKind, Arc<dyn AdapterNewThread>>,
    post_binding_followups: HashMap<AdapterKind, Arc<dyn AdapterPostBindingFollowup>>,
    reactions: HashMap<AdapterKind, Arc<dyn AdapterReactions>>,
    inbound_presentations: HashMap<AdapterKind, Arc<dyn AdapterInboundPresentation>>,
    binding_statuses: HashMap<AdapterKind, Arc<dyn AdapterBindingStatus>>,
    github_pr: Option<Arc<super::github_pr::adapter::GithubPrAdapter>>,
    linear: Option<Arc<super::linear::adapter::LinearAdapter>>,
    slack: Option<Arc<super::slack::adapter::SlackAdapter>>,
    telegram: Option<Arc<super::telegram::adapter::TelegramAdapter>>,
}

impl Default for Adapters {
    fn default() -> Self {
        Self::new()
    }
}

impl Adapters {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
            identities: HashMap::new(),
            user_identities: HashMap::new(),
            destination_builders: HashMap::new(),
            binding_replies: HashMap::new(),
            message_replies: HashMap::new(),
            direct_destinations: HashMap::new(),
            direct_messages: HashMap::new(),
            new_threads: HashMap::new(),
            post_binding_followups: HashMap::new(),
            reactions: HashMap::new(),
            inbound_presentations: HashMap::new(),
            binding_statuses: HashMap::new(),
            github_pr: None,
            linear: None,
            slack: None,
            telegram: None,
        }
    }

    /// Insert into the trait-object map only.
    ///
    /// Private: every adapter kind that has a typed handle must go through its
    /// typed registrar, or the map and the handle could disagree. Widen this to
    /// `pub(crate)` if a stand-in registry is ever needed from another module.
    fn register_trait_object(&mut self, provider: Arc<dyn Adapter>) {
        self.providers.insert(provider.adapter_kind(), provider);
    }

    fn register_identity(&mut self, provider: Arc<dyn AdapterIdentity>) {
        self.identities.insert(provider.adapter_kind(), provider);
    }

    fn register_user_identity(
        &mut self,
        kind: AdapterKind,
        provider: Arc<dyn AdapterUserIdentity>,
    ) {
        self.user_identities.insert(kind, provider);
    }

    fn register_destination_builder(
        &mut self,
        kind: AdapterKind,
        provider: Arc<dyn AdapterDestinationBuilder>,
    ) {
        self.destination_builders.insert(kind, provider);
    }

    fn register_binding_reply(&mut self, provider: Arc<dyn AdapterBindingReply>) {
        self.binding_replies
            .insert(provider.adapter_kind(), provider);
    }

    fn register_message_reply(&mut self, provider: Arc<dyn AdapterMessageReply>) {
        self.message_replies
            .insert(provider.adapter_kind(), provider);
    }

    fn register_direct_destination(&mut self, provider: Arc<dyn AdapterDirectDestination>) {
        self.direct_destinations
            .insert(provider.adapter_kind(), provider);
    }

    fn register_direct_message(&mut self, provider: Arc<dyn AdapterDirectMessage>) {
        self.direct_messages
            .insert(provider.adapter_kind(), provider);
    }

    fn register_new_thread(&mut self, provider: Arc<dyn AdapterNewThread>) {
        self.new_threads.insert(provider.adapter_kind(), provider);
    }

    fn register_post_binding_followup(&mut self, provider: Arc<dyn AdapterPostBindingFollowup>) {
        self.post_binding_followups
            .insert(provider.adapter_kind(), provider);
    }

    fn register_reactions(&mut self, provider: Arc<dyn AdapterReactions>) {
        self.reactions.insert(provider.adapter_kind(), provider);
    }

    fn register_inbound_presentation(
        &mut self,
        kind: AdapterKind,
        provider: Arc<dyn AdapterInboundPresentation>,
    ) {
        self.inbound_presentations.insert(kind, provider);
    }

    fn register_binding_status(
        &mut self,
        kind: AdapterKind,
        provider: Arc<dyn AdapterBindingStatus>,
    ) {
        self.binding_statuses.insert(kind, provider);
    }

    /// The Webhook adapter has no concrete-only methods, so it has no typed
    /// handle and lives in the map alone.
    pub fn register_webhook(&mut self, provider: Arc<super::webhook::adapter::WebhookAdapter>) {
        let kind = AdapterKind::Webhook;
        self.register_trait_object(provider.clone());
        self.register_identity(provider.clone());
        self.register_user_identity(kind, provider.clone());
        self.register_binding_reply(provider.clone());
        self.register_message_reply(provider);
    }

    pub(crate) fn register_github_pr(
        &mut self,
        provider: Arc<super::github_pr::adapter::GithubPrAdapter>,
    ) {
        let kind = AdapterKind::GithubPr;
        self.register_trait_object(provider.clone());
        self.register_identity(provider.clone());
        self.register_user_identity(kind, provider.clone());
        self.register_destination_builder(kind, provider.clone());
        self.register_binding_reply(provider.clone());
        self.register_message_reply(provider.clone());
        self.register_reactions(provider.clone());
        self.register_inbound_presentation(kind, provider.clone());
        self.register_binding_status(kind, provider.clone());
        self.github_pr = Some(provider);
    }

    pub fn register_linear(&mut self, provider: Arc<super::linear::adapter::LinearAdapter>) {
        let kind = AdapterKind::Linear;
        self.register_trait_object(provider.clone());
        self.register_identity(provider.clone());
        self.register_user_identity(kind, provider.clone());
        self.register_binding_reply(provider.clone());
        self.register_message_reply(provider.clone());
        self.register_direct_destination(provider.clone());
        self.register_post_binding_followup(provider.clone());
        self.register_inbound_presentation(kind, provider.clone());
        self.register_binding_status(kind, provider.clone());
        self.linear = Some(provider);
    }

    pub(crate) fn register_slack(&mut self, provider: Arc<super::slack::adapter::SlackAdapter>) {
        let kind = AdapterKind::Slack;
        self.register_trait_object(provider.clone());
        self.register_identity(provider.clone());
        self.register_user_identity(kind, provider.clone());
        self.register_destination_builder(kind, provider.clone());
        self.register_binding_reply(provider.clone());
        self.register_message_reply(provider.clone());
        self.register_direct_destination(provider.clone());
        self.register_direct_message(provider.clone());
        self.register_new_thread(provider.clone());
        self.register_reactions(provider.clone());
        self.register_inbound_presentation(kind, provider.clone());
        self.slack = Some(provider);
    }

    pub(crate) fn register_telegram(
        &mut self,
        provider: Arc<super::telegram::adapter::TelegramAdapter>,
    ) {
        let kind = AdapterKind::Telegram;
        self.register_trait_object(provider.clone());
        self.register_identity(provider.clone());
        self.register_user_identity(kind, provider.clone());
        self.register_destination_builder(kind, provider.clone());
        self.register_binding_reply(provider.clone());
        self.register_message_reply(provider.clone());
        self.register_new_thread(provider.clone());
        self.register_reactions(provider.clone());
        self.register_inbound_presentation(kind, provider.clone());
        self.telegram = Some(provider);
    }

    /// Typed handle to the GitHub PR adapter, for code inside its own module.
    pub(crate) fn github_pr(&self) -> Option<&super::github_pr::adapter::GithubPrAdapter> {
        self.github_pr.as_deref()
    }

    /// Typed handle to the Linear adapter, for code inside its own module.
    pub fn linear(&self) -> Option<&super::linear::adapter::LinearAdapter> {
        self.linear.as_deref()
    }

    /// Typed handle to the Slack adapter, for code inside its own module.
    pub fn slack(&self) -> Option<&super::slack::adapter::SlackAdapter> {
        self.slack.as_deref()
    }

    /// Typed handle to the Telegram adapter, for code inside its own module.
    pub(crate) fn telegram(&self) -> Option<&super::telegram::adapter::TelegramAdapter> {
        self.telegram.as_deref()
    }

    pub fn get(&self, adapter_kind: AdapterKind) -> Option<&Arc<dyn Adapter>> {
        self.providers.get(&adapter_kind)
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "identity capability accessor is exercised by optional adapter workflows"
        )
    )]
    pub(crate) fn identity(
        &self,
        adapter_kind: AdapterKind,
    ) -> Result<&dyn AdapterIdentity, AdapterError> {
        self.identities
            .get(&adapter_kind)
            .map(Arc::as_ref)
            .ok_or_else(|| unsupported_adapter(adapter_kind))
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "user identity capability accessor is exercised by optional adapter workflows"
        )
    )]
    pub(crate) fn user_identity(
        &self,
        adapter_kind: AdapterKind,
    ) -> Result<&dyn AdapterUserIdentity, AdapterError> {
        self.user_identities
            .get(&adapter_kind)
            .map(Arc::as_ref)
            .ok_or_else(|| unsupported_user_identity(adapter_kind))
    }

    pub(crate) fn destination_builder(
        &self,
        adapter_kind: AdapterKind,
    ) -> Result<&dyn AdapterDestinationBuilder, AdapterError> {
        self.destination_builders
            .get(&adapter_kind)
            .map(Arc::as_ref)
            .ok_or_else(|| unsupported_destination_builder(adapter_kind))
    }

    pub(crate) fn outbound_for_binding_reply_by_name(
        &self,
        name: &str,
    ) -> Result<&dyn AdapterBindingReply, AdapterError> {
        let adapter_kind = self
            .adapter_kind_for_name(name)
            .ok_or_else(|| AdapterError::BadRequest(format!("Unknown adapter `{name}`")))?;
        self.binding_replies
            .get(&adapter_kind)
            .map(Arc::as_ref)
            .ok_or_else(|| unsupported_outbound(adapter_kind, OutboundOperation::BindingReply))
    }

    pub(crate) fn outbound_for_message_reply_by_name(
        &self,
        name: &str,
    ) -> Result<&dyn AdapterMessageReply, AdapterError> {
        let adapter_kind = self
            .adapter_kind_for_name(name)
            .ok_or_else(|| AdapterError::BadRequest(format!("Unknown adapter `{name}`")))?;
        self.message_replies
            .get(&adapter_kind)
            .map(Arc::as_ref)
            .ok_or_else(|| unsupported_outbound(adapter_kind, OutboundOperation::MessageReply))
    }

    pub(crate) fn outbound_for_new_thread_by_name(
        &self,
        name: &str,
    ) -> Result<&dyn AdapterNewThread, AdapterError> {
        let adapter_kind = self
            .adapter_kind_for_name(name)
            .ok_or_else(|| AdapterError::BadRequest(format!("Unknown adapter `{name}`")))?;
        self.new_threads
            .get(&adapter_kind)
            .map(Arc::as_ref)
            .ok_or_else(|| unsupported_outbound(adapter_kind, OutboundOperation::NewThread))
    }

    pub(crate) fn outbound_for_direct_destination(
        &self,
        adapter_kind: AdapterKind,
    ) -> Result<&dyn AdapterDirectDestination, AdapterError> {
        self.direct_destinations
            .get(&adapter_kind)
            .map(Arc::as_ref)
            .ok_or_else(|| unsupported_outbound(adapter_kind, OutboundOperation::DirectDestination))
    }

    pub(crate) fn outbound_for_direct_message(
        &self,
        adapter_kind: AdapterKind,
    ) -> Result<&dyn AdapterDirectMessage, AdapterError> {
        self.direct_messages
            .get(&adapter_kind)
            .map(Arc::as_ref)
            .ok_or_else(|| unsupported_outbound(adapter_kind, OutboundOperation::DirectMessage))
    }

    pub(crate) fn post_binding_followup(
        &self,
        adapter_kind: AdapterKind,
    ) -> Option<&dyn AdapterPostBindingFollowup> {
        self.post_binding_followups
            .get(&adapter_kind)
            .map(Arc::as_ref)
    }

    pub(crate) fn reactions(
        &self,
        adapter_kind: AdapterKind,
    ) -> Result<&dyn AdapterReactions, AdapterError> {
        self.reactions
            .get(&adapter_kind)
            .map(Arc::as_ref)
            .ok_or_else(|| unsupported_reactions(adapter_kind))
    }

    pub(crate) fn reactions_by_name(
        &self,
        name: &str,
    ) -> Result<&dyn AdapterReactions, AdapterError> {
        let adapter_kind = self
            .adapter_kind_for_name(name)
            .ok_or_else(|| AdapterError::BadRequest(format!("Unknown adapter `{name}`")))?;
        self.reactions(adapter_kind)
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "inbound presentation accessor is exercised by optional adapter workflows"
        )
    )]
    pub(crate) fn inbound_presentation(
        &self,
        adapter_kind: AdapterKind,
    ) -> Result<&dyn AdapterInboundPresentation, AdapterError> {
        self.inbound_presentations
            .get(&adapter_kind)
            .map(Arc::as_ref)
            .ok_or_else(|| unsupported_inbound_presentation(adapter_kind))
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "binding status accessor is exercised by optional adapter workflows"
        )
    )]
    pub(crate) fn binding_status(
        &self,
        adapter_kind: AdapterKind,
    ) -> Option<&dyn AdapterBindingStatus> {
        self.binding_statuses.get(&adapter_kind).map(Arc::as_ref)
    }

    pub fn get_by_name(&self, name: &str) -> Option<&Arc<dyn Adapter>> {
        self.adapter_kind_for_name(name)
            .and_then(|kind| self.providers.get(&kind))
    }

    pub fn provider_key_for_name(&self, name: &str) -> Option<&'static str> {
        self.identity_by_name(name)
            .map(AdapterIdentity::provider_key)
    }

    pub(crate) fn identity_by_name(&self, name: &str) -> Option<&dyn AdapterIdentity> {
        self.adapter_kind_for_name(name)
            .and_then(|kind| self.identities.get(&kind).map(Arc::as_ref))
    }

    fn adapter_kind_for_name(&self, name: &str) -> Option<AdapterKind> {
        if let Some(kind) =
            AdapterKind::parse(name).filter(|kind| self.identities.contains_key(kind))
        {
            return Some(kind);
        }
        self.identities.iter().find_map(|(kind, provider)| {
            (provider.provider_key() == name || provider.connection_provider_key() == name)
                .then_some(*kind)
        })
    }

    pub(crate) fn inbound_presentation_by_name(
        &self,
        name: &str,
    ) -> Option<&dyn AdapterInboundPresentation> {
        self.adapter_kind_for_name(name)
            .and_then(|kind| self.inbound_presentations.get(&kind))
            .map(Arc::as_ref)
    }

    pub(crate) fn binding_status_by_name(&self, name: &str) -> Option<&dyn AdapterBindingStatus> {
        self.adapter_kind_for_name(name)
            .and_then(|kind| self.binding_statuses.get(&kind))
            .map(Arc::as_ref)
    }

    /// Returns configured adapter kinds.
    pub fn registered_types(&self) -> Vec<AdapterKind> {
        self.providers.keys().copied().collect()
    }
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "paired with the optional identity capability accessor"
    )
)]
fn unsupported_adapter(adapter_kind: AdapterKind) -> AdapterError {
    AdapterError::BadRequest(format!("No adapter registered for {adapter_kind:?}"))
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "paired with the optional user identity capability accessor"
    )
)]
fn unsupported_user_identity(adapter_kind: AdapterKind) -> AdapterError {
    AdapterError::BadRequest(format!(
        "{} Adapter does not support user identity lookup",
        adapter_kind.as_str()
    ))
}

fn unsupported_destination_builder(adapter_kind: AdapterKind) -> AdapterError {
    AdapterError::BadRequest(format!(
        "{} provider does not support task bindings",
        adapter_kind.as_str()
    ))
}

#[derive(Copy, Clone)]
enum OutboundOperation {
    BindingReply,
    MessageReply,
    DirectDestination,
    DirectMessage,
    NewThread,
}

fn unsupported_outbound(adapter_kind: AdapterKind, operation: OutboundOperation) -> AdapterError {
    match operation {
        OutboundOperation::BindingReply | OutboundOperation::MessageReply => {
            AdapterError::BadRequest(format!(
                "{} provider does not support outbound replies",
                adapter_kind.as_str()
            ))
        }
        OutboundOperation::DirectDestination => AdapterError::BadRequest(format!(
            "{adapter_kind:?} provider does not support direct destination sends"
        )),
        OutboundOperation::DirectMessage => AdapterError::BadRequest(format!(
            "User DM is not supported for adapter {}",
            adapter_kind.as_str()
        )),
        OutboundOperation::NewThread => {
            AdapterError::Internal(format!("{adapter_kind:?} does not support send_message"))
        }
    }
}

fn unsupported_reactions(adapter_kind: AdapterKind) -> AdapterError {
    AdapterError::BadRequest(format!(
        "Provider {} does not support reactions",
        adapter_kind.as_str()
    ))
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "paired with the optional inbound presentation capability accessor"
    )
)]
fn unsupported_inbound_presentation(adapter_kind: AdapterKind) -> AdapterError {
    AdapterError::BadRequest(format!(
        "{} Adapter does not support inbound presentation",
        adapter_kind.as_str()
    ))
}

#[cfg(test)]
mod tests;
