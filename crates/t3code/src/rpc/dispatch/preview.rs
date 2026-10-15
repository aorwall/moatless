//! The `preview.*` methods.

use crate::authentication::viewer_scope;
use crate::oauth::Scope;
use crate::preview;
use crate::rpc::errors::preview_answer;
use crate::rpc::table::MethodTable;
use crate::rpc::table::Request;
use serde_json::Value;

pub(super) fn preview_methods() -> MethodTable {
    MethodTable::new()
        .declared("preview.automationConnect", Scope::OrchestrationOperate)
        .declared("preview.automationFocusHost", Scope::OrchestrationOperate)
        .declared("preview.automationRespond", Scope::OrchestrationOperate)
        .session("preview.close", Scope::OrchestrationOperate, preview_close)
        .detached("preview.list", Scope::OrchestrationRead, preview_list)
        .session(
            "preview.navigate",
            Scope::OrchestrationOperate,
            preview_navigate,
        )
        .session("preview.open", Scope::OrchestrationOperate, preview_open)
        .session(
            "preview.refresh",
            Scope::OrchestrationOperate,
            preview_refresh,
        )
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

pub(super) async fn preview_list(request: Request<Value>) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    preview_answer(
        &id,
        preview::list(&state.preview, &viewer_scope(&identity), &payload).await,
    )
}

pub(super) async fn preview_open(request: Request<Value>) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    preview_answer(
        &id,
        preview::open(&state.preview, &viewer_scope(&identity), &payload).await,
    )
}

pub(super) async fn preview_navigate(request: Request<Value>) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    preview_answer(
        &id,
        preview::navigate(&state.preview, &viewer_scope(&identity), &payload).await,
    )
}

pub(super) async fn preview_resize(request: Request<Value>) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    preview_answer(
        &id,
        preview::resize(&state.preview, &viewer_scope(&identity), &payload).await,
    )
}

/// The three void methods answer `null` rather than a snapshot, matching the
/// contract's declared success type.
pub(super) async fn preview_refresh(request: Request<Value>) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    preview_answer(
        &id,
        preview::refresh(&state.preview, &viewer_scope(&identity), &payload)
            .await
            .map(|()| Value::Null),
    )
}

pub(super) async fn preview_report_status(request: Request<Value>) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    preview_answer(
        &id,
        preview::report_status(&state.preview, &viewer_scope(&identity), &payload)
            .await
            .map(|()| Value::Null),
    )
}

pub(super) async fn preview_close(request: Request<Value>) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    preview_answer(
        &id,
        preview::close(&state.preview, &viewer_scope(&identity), &payload)
            .await
            .map(|()| Value::Null),
    )
}
