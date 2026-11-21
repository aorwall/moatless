//! The `terminal.*` methods that are not the attach stream.

use crate::authentication::viewer_scope;
use crate::oauth::Scope;
use crate::rpc::envelope::success_exit;
use crate::rpc::errors::terminal_failure_exit;
use crate::rpc::table::MethodTable;
use crate::rpc::table::Request;
use crate::terminal;
use crate::terminal::TerminalInput;
use serde_json::Value;

pub(super) fn terminal_methods() -> MethodTable {
    MethodTable::new()
        .session("terminal.clear", Scope::TerminalOperate, terminal_clear)
        .session("terminal.close", Scope::TerminalOperate, terminal_close)
        .session("terminal.open", Scope::TerminalOperate, terminal_open)
        .session("terminal.resize", Scope::TerminalOperate, terminal_resize)
        .session("terminal.write", Scope::TerminalOperate, terminal_write)
}

pub(super) async fn terminal_open(request: Request<TerminalInput>) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match terminal::open(
        state.reads.conversations(),
        state.sandboxes.as_ref(),
        &state.terminal,
        &viewer,
        &payload,
    )
    .await
    {
        Ok(snapshot) => vec![success_exit(&id, snapshot)],
        Err(error) => vec![terminal_failure_exit(&id, &payload, &error, &identity)],
    }
}

pub(super) async fn terminal_write(request: Request<TerminalInput>) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match terminal::write(
        state.reads.conversations(),
        state.sandboxes.as_ref(),
        &viewer,
        &payload,
    )
    .await
    {
        Ok(()) => vec![success_exit(&id, Value::Null)],
        Err(error) => vec![terminal_failure_exit(&id, &payload, &error, &identity)],
    }
}

pub(super) async fn terminal_resize(request: Request<TerminalInput>) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match terminal::resize(
        state.reads.conversations(),
        state.sandboxes.as_ref(),
        &viewer,
        &payload,
    )
    .await
    {
        Ok(()) => vec![success_exit(&id, Value::Null)],
        Err(error) => vec![terminal_failure_exit(&id, &payload, &error, &identity)],
    }
}

pub(super) async fn terminal_clear(request: Request<TerminalInput>) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match terminal::clear(
        state.reads.conversations(),
        state.sandboxes.as_ref(),
        &viewer,
        &payload,
    )
    .await
    {
        Ok(()) => vec![success_exit(&id, Value::Null)],
        Err(error) => vec![terminal_failure_exit(&id, &payload, &error, &identity)],
    }
}

pub(super) async fn terminal_close(request: Request<TerminalInput>) -> Vec<Value> {
    let Request {
        id,
        payload,
        state,
        identity,
        ..
    } = request;
    let viewer = viewer_scope(&identity);
    match terminal::close(
        state.reads.conversations(),
        state.sandboxes.as_ref(),
        &state.terminal,
        &viewer,
        &payload,
    )
    .await
    {
        Ok(()) => vec![success_exit(&id, Value::Null)],
        Err(error) => vec![terminal_failure_exit(&id, &payload, &error, &identity)],
    }
}
