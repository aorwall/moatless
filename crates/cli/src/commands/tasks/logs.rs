//! A task's sandbox container logs.

use crate::client::{ClientError, MoatClient};
use futures_util::StreamExt;

/// Stream the sandbox container's logs via SSE.
///
/// With `follow` the stream stays open until Ctrl+C; without it the backend
/// ends the response after the buffered lines, so a caller need not wrap this
/// in `timeout`.
pub(super) async fn stream_logs(
    client: &MoatClient,
    task_id: &str,
    previous: bool,
    follow: bool,
    tail: i64,
) -> Result<(), ClientError> {
    let path = format!(
        "/api/sandbox/v1/tasks/{task_id}/logs?previous={previous}&follow={follow}&tail={tail}"
    );
    let resp = client.raw_get_stream(&path).await?;
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();

    if follow {
        eprintln!("Streaming sandbox logs (Ctrl+C to stop)...");
    }

    loop {
        tokio::select! {
            chunk = stream.next() => {
                match chunk {
                    Some(Ok(bytes)) => {
                        buf.push_str(&String::from_utf8_lossy(&bytes));
                        // SSE format: lines starting with "data:" followed by content.
                        while let Some(pos) = buf.find('\n') {
                            let line: String = buf.drain(..=pos).collect();
                            let line = line.trim();
                            if line.is_empty() {
                                continue;
                            }
                            if let Some(data) = line.strip_prefix("data:") {
                                println!("{}", data.trim_start());
                            } else if !line.starts_with(':') {
                                println!("{line}");
                            }
                        }
                    }
                    Some(Err(e)) => return Err(e.into()),
                    None => break,
                }
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nStopped.");
                break;
            }
        }
    }
    Ok(())
}
