//! Adapter registry and application configuration wiring.

use std::collections::HashMap;
use std::sync::Arc;

use crate::capabilities::{
    Adapter, AdapterBindingReply, AdapterBindingStatus, AdapterDirectDestination,
    AdapterDirectMessage, AdapterInboundPresentation, AdapterMessageReply,
    AdapterPostBindingFollowup, AdapterReactions,
};
use crate::error::{AdapterError, SharedAdapterError};

use moatless_types::adapter::AdapterKind;

/// Registry of external adapters keyed by `AdapterKind`.
///
/// The trait-object map is the whole registry: cross-provider code dispatches
/// through it, and each provider's concrete handle lives on the composition root
/// (`ProviderHandles`), not here.
///
/// Each provider registers into it through its own `register_into` method,
/// called once at the composition root: construction lives there, and the
/// provider owns the mapping from itself to the capability surfaces it
/// implements.
#[derive(Clone)]
pub struct Adapters {
    providers: HashMap<AdapterKind, Arc<dyn Adapter>>,
    binding_replies: HashMap<AdapterKind, Arc<dyn AdapterBindingReply>>,
    message_replies: HashMap<AdapterKind, Arc<dyn AdapterMessageReply>>,
    direct_destinations: HashMap<AdapterKind, Arc<dyn AdapterDirectDestination>>,
    direct_messages: HashMap<AdapterKind, Arc<dyn AdapterDirectMessage>>,
    post_binding_followups: HashMap<AdapterKind, Arc<dyn AdapterPostBindingFollowup>>,
    reactions: HashMap<AdapterKind, Arc<dyn AdapterReactions>>,
    inbound_presentations: HashMap<AdapterKind, Arc<dyn AdapterInboundPresentation>>,
    binding_statuses: HashMap<AdapterKind, Arc<dyn AdapterBindingStatus>>,
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
            binding_replies: HashMap::new(),
            message_replies: HashMap::new(),
            direct_destinations: HashMap::new(),
            direct_messages: HashMap::new(),
            post_binding_followups: HashMap::new(),
            reactions: HashMap::new(),
            inbound_presentations: HashMap::new(),
            binding_statuses: HashMap::new(),
        }
    }

    /// Insert into the trait-object map only.
    ///
    /// `pub(crate)` so each provider's `register_into` can fan its one concrete
    /// handle across the capability maps without the registry naming the
    /// provider type. Every adapter kind that has a typed handle must go through
    /// its own `register_into`, or the map and the handle could disagree.
    pub fn register_trait_object(&mut self, provider: Arc<dyn Adapter>) {
        self.providers.insert(provider.adapter_kind(), provider);
    }

    pub fn register_binding_reply(&mut self, provider: Arc<dyn AdapterBindingReply>) {
        self.binding_replies
            .insert(provider.adapter_kind(), provider);
    }

    pub fn register_message_reply(&mut self, provider: Arc<dyn AdapterMessageReply>) {
        self.message_replies
            .insert(provider.adapter_kind(), provider);
    }

    pub fn register_direct_destination(&mut self, provider: Arc<dyn AdapterDirectDestination>) {
        self.direct_destinations
            .insert(provider.adapter_kind(), provider);
    }

    pub fn register_direct_message(&mut self, provider: Arc<dyn AdapterDirectMessage>) {
        self.direct_messages
            .insert(provider.adapter_kind(), provider);
    }

    pub fn register_post_binding_followup(
        &mut self,
        provider: Arc<dyn AdapterPostBindingFollowup>,
    ) {
        self.post_binding_followups
            .insert(provider.adapter_kind(), provider);
    }

    pub fn register_reactions(&mut self, provider: Arc<dyn AdapterReactions>) {
        self.reactions.insert(provider.adapter_kind(), provider);
    }

    pub fn register_inbound_presentation(
        &mut self,
        kind: AdapterKind,
        provider: Arc<dyn AdapterInboundPresentation>,
    ) {
        self.inbound_presentations.insert(kind, provider);
    }

    pub fn register_binding_status(
        &mut self,
        kind: AdapterKind,
        provider: Arc<dyn AdapterBindingStatus>,
    ) {
        self.binding_statuses.insert(kind, provider);
    }

    pub fn get(&self, adapter_kind: AdapterKind) -> Option<&Arc<dyn Adapter>> {
        self.providers.get(&adapter_kind)
    }

    /// Public because the loops band's outbound path resolves a reply provider
    /// by name across the crate boundary.
    pub fn outbound_for_binding_reply_by_name(
        &self,
        name: &str,
    ) -> Result<&dyn AdapterBindingReply, AdapterError> {
        let adapter_kind = self
            .adapter_kind_for_name(name)
            .ok_or_else(|| SharedAdapterError::BadRequest(format!("Unknown adapter `{name}`")))?;
        self.binding_replies
            .get(&adapter_kind)
            .map(Arc::as_ref)
            .ok_or_else(|| unsupported_outbound(adapter_kind, OutboundOperation::BindingReply))
    }

    pub fn outbound_for_message_reply_by_name(
        &self,
        name: &str,
    ) -> Result<&dyn AdapterMessageReply, AdapterError> {
        let adapter_kind = self
            .adapter_kind_for_name(name)
            .ok_or_else(|| SharedAdapterError::BadRequest(format!("Unknown adapter `{name}`")))?;
        self.message_replies
            .get(&adapter_kind)
            .map(Arc::as_ref)
            .ok_or_else(|| unsupported_outbound(adapter_kind, OutboundOperation::MessageReply))
    }

    pub fn outbound_for_direct_destination(
        &self,
        adapter_kind: AdapterKind,
    ) -> Result<&dyn AdapterDirectDestination, AdapterError> {
        self.direct_destinations
            .get(&adapter_kind)
            .map(Arc::as_ref)
            .ok_or_else(|| unsupported_outbound(adapter_kind, OutboundOperation::DirectDestination))
    }

    pub fn outbound_for_direct_message(
        &self,
        adapter_kind: AdapterKind,
    ) -> Result<&dyn AdapterDirectMessage, AdapterError> {
        self.direct_messages
            .get(&adapter_kind)
            .map(Arc::as_ref)
            .ok_or_else(|| unsupported_outbound(adapter_kind, OutboundOperation::DirectMessage))
    }

    pub fn post_binding_followup(
        &self,
        adapter_kind: AdapterKind,
    ) -> Option<&dyn AdapterPostBindingFollowup> {
        self.post_binding_followups
            .get(&adapter_kind)
            .map(Arc::as_ref)
    }

    pub fn reactions(
        &self,
        adapter_kind: AdapterKind,
    ) -> Result<&dyn AdapterReactions, AdapterError> {
        self.reactions
            .get(&adapter_kind)
            .map(Arc::as_ref)
            .ok_or_else(|| unsupported_reactions(adapter_kind))
    }

    pub fn reactions_by_name(
        &self,
        name: &str,
    ) -> Result<&dyn AdapterReactions, AdapterError> {
        let adapter_kind = self
            .adapter_kind_for_name(name)
            .ok_or_else(|| SharedAdapterError::BadRequest(format!("Unknown adapter `{name}`")))?;
        self.reactions(adapter_kind)
    }

    pub fn get_by_name(&self, name: &str) -> Option<&Arc<dyn Adapter>> {
        self.adapter_kind_for_name(name)
            .and_then(|kind| self.providers.get(&kind))
    }

    pub fn provider_key_for_name(&self, name: &str) -> Option<&'static str> {
        self.identity_by_name(name).map(Adapter::provider_key)
    }

    pub fn identity_by_name(&self, name: &str) -> Option<&dyn Adapter> {
        self.adapter_kind_for_name(name)
            .and_then(|kind| self.providers.get(&kind).map(Arc::as_ref))
    }

    fn adapter_kind_for_name(&self, name: &str) -> Option<AdapterKind> {
        if let Some(kind) =
            AdapterKind::parse(name).filter(|kind| self.providers.contains_key(kind))
        {
            return Some(kind);
        }
        self.providers.iter().find_map(|(kind, provider)| {
            (provider.provider_key() == name || provider.connection_provider_key() == name)
                .then_some(*kind)
        })
    }

    pub fn inbound_presentation_by_name(
        &self,
        name: &str,
    ) -> Option<&dyn AdapterInboundPresentation> {
        self.adapter_kind_for_name(name)
            .and_then(|kind| self.inbound_presentations.get(&kind))
            .map(Arc::as_ref)
    }

    pub fn binding_status_by_name(&self, name: &str) -> Option<&dyn AdapterBindingStatus> {
        self.adapter_kind_for_name(name)
            .and_then(|kind| self.binding_statuses.get(&kind))
            .map(Arc::as_ref)
    }

    pub fn registered_types(&self) -> Vec<AdapterKind> {
        self.providers.keys().copied().collect()
    }
}

