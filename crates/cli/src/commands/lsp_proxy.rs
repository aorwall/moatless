//! `moat lsp-proxy`: the language server command Claude Code starts. It bridges
//! Claude's stdio to the sandbox's shared server socket, so the agent and the
//! UI use one server per checkout.

use std::path::Path;

use tokio::net::UnixStream;

/// Copy stdin to the socket and the socket to stdout until either side closes.
pub async fn run(socket: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let stream = UnixStream::connect(socket).await.map_err(|e| {
        format!(
            "cannot reach the language server at {}: {e}",
            socket.display()
        )
    })?;
    let (mut from_server, mut to_server) = stream.into_split();
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    tokio::select! {
        sent = tokio::io::copy(&mut stdin, &mut to_server) => { sent?; }
        received = tokio::io::copy(&mut from_server, &mut stdout) => { received?; }
    }
    Ok(())
}
