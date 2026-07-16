//! Agent availability lookup — metadata-only credential presence check.

use std::collections::HashSet;

use super::catalog::{catalog, AgentEntry};
use super::schemas::{AgentDescriptor, AgentModel};
use crate::state::AppState;
use moatless_backend_core::scope::ScopeContext;

/// Materialize the full catalog with `installed` populated per the given scope.
pub async fn list_agents(state: &AppState, ctx: &ScopeContext) -> Vec<AgentDescriptor> {
    let present_keys = collect_credential_keys(state, ctx).await;
    let present_providers = collect_provider_token_providers(state, ctx).await;

    catalog()
        .iter()
        .map(|entry| build_descriptor(entry, &present_keys, &present_providers))
        .collect()
}

fn build_descriptor(
    entry: &AgentEntry,
    present_keys: &HashSet<String>,
    present_providers: &HashSet<String>,
) -> AgentDescriptor {
    let installed = entry
        .credential_keys
        .iter()
        .any(|k| present_keys.contains(*k))
        || entry
            .provider_token_providers
            .iter()
            .any(|p| present_providers.contains(*p));

    AgentDescriptor {
        id: entry.id.to_string(),
        label: entry.label.to_string(),
        installed,
        default_model: entry.default_model.to_string(),
        models: entry
            .models
            .iter()
            .map(|m| AgentModel {
                id: m.id.to_string(),
                label: m.label.to_string(),
            })
            .collect(),
        missing_credential_hint: if installed {
            None
        } else {
            Some(entry.missing_credential_hint.to_string())
        },
    }
}

/// Collect the set of credential keys visible to the caller across:
///   - Global / Team / User scoped secrets (metadata only).
async fn collect_credential_keys(state: &AppState, ctx: &ScopeContext) -> HashSet<String> {
    let mut keys: HashSet<String> = HashSet::new();

    match state.secret_records.list_by_scope("global", None).await {
        Ok(records) => keys.extend(records.into_iter().map(|r| r.key)),
        Err(e) => tracing::warn!(error = %e, "agent service: failed to list global secrets"),
    }

    for team_id in &ctx.team_ids {
        match state
            .secret_records
            .list_by_scope("team", Some(team_id))
            .await
        {
            Ok(records) => keys.extend(records.into_iter().map(|r| r.key)),
            Err(e) => {
                tracing::warn!(error = %e, team_id, "agent service: failed to list team secrets");
            }
        }
    }

    match state
        .secret_records
        .list_by_scope("user", Some(&ctx.user_id))
        .await
    {
        Ok(records) => keys.extend(records.into_iter().map(|r| r.key)),
        Err(e) => {
            tracing::warn!(
                error = %e,
                user_id = %ctx.user_id,
                "agent service: failed to list user secrets"
            );
        }
    }

    keys
}

/// Collect the set of provider-token provider names visible to the caller.
async fn collect_provider_token_providers(state: &AppState, ctx: &ScopeContext) -> HashSet<String> {
    let mut providers: HashSet<String> = HashSet::new();

    let relevant: HashSet<&'static str> = catalog()
        .iter()
        .flat_map(|e| e.provider_token_providers.iter().copied())
        .collect();

    for provider in &relevant {
        match state
            .provider_tokens
            .find_provider_token(&ctx.user_id, provider)
            .await
        {
            Ok(Some(_)) => {
                providers.insert((*provider).to_string());
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    user_id = %ctx.user_id,
                    provider,
                    "agent service: failed to query user provider_token"
                );
            }
        }
    }

    for team_id in &ctx.team_ids {
        for provider in &relevant {
            if providers.contains(*provider) {
                continue;
            }
            match state
                .provider_tokens
                .find_team_token(team_id, provider)
                .await
            {
                Ok(Some(_)) => {
                    providers.insert((*provider).to_string());
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        team_id,
                        provider,
                        "agent service: failed to query team provider_token"
                    );
                }
            }
        }
    }

    providers
}
