//! The Loop operations a host drives: mutation policy, state transitions, and
//! the access-filtered reads the HTTP surface projects.

use std::sync::Arc;

use super::access::ensure_workspace_usable;
use super::dao::LoopDao;
use super::models::{
    AdapterSourceUpdate, LoopListFilter, LoopRecord, LoopSourceRow, LoopUpdate, ManagedLoopUpsert,
    NewAdapterEventSource, NewLoop, NewLoopConfig, NewLoopSource, NewScheduleSource,
    UpdateLoopSource as StoreUpdateSource,
};
use super::schedule::next_fire_at;
use super::sync::LoopSyncService;
use chrono::Utc;
use moatless_adapter_model::access::resolve_scope_for_adapter_create;
use moatless_adapter_model::connections::AdapterConnections;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_api_schemas::loops::{
    CreateLoopRequest, UpdateLoopConfig, UpdateLoopRequest, UpdateLoopSource,
};
use moatless_ports::identity::UserResolver;
use moatless_ports::workspace::{RepositoryReader, WorkspaceResolver};
use moatless_tasks::agent::catalog;
use moatless_types::access_control::ScopeContext;
use moatless_types::loops::LoopKind as DomainLoopKind;

fn require_admin(ctx: &ScopeContext) -> Result<(), AdapterError> {
    if ctx.is_admin {
        Ok(())
    } else {
        Err(
            SharedAdapterError::AccessDenied("Only administrators can modify Loops".to_string())
                .into(),
        )
    }
}

fn require_agent_and_model(agent_type: &str, model: Option<&str>) -> Result<(), AdapterError> {
    catalog::validate_agent_model(agent_type, model)
        .map_err(|e| SharedAdapterError::BadRequest(e).into())
}

fn require_name(name: &str) -> Result<String, AdapterError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(SharedAdapterError::BadRequest("A Loop needs a name".to_string()).into());
    }
    Ok(trimmed.to_string())
}

async fn resolve_run_as_user(
    users: &dyn UserResolver,
    requested: &str,
) -> Result<String, AdapterError> {
    let canonical = users
        .canonical_id(requested)
        .await
        .map_err(|e| SharedAdapterError::Internal(e.to_string()))?;
    canonical.map(|id| id.to_string()).ok_or_else(|| {
        SharedAdapterError::BadRequest(format!("Run-as user {requested} does not exist")).into()
    })
}

/// The Loop operations a host drives, over the private Loop store.
///
/// Built by [`crate::compose::LoopGraph::loops`].
pub struct Loops {
    store: Arc<LoopDao>,
}

