use chrono::Utc;
use moatless_api_schemas::adapters::{
    AdapterConnectionResponse, AdapterSendRequest, AdapterSendResponse,
    CreateAdapterConnectionRequest,
};
use moatless_api_schemas::user::{SendDmRequest, SendDmResponse};

use super::access::ensure_task_access_only;
use super::projection::adapter_connection_response;
use moatless_adapter_model::access::resolve_scope_for_adapter_create;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::outbound::OutboundReply;
use moatless_identity::ports::ProviderAccount;
use moatless_loops::bindings::outbound::checked_presentation;
use moatless_loops::bindings::outbound::outbound_files;
use moatless_loops::bindings::outbound::truncate_text;
use moatless_loops::bindings::runtime::ORIGIN_CONNECTED_THREAD;
use moatless_types::adapter::AdapterConnection;
use moatless_types::adapter::AdapterKind;

use moatless_types::access_control::ScopeContext;

/// Adapter connection and outbound delivery operations.
pub struct AdapterCommands {
    pub(crate) adapter_connections:
        std::sync::Arc<moatless_adapter_model::connections::AdapterConnections>,
    pub(crate) adapters: Option<std::sync::Arc<moatless_adapter_model::registry::Adapters>>,
    pub(crate) task_reader: std::sync::Arc<moatless_tasks::ports::TaskProjection>,
    pub(crate) files: std::sync::Arc<moatless_core::file_storage::Files>,
    pub(crate) external_reply:
        std::sync::Arc<moatless_loops::bindings::reply::ExternalReplyService>,
    pub(crate) task_bindings: std::sync::Arc<moatless_loops::bindings::operations::Bindings>,
    pub(crate) users: std::sync::Arc<moatless_identity::ports::UserDirectory>,
}

