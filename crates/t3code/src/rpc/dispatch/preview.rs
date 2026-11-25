//! The `preview.*` methods.

use crate::authentication::viewer_scope;
use crate::oauth::Scope;
use crate::preview;
use crate::preview::PreviewInput;
use crate::rpc::errors::preview_answer;
use crate::rpc::table::MethodTable;
use crate::rpc::table::Request;
use serde_json::Value;

pub(super) fn preview_methods() -> MethodTable {
    MethodTable::new()
        .session("preview.close", Scope::OrchestrationOperate, preview_close)
        .detached("preview.list", Scope::OrchestrationRead, preview_list)
        .session(
            "preview.navigate",
            Scope::OrchestrationOperate,
            preview_navigate,
        )
        .session("preview.open", Scope::OrchestrationOperate, preview_open)
        .session(
            "preview.reportStatus",
            Scope::OrchestrationOperate,
            preview_report_status,
        )
        .session(
            "preview.resize",
            Scope::OrchestrationOperate,
            preview_resize,
        )
}

pub(super) async fn preview_list(request: Request<PreviewInput>) -> Vec<Value> {
    let Request {
        id,
        scope,
        payload,
        state,
        identity,
        ..
    } = request;
    preview_answer(
        &id,
        scope,
        preview::list(&state.preview, &viewer_scope(&identity), &payload).await,
    )
}

pub(super) async fn preview_open(request: Request<PreviewInput>) -> Vec<Value> {
    let Request {
        id,
        scope,
        payload,
        state,
        identity,
        ..
    } = request;
    preview_answer(
        &id,
        scope,
        preview::open(&state.preview, &viewer_scope(&identity), &payload).await,
    )
}

pub(super) async fn preview_navigate(request: Request<PreviewInput>) -> Vec<Value> {
    let Request {
        id,
        scope,
        payload,
        state,
        identity,
        ..
    } = request;
    preview_answer(
        &id,
        scope,
        preview::navigate(&state.preview, &viewer_scope(&identity), &payload).await,
    )
}

pub(super) async fn preview_resize(request: Request<PreviewInput>) -> Vec<Value> {
    let Request {
        id,
        scope,
        payload,
        state,
        identity,
        ..
    } = request;
    preview_answer(
        &id,
        scope,
        preview::resize(&state.preview, &viewer_scope(&identity), &payload).await,
    )
}

/// The three void methods answer `null` rather than a snapshot, matching the
/// contract's declared success type.
pub(super) async fn preview_report_status(request: Request<PreviewInput>) -> Vec<Value> {
    let Request {
        id,
        scope,
        payload,
        state,
        identity,
        ..
    } = request;
    preview_answer(
        &id,
        scope,
        preview::report_status(&state.preview, &viewer_scope(&identity), &payload)
            .await
            .map(|()| Value::Null),
    )
}

pub(super) async fn preview_close(request: Request<PreviewInput>) -> Vec<Value> {
    let Request {
        id,
        scope,
        payload,
        state,
        identity,
        ..
    } = request;
    preview_answer(
        &id,
        scope,
        preview::close(&state.preview, &viewer_scope(&identity), &payload)
            .await
            .map(|()| Value::Null),
    )
}
