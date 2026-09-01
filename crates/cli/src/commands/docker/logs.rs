//! `moat docker logs` — stream logs from one or all services.

use super::compose::{detect_compose_cmd, resolve_compose_dir, run_compose_streaming};
use super::LogsArgs;
use crate::client::ClientError;

pub(super) async fn run(args: LogsArgs, _json: bool) -> Result<(), ClientError> {
    let (compose_dir, _source) = resolve_compose_dir(args.compose_dir.as_deref())?;
    let compose = detect_compose_cmd().await?;

    let mut extra: Vec<String> = vec!["logs".to_string()];
    if args.follow {
        extra.push("-f".to_string());
    }
    if let Some(since) = args.since.as_deref() {
        extra.push("--since".to_string());
        extra.push(since.to_string());
    }
    if let Some(svc) = args.service.as_deref() {
        extra.push(svc.to_string());
    }

    let refs: Vec<&str> = extra.iter().map(String::as_str).collect();
    run_compose_streaming(&compose, &compose_dir, &refs).await
}
