//! The `preview.*` methods.

use super::routing::Request;
use crate::authentication::viewer_scope;
use crate::preview;
use crate::rpc::frames::preview_answer;
use serde_json::Value;

pub(super) async fn preview_list(request: Request) -> Vec<Value> {
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

pub(super) async fn preview_open(request: Request) -> Vec<Value> {
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

pub(super) async fn preview_navigate(request: Request) -> Vec<Value> {
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

pub(super) async fn preview_resize(request: Request) -> Vec<Value> {
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
pub(super) async fn preview_refresh(request: Request) -> Vec<Value> {
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

pub(super) async fn preview_report_status(request: Request) -> Vec<Value> {
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

pub(super) async fn preview_close(request: Request) -> Vec<Value> {
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