impl Loops {
    pub(crate) fn new(store: Arc<LoopDao>) -> Self {
        Self { store }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn store(&self) -> &LoopDao {
        &self.store
    }

    /// The Loops an access-filtered caller may see.
    pub(crate) async fn list(
        &self,
        ctx: &ScopeContext,
        filter: LoopListFilter<'_>,
    ) -> Result<Vec<LoopRecord>, sqlx::Error> {
        self.store.list(ctx, filter).await
    }

    /// One Loop, or `None` when the caller's scope does not reach it.
    pub(crate) async fn get(
        &self,
        ctx: &ScopeContext,
        id: &str,
    ) -> Result<Option<LoopRecord>, sqlx::Error> {
        self.store.get(id, ctx).await
    }

    /// One Loop the caller's scope reaches, or `NotFound`.
    pub(crate) async fn load_for_auth(
        &self,
        ctx: &ScopeContext,
        id: &str,
    ) -> Result<LoopRecord, AdapterError> {
        self.get(ctx, id)
            .await?
            .ok_or_else(|| SharedAdapterError::NotFound(format!("Loop {id} not found")).into())
    }

    /// The Loops a user created, for an owner filter another domain applies.
    pub async fn ids_created_by(&self, user_id: &str) -> Result<Vec<String>, sqlx::Error> {
        self.store.ids_created_by(user_id).await
    }

    /// A Loop's adapter-specific configuration blob.
    pub async fn adapter_config(
        &self,
        loop_id: &str,
    ) -> Result<Option<serde_json::Value>, sqlx::Error> {
        self.store.get_adapter_config(loop_id).await
    }

    /// Replace a Loop's adapter-specific configuration blob.
    pub async fn set_adapter_config(
        &self,
        loop_id: &str,
        config: &serde_json::Value,
    ) -> Result<(), sqlx::Error> {
        self.store.set_adapter_config(loop_id, config).await
    }

    /// Restore one Git-owned Loop to its declaration, undoing an override.
    pub async fn reset_managed_loop(&self, input: &ManagedLoopUpsert) -> Result<bool, sqlx::Error> {
        self.store.reset_managed_loop(input).await
    }

    /// Create a Loop after validating source, scope, and execution approval.
    pub(crate) async fn create(
        &self,
        users: &dyn UserResolver,
        workspaces: &dyn WorkspaceResolver,
        adapter_connections: &AdapterConnections,
        ctx: &ScopeContext,
        req: CreateLoopRequest,
    ) -> Result<LoopRecord, AdapterError> {
        require_admin(ctx)?;
        let name = require_name(&req.name)?;
        let agent_type = req
            .config
            .agent_type
            .clone()
            .unwrap_or_else(|| "claude-code".to_string());
        require_agent_and_model(&agent_type, req.config.model.as_deref())?;
        ensure_workspace_usable(workspaces, ctx, &req.config.workspace_id).await?;
        let run_as_user_id = match req.config.run_as_user_id.as_deref() {
            Some(requested) => Some(resolve_run_as_user(users, requested).await?),
            None => None,
        };
        let active = req.active.unwrap_or(false);
        if active && run_as_user_id.is_none() {
            return Err(SharedAdapterError::BadRequest(
                "Activating a Loop requires an explicit run-as user".to_string(),
            )
            .into());
        }

        let kind = req.kind;
        let adapter_source = match (kind, &req.source) {
            (
                DomainLoopKind::AdapterEvent,
                moatless_api_schemas::loops::CreateLoopSource::AdapterEvent(source),
            ) => Some(source),
            _ => None,
        };
        let schedule_source = match (kind, &req.source) {
            (
                DomainLoopKind::Schedule,
                moatless_api_schemas::loops::CreateLoopSource::Schedule(source),
            ) => Some(source),
            _ => None,
        };
        if matches!(kind, DomainLoopKind::AdapterEvent) && adapter_source.is_none() {
            return Err(SharedAdapterError::BadRequest(
                "adapter_event loops require an adapter-event source".into(),
            )
            .into());
        }
        if matches!(kind, DomainLoopKind::Schedule) && schedule_source.is_none() {
            return Err(SharedAdapterError::BadRequest(
                "schedule loops require a schedule source".into(),
            )
            .into());
        }

        if let Some(source) = adapter_source {
            let adapter_kind = source.adapter_kind.as_str().to_string();
            let connection = adapter_connections
                .for_auth(ctx, &source.connection_id)
                .await?;
            if connection.adapter_kind.as_str() != adapter_kind {
                return Err(SharedAdapterError::BadRequest(format!(
                    "Connection {} is for adapter {}, not {}",
                    connection.id, connection.adapter_kind, adapter_kind
                ))
                .into());
            }
        }

        let next_fire = if let Some(source) = schedule_source {
            Some(
                next_fire_at(&source.cron_expression)
                    .map_err(|e| SharedAdapterError::BadRequest(e.to_string()))?,
            )
        } else {
            None
        };

        let resolved = resolve_scope_for_adapter_create(ctx, req.scope)?;
        let id = format!("tr_{}", uuid::Uuid::new_v4());
        let now = Utc::now();

        let source = if let Some(source) = adapter_source {
            NewLoopSource::AdapterEvent(NewAdapterEventSource {
                adapter_kind: source.adapter_kind.as_str().to_string(),
                connection_id: source.connection_id.clone(),
                source_matcher: source.source_matcher.clone(),
                source_name: source.source_name.clone(),
                event_filter: source
                    .event_filter
                    .clone()
                    .unwrap_or_else(|| serde_json::json!([])),
                adapter_config: source.adapter_config.clone(),
                allow_direct_send: source.allow_direct_send.unwrap_or(false),
                close_task_on_close: source.close_task_on_close.unwrap_or(true),
                notify_on_success: source.notify_on_success.unwrap_or(false),
            })
        } else if let Some(source) = schedule_source {
            let next_fire = next_fire.ok_or_else(|| {
                SharedAdapterError::Internal("missing schedule next fire".to_string())
            })?;
            NewLoopSource::Schedule(NewScheduleSource {
                cron_expression: source.cron_expression.clone(),
                timezone: source.timezone.clone().unwrap_or_else(|| "UTC".to_string()),
                message_template: source.message_template.clone(),
                next_fire_at: next_fire,
            })
        } else {
            NewLoopSource::Manual
        };

        self.store
            .create(NewLoop {
                id: id.clone(),
                name,
                kind: kind.as_str().to_string(),
                active,
                scope: resolved.scope,
                owner_user_id: resolved.owner_user_id,
                created_by: ctx.user_id.clone(),
                created_at: now,
                config: NewLoopConfig {
                    workspace_id: req.config.workspace_id.clone(),
                    routing_mode: req.config.routing_mode,
                    ongoing_task_id: req.config.ongoing_task_id.clone(),
                    prompt: req.config.prompt.clone(),
                    task_name: req.config.task_name.clone(),
                    tag_ids: req.config.tag_ids.clone().unwrap_or_default(),
                    skills: req.config.skills.clone().unwrap_or_default(),
                    run_as_user_id,
                    agent_type,
                    model: req.config.model.clone(),
                    inactive_ttl_days: req.config.inactive_ttl_days,
                },
                source,
            })
            .await?;

        let record = self
            .store
            .get(&id, ctx)
            .await?
            .ok_or_else(|| SharedAdapterError::NotFound(format!("Loop {id} not found")))?;
        Ok(record)
    }

    /// Apply administrator edits to a mutable Loop configuration.
    pub(crate) async fn update(
        &self,
        workspaces: &dyn WorkspaceResolver,
        adapter_connections: &AdapterConnections,
        ctx: &ScopeContext,
        id: &str,
        req: UpdateLoopRequest,
    ) -> Result<LoopRecord, AdapterError> {
        require_admin(ctx)?;
        let current = self.load_for_auth(ctx, id).await?;
        if current.loop_.config_source != moatless_types::loops::LoopConfigSource::Manual
            || current.loop_.deleted
        {
            return Err(SharedAdapterError::Conflict(
                "Git-synced Loops are read-only; override the Loop before editing it".to_string(),
            )
            .into());
        }

        let next_name = match req.name.as_deref() {
            Some(name) => require_name(name)?,
            None => current.loop_.name.clone(),
        };

        if let Some(rule) = &req.config {
            if let Some(workspace_id) = rule.workspace_id.as_deref() {
                ensure_workspace_usable(workspaces, ctx, workspace_id).await?;
            }
        }

        // Scope update: if a new scope is requested, resolve it; otherwise keep current.
        let (next_scope, next_owner) = if let Some(requested) = req.scope {
            let resolved = resolve_scope_for_adapter_create(ctx, Some(requested))?;
            (resolved.scope, resolved.owner_user_id)
        } else {
            (current.loop_.scope, current.loop_.owner_user_id.clone())
        };

        if let Some(UpdateLoopSource::AdapterEvent(source)) = &req.source {
            let LoopSourceRow::AdapterEvent(current_source) = &current.source else {
                return Err(SharedAdapterError::BadRequest(
                    "adapter-event source updates are only valid for adapter_event loops".into(),
                )
                .into());
            };
            let next_connection_id = source
                .connection_id
                .clone()
                .unwrap_or_else(|| current_source.connection_id.clone());
            let connection = adapter_connections
                .for_auth(ctx, &next_connection_id)
                .await?;
            if connection.adapter_kind != current_source.adapter_kind {
                return Err(SharedAdapterError::BadRequest(format!(
                    "Connection {} is for adapter {}, not {}",
                    connection.id, connection.adapter_kind, current_source.adapter_kind
                ))
                .into());
            }
        }
        let next_fire = if let Some(UpdateLoopSource::Schedule(source)) = &req.source {
            let LoopSourceRow::Schedule(current_source) = &current.source else {
                return Err(SharedAdapterError::BadRequest(
                    "schedule source updates are only valid for schedule loops".into(),
                )
                .into());
            };
            let expr = source
                .cron_expression
                .as_deref()
                .unwrap_or(&current_source.cron_expression);
            Some(next_fire_at(expr).map_err(|e| SharedAdapterError::BadRequest(e.to_string()))?)
        } else {
            None
        };

        let rule_update = req
            .config
            .map(|rule| next_config(&current, rule))
            .transpose()?;
        let source_update = match req.source {
            Some(UpdateLoopSource::AdapterEvent(source)) => {
                let LoopSourceRow::AdapterEvent(current_source) = &current.source else {
                    return Err(SharedAdapterError::BadRequest(
                        "adapter-event source updates are only valid for adapter_event loops"
                            .into(),
                    )
                    .into());
                };
                Some(StoreUpdateSource::AdapterEvent(AdapterSourceUpdate {
                    connection_id: source
                        .connection_id
                        .unwrap_or_else(|| current_source.connection_id.clone()),
                    source_matcher: source
                        .source_matcher
                        .unwrap_or_else(|| current_source.source_matcher.clone()),
                    source_name: source
                        .source_name
                        .unwrap_or_else(|| current_source.source_name.clone()),
                    event_filter: source
                        .event_filter
                        .unwrap_or_else(|| current_source.event_filter.clone()),
                    adapter_config: source
                        .adapter_config
                        .unwrap_or_else(|| current_source.adapter_config.clone()),
                    allow_direct_send: source
                        .allow_direct_send
                        .unwrap_or(current_source.allow_direct_send),
                    close_task_on_close: source
                        .close_task_on_close
                        .unwrap_or(current_source.close_task_on_close),
                    notify_on_success: source
                        .notify_on_success
                        .unwrap_or(current_source.notify_on_success),
                }))
            }
            Some(UpdateLoopSource::Schedule(source)) => {
                let LoopSourceRow::Schedule(current_source) = &current.source else {
                    return Err(SharedAdapterError::BadRequest(
                        "schedule source updates are only valid for schedule loops".into(),
                    )
                    .into());
                };
                let next_fire = next_fire.ok_or_else(|| {
                    SharedAdapterError::Internal("missing schedule next fire".to_string())
                })?;
                Some(StoreUpdateSource::Schedule(NewScheduleSource {
                    cron_expression: source
                        .cron_expression
                        .unwrap_or_else(|| current_source.cron_expression.clone()),
                    timezone: source
                        .timezone
                        .unwrap_or_else(|| current_source.timezone.clone()),
                    message_template: source
                        .message_template
                        .unwrap_or_else(|| current_source.message_template.clone()),
                    next_fire_at: next_fire,
                }))
            }
            Some(UpdateLoopSource::Manual(_)) | None => None,
        };

        let updated = self
            .store
            .update(
                id,
                LoopUpdate {
                    name: next_name,
                    scope: next_scope,
                    owner_user_id: next_owner,
                },
                rule_update,
                source_update,
                ctx,
            )
            .await?;

        if !updated {
            return Err(SharedAdapterError::NotFound(format!("Loop {id}")).into());
        }

        let record = self
            .store
            .get(id, ctx)
            .await?
            .ok_or_else(|| SharedAdapterError::NotFound(format!("Loop {id} not found")))?;
        Ok(record)
    }

    /// Delete or tombstone a Loop for an administrator.
    pub async fn delete(&self, ctx: &ScopeContext, id: &str) -> Result<(), AdapterError> {
        require_admin(ctx)?;
        let result = self.store.delete(id, ctx).await?;

        if result == 0 {
            return Err(SharedAdapterError::NotFound(format!("Loop {id}")).into());
        }

        Ok(())
    }

    /// Approve Loop execution under a resolved run-as identity.
    pub async fn activate(
        &self,
        users: &dyn UserResolver,
        ctx: &ScopeContext,
        id: &str,
        requested_user: &str,
    ) -> Result<(), AdapterError> {
        require_admin(ctx)?;
        let run_as_user_id = resolve_run_as_user(users, requested_user).await?;
        if !self.store.activate(id, &run_as_user_id, ctx).await? {
            return Err(SharedAdapterError::NotFound(format!("Loop {id} not found")).into());
        }
        Ok(())
    }

    /// Pause an administrator-visible Loop.
    pub async fn pause(&self, ctx: &ScopeContext, id: &str) -> Result<(), AdapterError> {
        require_admin(ctx)?;
        if !self.store.pause(id, ctx).await? {
            return Err(SharedAdapterError::NotFound(format!("Loop {id} not found")).into());
        }
        Ok(())
    }

    /// Resume a Loop with an existing execution approval.
    pub async fn resume(&self, ctx: &ScopeContext, id: &str) -> Result<(), AdapterError> {
        require_admin(ctx)?;
        if !self.store.resume(id, ctx).await? {
            return Err(SharedAdapterError::Conflict(
                "Loop cannot resume until an administrator selects a run-as user".to_string(),
            )
            .into());
        }
        Ok(())
    }

    /// Make a non-deleted Git-owned configuration editable.
    pub async fn override_git(&self, ctx: &ScopeContext, id: &str) -> Result<(), AdapterError> {
        require_admin(ctx)?;
        if !self.store.override_git(id, ctx).await? {
            return Err(SharedAdapterError::Conflict(
                "Only a non-deleted Git-synced Loop can be overridden".to_string(),
            )
            .into());
        }
        Ok(())
    }

    /// Restore a Loop from its Git provenance and report synchronization failure.
    pub(crate) async fn reset_git(
        &self,
        repositories: &dyn RepositoryReader,
        loop_sync: &LoopSyncService,
        ctx: &ScopeContext,
        id: &str,
    ) -> Result<ResetLoopOutcome, AdapterError> {
        require_admin(ctx)?;
        let current = self
            .store
            .get(id, ctx)
            .await?
            .ok_or_else(|| SharedAdapterError::NotFound(format!("Loop {id} not found")))?;
        let repository_id = current
            .loop_
            .source_repository_id
            .as_deref()
            .ok_or_else(|| {
                SharedAdapterError::BadRequest("Loop has no Git provenance".to_string())
            })?;
        let source_key = current.loop_.source_key.as_deref().ok_or_else(|| {
            SharedAdapterError::BadRequest("Loop has no Git source key".to_string())
        })?;
        let repository = repositories
            .describe(ctx, repository_id)
            .await?
            .ok_or_else(|| {
                SharedAdapterError::NotFound(format!("Repository {repository_id} not found"))
            })?;

        let result = loop_sync
            .reset_repo_loop_for(&repository, ctx, source_key)
            .await;
        let record = self
            .store
            .get(id, ctx)
            .await?
            .ok_or_else(|| SharedAdapterError::NotFound(format!("Loop {id} not found")))?;
        let (resynced, error) = match result {
            Ok(true) => (true, None),
            Ok(false) => (false, Some("Loop provenance no longer exists".to_string())),
            Err(error) => (false, Some(error.to_string())),
        };
        Ok(ResetLoopOutcome {
            record,
            resynced,
            error,
        })
    }
}

/// Merge a config patch onto the Loop's current config.
///
/// The Loop form does not send `model` (`buildUpdateConfig` in
/// `loop-form.tsx`), so a carried-forward value is dropped on an `agent_type`
/// change rather than rejected. Only a `model` the patch names is validated.
fn next_config(
    current: &LoopRecord,
    rule: UpdateLoopConfig,
) -> Result<NewLoopConfig, AdapterError> {
    // Both read `rule` before `unwrap_or_else` below moves it.
    let agent_changed = rule
        .agent_type
        .as_deref()
        .is_some_and(|next| next != current.config.agent_type.as_str());
    let named_model = rule.model.as_ref().and_then(|m| m.as_deref());

    let agent_type = rule
        .agent_type
        .unwrap_or_else(|| current.config.agent_type.as_str().to_string());
    require_agent_and_model(&agent_type, named_model)?;
    let model = rule.model.unwrap_or_else(|| {
        current
            .config
            .model
            .clone()
            .filter(|m| !(agent_changed && catalog::model_conflicts(&agent_type, m)))
    });
    Ok(NewLoopConfig {
        workspace_id: rule
            .workspace_id
            .unwrap_or_else(|| current.config.workspace_id.clone()),
        routing_mode: rule.routing_mode.unwrap_or(current.config.routing_mode),
        ongoing_task_id: rule
            .ongoing_task_id
            .unwrap_or_else(|| current.config.ongoing_task_id.clone()),
        prompt: rule.prompt.unwrap_or_else(|| current.config.prompt.clone()),
        task_name: rule
            .task_name
            .unwrap_or_else(|| current.config.task_name.clone()),
        tag_ids: rule
            .tag_ids
            .unwrap_or_else(|| current.config.tag_ids.clone()),
        skills: rule.skills.unwrap_or_else(|| current.config.skills.clone()),
        run_as_user_id: current.config.run_as_user_id.clone(),
        agent_type,
        model,
        inactive_ttl_days: rule
            .inactive_ttl_days
            .unwrap_or(current.config.inactive_ttl_days),
    })
}

/// Result of resetting a Git declaration, including a best-effort sync failure.
pub(crate) struct ResetLoopOutcome {
    pub record: LoopRecord,
    pub resynced: bool,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_ports::identity::RunAsUser;
    use moatless_types::store::StoreError;