impl AdapterCommands {
    pub async fn create_adapter_connection(
        &self,
        ctx: &ScopeContext,
        req: CreateAdapterConnectionRequest,
    ) -> Result<AdapterConnectionResponse, AdapterError> {
        if req.external_account_id.trim().is_empty() {
            return Err(
                SharedAdapterError::BadRequest("external_account_id is required".into()).into(),
            );
        }

        let now = Utc::now();
        let adapter_kind = req.adapter_kind;
        let resolved = resolve_scope_for_adapter_create(ctx, req.scope)?;
        let row = AdapterConnection {
            id: format!("pc_{}", uuid::Uuid::new_v4()),
            adapter_kind,
            connection_kind: req.connection_kind.unwrap_or_else(|| "webhook".to_string()),
            scope_value: req.external_account_id,
            credential_ref: None,
            webhook_secret: req.webhook_secret,
            transport_config: req
                .transport_config
                .unwrap_or_else(|| serde_json::json!({})),
            created_by: ctx.user_id.clone(),
            scope: resolved.scope,
            owner_user_id: resolved.owner_user_id,
            created_at: now,
            updated_at: now,
        };
        let saved = self.adapter_connections.save(ctx, &row).await?;
        Ok(adapter_connection_response(&saved))
    }
    pub async fn adapter_send(
        &self,
        ctx: &ScopeContext,
        req: AdapterSendRequest,
    ) -> Result<AdapterSendResponse, AdapterError> {
        let adapter_kind = req.adapter;

        if let Some(task_id) = req.task_id.as_deref() {
            ensure_task_access_only(self.task_reader.as_ref(), ctx, task_id).await?;
        }

        let connection = self
            .adapter_connections
            .resolve(ctx, adapter_kind, req.connection_id.as_deref())
            .await?;

        let providers = self.adapters.as_ref().ok_or_else(|| {
            SharedAdapterError::AdapterNotConfigured("adapters not configured".into())
        })?;
        let provider = providers.outbound_for_direct_destination(adapter_kind)?;

        let files = outbound_files(&self.files, &req.file_ids).await;
        if !files.is_empty() && !provider.supports_files() {
            return Err(SharedAdapterError::BadRequest(format!(
                "{} adapter does not support file attachments",
                req.adapter.as_str()
            ))
            .into());
        }

        let presentation = checked_presentation(req.presentation, adapter_kind)?;
        // A destination post is often the only message a reader sees — nobody in
        // the channel is in a thread that leads anywhere else — so it carries the
        // same footer a reply does whenever the caller named a task.
        let provenance = match req.task_id.as_deref() {
            Some(task_id) => {
                self.external_reply
                    .task_provenance(adapter_kind, task_id)
                    .await
            }
            None => None,
        };
        let reply = OutboundReply {
            text: truncate_text(&req.text, provider.max_message_length()),
            provenance,
            footer: None,
            author_user_id: None,
            is_error: false,
            files,
            presentation,
        };

        let result = provider
            .post_to_destination(&req.destination, &reply)
            .await?;

        let binding_id = if let Some(task_id) = req.task_id.as_deref() {
            let binding = self
                .task_bindings
                .bind(
                    &connection,
                    None,
                    task_id,
                    provider.connection_provider_key(),
                    result.subject_kind,
                    &result.subject_key,
                    result.destination_config.clone(),
                    ORIGIN_CONNECTED_THREAD,
                    Some(result.message_external_id.clone()),
                )
                .await?;
            if let Some(followup) = providers.post_binding_followup(adapter_kind) {
                followup
                    .on_binding_created(&connection, &binding, task_id)
                    .await;
            }
            Some(binding.id)
        } else {
            None
        };

        Ok(AdapterSendResponse {
            external_id: result.message_external_id,
            subject_kind: result.subject_kind.to_string(),
            binding_id,
        })
    }
    pub async fn send_user_dm(
        &self,
        ctx: &ScopeContext,
        login: String,
        req: SendDmRequest,
    ) -> Result<SendDmResponse, AdapterError> {
        if let Some(task_id) = req.task_id.as_deref() {
            ensure_task_access_only(self.task_reader.as_ref(), ctx, task_id).await?;
        }

        let provider_user_id = match self
            .users
            .provider_account_for_login(&req.provider, &login)
            .await?
        {
            ProviderAccount::Linked(id) => id,
            ProviderAccount::NoUser => {
                return Err(SharedAdapterError::NotFound(format!("User {login} not found")).into());
            }
            ProviderAccount::Unlinked => {
                return Err(SharedAdapterError::NotFound(format!(
                    "User {login} has no {} provider id configured",
                    req.provider
                ))
                .into());
            }
        };

        let kind = AdapterKind::parse(&req.provider).ok_or_else(|| {
            SharedAdapterError::BadRequest(format!("Unknown adapter provider: {}", req.provider))
        })?;

        let connection = self.adapter_connections.resolve(ctx, kind, None).await?;

        let providers = self.adapters.as_ref().ok_or_else(|| {
            SharedAdapterError::AdapterNotConfigured("adapters not configured".into())
        })?;
        let dm_provider = providers.outbound_for_direct_message(kind)?;
        let send_provider = providers.outbound_for_direct_destination(kind)?;

        let destination = dm_provider
            .dm_destination(&provider_user_id)
            .await?
            .ok_or_else(|| {
                SharedAdapterError::BadRequest(format!(
                    "User DM is not supported for adapter {}",
                    kind.as_str()
                ))
            })?;

        let reply = OutboundReply {
            text: truncate_text(&req.text, send_provider.max_message_length()),
            // A DM has no task to point at: this path addresses a person, not a
            // run. The destination path does carry provenance.
            provenance: None,
            footer: None,
            author_user_id: None,
            is_error: false,
            files: vec![],
            presentation: None,
        };

        let result = send_provider
            .post_to_destination(&destination, &reply)
            .await?;

        let conversation_id = result
            .destination_config
            .get("channel")
            .and_then(|v| v.as_str())
            .or_else(|| {
                result
                    .destination_config
                    .get("chat_id")
                    .and_then(|v| v.as_str())
            })
            .unwrap_or(provider_user_id.as_str())
            .to_string();

        let binding_id = if let Some(task_id) = req.task_id.as_deref() {
            let binding = self
                .task_bindings
                .bind(
                    &connection,
                    None,
                    task_id,
                    send_provider.connection_provider_key(),
                    result.subject_kind,
                    &result.subject_key,
                    result.destination_config.clone(),
                    ORIGIN_CONNECTED_THREAD,
                    Some(result.message_external_id.clone()),
                )
                .await?;
            binding.id
        } else {
            String::new()
        };

        Ok(SendDmResponse {
            binding_id,
            conversation_id,
            message_ts: result.message_external_id,
        })
    }
}
