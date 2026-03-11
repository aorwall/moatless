//! In-process HTTP callback server for webhook outbound tests.
//!
//! Starts a lightweight TCP listener that captures POST requests and exposes
//! `wait_for_callback(timeout)` for test assertions.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// A captured HTTP callback payload.
#[derive(Clone, Debug)]
pub struct CapturedCallback {
    pub body: serde_json::Value,
}

/// A simple HTTP server that captures POST callbacks.
pub struct CallbackServer {
    pub port: u16,
    captures: Arc<Mutex<Vec<CapturedCallback>>>,
    handle: JoinHandle<()>,
}

impl CallbackServer {
    /// Start the callback server on an ephemeral port.
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let captures: Arc<Mutex<Vec<CapturedCallback>>> = Arc::new(Mutex::new(Vec::new()));
        let captures_clone = Arc::clone(&captures);

        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let captures = Arc::clone(&captures_clone);

                tokio::spawn(async move {
                    let mut buf = vec![0u8; 65536];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]).to_string();

                    // Extract body after double CRLF.
                    if let Some(body_start) = request.find("\r\n\r\n") {
                        let body_str = &request[body_start + 4..];
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(body_str) {
                            if let Ok(mut vec) = captures.lock() {
                                vec.push(CapturedCallback { body: json });
                            }
                        }
                    }

                    // Send HTTP 200 response.
                    let response = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });

        Self {
            port,
            captures,
            handle,
        }
    }

    /// Return the callback URL for this server.
    pub fn callback_url(&self) -> String {
        let host =
            std::env::var("CALLBACK_HOST").unwrap_or_else(|_| "127.0.0.1".into());
        format!("http://{}:{}/callback", host, self.port)
    }

    /// Wait for at least one callback to be received, up to `timeout`.
    pub async fn wait_for_callback(&self, timeout: Duration) -> Option<CapturedCallback> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            {
                let guard = self.captures.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(cb) = guard.first() {
                    return Some(cb.clone());
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Stop the callback server.
    pub fn stop(self) {
        self.handle.abort();
    }
}