    struct MissingUser;

    #[async_trait::async_trait]
    impl UserResolver for MissingUser {
        async fn canonical_id(&self, _: &str) -> Result<Option<uuid::Uuid>, StoreError> {
            Ok(None)
        }

        async fn run_as_target(&self, _: &str) -> Result<Option<RunAsUser>, StoreError> {
            panic!("Loop approval resolves canonical identity")
        }
    }

    fn disconnected() -> Loops {
        Loops::new(Arc::new(LoopDao::new(
            sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://unused@127.0.0.1:1/unused")
                .unwrap(),
        )))
    }

    fn record_on(agent_type: &str, model: Option<&str>) -> LoopRecord {
        let now = Utc::now();
        LoopRecord {
            loop_: crate::loops::models::LoopRow {
                id: "loop".into(),
                name: "Review".into(),
                kind: DomainLoopKind::AdapterEvent,
                active: true,
                scope: moatless_types::access_control::Scope::Global,
                owner_user_id: None,
                created_by: "admin".into(),
                created_at: now,
                updated_at: now,
                config_source: moatless_types::loops::LoopConfigSource::Manual,
                source_repository_id: None,
                source_key: None,
                source_config_path: None,
                deleted: false,
            },
            config: crate::loops::models::LoopConfigRow {
                workspace_id: "ws".into(),
                routing_mode: moatless_types::loops::RoutingMode::BySubject,
                ongoing_task_id: None,
                prompt: None,
                task_name: None,
                tag_ids: vec![],
                skills: vec![],
                run_as_user_id: Some("runner".into()),
                agent_type: moatless_types::agent::AgentType(agent_type.into()),
                model: model.map(ToString::to_string),
                inactive_ttl_days: None,
            },
            source: LoopSourceRow::Manual,
        }
    }

