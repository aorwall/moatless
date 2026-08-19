use std::sync::Arc;

use super::super::capabilities::Adapter;
use super::Adapters;
use crate::adapters::error::{AdapterError, SharedAdapterError};
use moatless_types::adapter::AdapterKind;

// The capability-matrix tests that build the five concrete providers live in the
// backend crate (`backend/src/adapters/registry_matrix_tests.rs`): the providers
// sit above the loops band, so a loops test cannot name them. They reach the
// registry's `pub(crate)` accessors through `adapters::test_support::registry`.
// What stays here is the pair that needs no provider — the empty/stand-in
// registry and the registry-owned "missing outbound capability" error — proven
// with in-crate stub adapters alone.

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