#[derive(Copy, Clone)]
enum OutboundOperation {
    BindingReply,
    MessageReply,
    DirectDestination,
    DirectMessage,
}

fn unsupported_outbound(adapter_kind: AdapterKind, operation: OutboundOperation) -> AdapterError {
    match operation {
        OutboundOperation::BindingReply | OutboundOperation::MessageReply => {
            SharedAdapterError::BadRequest(format!(
                "{} provider does not support outbound replies",
                adapter_kind.as_str()
            ))
            .into()
        }
        OutboundOperation::DirectDestination => SharedAdapterError::BadRequest(format!(
            "{adapter_kind:?} provider does not support direct destination sends"
        ))
        .into(),
        OutboundOperation::DirectMessage => SharedAdapterError::BadRequest(format!(
            "User DM is not supported for adapter {}",
            adapter_kind.as_str()
        ))
        .into(),
    }
}

fn unsupported_reactions(adapter_kind: AdapterKind) -> AdapterError {
    SharedAdapterError::BadRequest(format!(
        "Provider {} does not support reactions",
        adapter_kind.as_str()
    ))
    .into()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::Adapters;
    use crate::capabilities::Adapter;
    use crate::error::{AdapterError, SharedAdapterError};
    use moatless_types::adapter::AdapterKind;

    // The capability-matrix tests that build the five concrete providers live in
    // the backend crate (`backend/src/adapters/registry_matrix_tests.rs`): the
    // providers sit above the loops band, so a loops test cannot name them. They
    // call these accessors directly.
    // What stays here is the pair that needs no provider — the empty/stand-in
    // registry and the registry-owned "missing outbound capability" error —
    // proven with in-crate stub adapters alone.

    struct DummyProvider;

    struct IdentityOnlyProvider {
        kind: AdapterKind,
    }

    impl Adapter for IdentityOnlyProvider {
        fn adapter_kind(&self) -> AdapterKind {
            self.kind
        }
    }

    impl Adapter for DummyProvider {
        fn adapter_kind(&self) -> AdapterKind {
            AdapterKind::Telegram
        }
    }

    #[test]
    fn empty_registry_takes_a_stand_in_adapter() {
        let mut providers = Adapters::new();
        providers.register_trait_object(Arc::new(DummyProvider));

        assert_eq!(providers.registered_types(), vec![AdapterKind::Telegram]);
        assert!(providers.get(AdapterKind::Telegram).is_some());
    }

    #[tokio::test]
    async fn missing_outbound_capability_returns_registry_owned_error() {
        let mut providers = Adapters::new();
        let kind = AdapterKind::Slack;
        providers.register_trait_object(Arc::new(IdentityOnlyProvider { kind }));

        match providers.outbound_for_direct_destination(kind) {
            Err(AdapterError::Shared(SharedAdapterError::BadRequest(msg))) => {
                assert!(
                    msg.contains("does not support direct destination sends"),
                    "unexpected message: {msg}"
                );
            }
            Err(other) => panic!("expected BadRequest, got: {other:?}"),
            Ok(_) => panic!("identity-only adapter should not have outbound capability"),
        }
    }
}