    #[test]
    fn an_omitted_model_keeps_the_current_one() {
        let next = next_config(
            &record_on("claude-code", Some("opus")),
            UpdateLoopConfig::default(),
        )
        .expect("no change is valid");
        assert_eq!(next.model.as_deref(), Some("opus"));
        assert_eq!(next.agent_type, "claude-code");
    }

    #[test]
    fn a_carried_forward_model_the_new_agent_does_not_offer_is_dropped() {
        // The patch names only the agent, so `opus` would carry forward onto
        // Codex. The Loop form never sends `model`, so this is the UI's every
        // agent-type change.
        let next = next_config(
            &record_on("claude-code", Some("opus")),
            UpdateLoopConfig {
                agent_type: Some("codex".into()),
                ..UpdateLoopConfig::default()
            },
        )
        .expect("changing agent drops the model it belonged to");
        assert_eq!(next.model, None);
        assert_eq!(next.agent_type, "codex");
    }

    /// Sync wrote `model` without validating it before this change, so a row
    /// can already hold a conflicting pair. An edit that names neither field
    /// leaves it alone.
    #[test]
    fn an_edit_that_changes_no_agent_keeps_a_conflicting_model() {
        let next = next_config(
            &record_on("claude-code", Some("gpt-5.5")),
            UpdateLoopConfig {
                prompt: Some(Some("review this".into())),
                ..UpdateLoopConfig::default()
            },
        )
        .expect("an unrelated edit validates nothing");
        assert_eq!(next.model.as_deref(), Some("gpt-5.5"));
    }

    #[test]
    fn a_model_belonging_to_another_agent_is_rejected() {
        let Err(err) = next_config(
            &record_on("claude-code", None),
            UpdateLoopConfig {
                agent_type: Some("codex".into()),
                model: Some(Some("opus".into())),
                ..UpdateLoopConfig::default()
            },
        ) else {
            panic!("codex does not offer opus");
        };
        assert!(
            matches!(&err, AdapterError::Shared(SharedAdapterError::BadRequest(m)) if m.contains("model `opus` belongs to another agentType")),
            "{err:?}"
        );
    }

    #[test]
    fn clearing_the_model_frees_an_agent_change() {
        let next = next_config(
            &record_on("claude-code", Some("opus")),
            UpdateLoopConfig {
                agent_type: Some("codex".into()),
                model: Some(None),
                ..UpdateLoopConfig::default()
            },
        )
        .expect("cleared model imposes nothing on codex");
        assert_eq!(next.model, None);
        assert_eq!(next.agent_type, "codex");
    }

    /// `model` reaches the agent CLI as written, so a model no catalog entry
    /// lists is unknown rather than wrong.
    #[test]
    fn a_model_no_agent_offers_is_accepted() {
        let next = next_config(
            &record_on("claude-code", None),
            UpdateLoopConfig {
                model: Some(Some("claude-haiku-4-5-20251001".into())),
                ..UpdateLoopConfig::default()
            },
        )
        .expect("a pinned upstream id runs");
        assert_eq!(next.model.as_deref(), Some("claude-haiku-4-5-20251001"));
    }

    #[tokio::test]
    async fn non_admin_mutations_refuse_before_reading_or_writing() {
        let loops = disconnected();
        let ctx = ScopeContext {
            user_id: "caller".into(),
            is_admin: false,
        };
        for result in [
            loops.delete(&ctx, "loop").await,
            loops.pause(&ctx, "loop").await,
            loops.resume(&ctx, "loop").await,
            loops.override_git(&ctx, "loop").await,
            loops.activate(&MissingUser, &ctx, "loop", "missing").await,
        ] {
            assert!(matches!(
                result,
                Err(AdapterError::Shared(SharedAdapterError::AccessDenied(_)))
            ));
        }
    }

    #[tokio::test]
    async fn unknown_run_as_user_is_rejected_before_activation() {
        let result = disconnected()
            .activate(
                &MissingUser,
                &ScopeContext {
                    user_id: "admin".into(),
                    is_admin: true,
                },
                "loop",
                "missing",
            )
            .await;
        assert!(
            matches!(result, Err(AdapterError::Shared(SharedAdapterError::BadRequest(message))) if message == "Run-as user missing does not exist")
        );
    }
}
