//! A proxied response body, watched until it ends.
//!
//! Every body logs how it ended, its size and its duration once, so a clone
//! that stalls in a pod can be traced to the side that went quiet. A silent
//! upstream is logged, never ended: the agent's own fetches use this proxy,
//! and the pod's stall limit is configured apart from it.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::Bytes;
use futures::Stream;
use tokio::time::{Instant, Sleep};
use tracing::{info, warn};

/// How an upstream body may end, as logged in `outcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Complete,
    UpstreamError,
    /// The client stopped reading before the body ended.
    ClientGone,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::UpstreamError => "upstream_error",
            Self::ClientGone => "client_gone",
        }
    }
}

/// An upstream body passed through chunk by chunk, with each silence longer
/// than `quiet` logged once and counted.
pub(super) struct WatchedBody<S> {
    inner: S,
    quiet: Option<(Duration, Pin<Box<Sleep>>)>,
    /// Whether the current silence has already been logged.
    quiet_now: bool,
    quiet_periods: u32,
    repository_id: String,
    service: &'static str,
    started: Instant,
    bytes: u64,
    ended: bool,
    /// Run once, when the body ends [`Outcome::Complete`].
    on_complete: Option<Box<dyn FnOnce() + Send>>,
}

impl<S> WatchedBody<S> {
    /// Watch `inner`; `None` for `quiet` logs no silence.
    pub(super) fn new(
        inner: S,
        quiet: Option<Duration>,
        repository_id: String,
        service: &'static str,
    ) -> Self {
        Self {
            inner,
            quiet: quiet.map(|limit| (limit, Box::pin(tokio::time::sleep(limit)))),
            quiet_now: false,
            quiet_periods: 0,
            repository_id,
            service,
            started: Instant::now(),
            bytes: 0,
            ended: false,
            on_complete: None,
        }
    }

    /// Run `f` once the body has passed through whole.
    pub(super) fn on_complete(mut self, f: impl FnOnce() + Send + 'static) -> Self {
        self.on_complete = Some(Box::new(f));
        self
    }

    fn end(&mut self, outcome: Outcome) {
        self.ended = true;
        let duration_ms = self.started.elapsed().as_millis();
        if outcome == Outcome::Complete {
            info!(
                repository_id = %self.repository_id,
                service = self.service,
                outcome = outcome.as_str(),
                bytes = self.bytes,
                quiet_periods = self.quiet_periods,
                duration_ms,
                "Proxied git body ended"
            );
            if let Some(on_complete) = self.on_complete.take() {
                on_complete();
            }
        } else {
            warn!(
                repository_id = %self.repository_id,
                service = self.service,
                outcome = outcome.as_str(),
                bytes = self.bytes,
                quiet_periods = self.quiet_periods,
                duration_ms,
                "Proxied git body ended early"
            );
        }
    }
}

impl<S, E> Stream for WatchedBody<S>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.ended {
            return Poll::Ready(None);
        }
        match Pin::new(&mut self.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                self.bytes += chunk.len() as u64;
                self.quiet_now = false;
                if let Some((limit, timer)) = self.quiet.as_mut() {
                    timer.as_mut().reset(Instant::now() + *limit);
                }
                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(Some(Err(error))) => {
                self.end(Outcome::UpstreamError);
                Poll::Ready(Some(Err(std::io::Error::other(error))))
            }
            Poll::Ready(None) => {
                self.end(Outcome::Complete);
                Poll::Ready(None)
            }
            Poll::Pending => {
                if !self.quiet_now
                    && self
                        .quiet
                        .as_mut()
                        .is_some_and(|(_, timer)| timer.as_mut().poll(cx).is_ready())
                {
                    self.quiet_now = true;
                    self.quiet_periods += 1;
                    warn!(
                        repository_id = %self.repository_id,
                        service = self.service,
                        bytes = self.bytes,
                        duration_ms = self.started.elapsed().as_millis(),
                        "Proxied git body went quiet"
                    );
                }
                Poll::Pending
            }
        }
    }
}

impl<S> Drop for WatchedBody<S> {
    fn drop(&mut self) {
        if !self.ended {
            self.end(Outcome::ClientGone);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    type Chunk = Result<Bytes, std::io::Error>;

    fn watched<S>(inner: S, idle: Option<Duration>) -> WatchedBody<S> {
        WatchedBody::new(inner, idle, "repo-1".to_string(), "git-upload-pack")
    }

    #[tokio::test]
    async fn a_finished_body_passes_through_whole() {
        let chunks: Vec<Chunk> = vec![Ok(Bytes::from_static(b"0008NAK\n")), Ok(Bytes::new())];
        let mut body = watched(futures::stream::iter(chunks), Some(Duration::from_secs(5)));

        assert_eq!(body.next().await.unwrap().unwrap(), "0008NAK\n");
        assert!(body.next().await.unwrap().unwrap().is_empty());
        assert!(body.next().await.is_none());
        assert_eq!(body.bytes, 8);
        assert!(body.ended);
    }

    #[tokio::test]
    async fn only_a_body_that_ends_whole_runs_its_completion() {
        let ran = Arc::new(AtomicBool::new(false));
        let flag = ran.clone();
        let chunks: Vec<Chunk> = vec![Ok(Bytes::from_static(b"0000"))];
        let body = watched(futures::stream::iter(chunks), None)
            .on_complete(move || flag.store(true, Ordering::SeqCst));
        let _: Vec<_> = body.collect().await;
        assert!(ran.load(Ordering::SeqCst));

        let ran = Arc::new(AtomicBool::new(false));
        let flag = ran.clone();
        let failed: Vec<Chunk> = vec![Err(std::io::Error::other("reset"))];
        let body = watched(futures::stream::iter(failed), None)
            .on_complete(move || flag.store(true, Ordering::SeqCst));
        let _: Vec<_> = body.collect().await;
        assert!(!ran.load(Ordering::SeqCst));
    }

    #[tokio::test(start_paused = true)]
    async fn a_body_that_goes_quiet_is_counted_and_kept_open() {
        let first: Chunk = Ok(Bytes::from_static(b"0032"));
        let last: Chunk = Ok(Bytes::from_static(b"0000"));
        let (tx, rx) = futures::channel::mpsc::unbounded();
        let inner = futures::stream::iter([first]).chain(rx);
        let mut body = watched(inner, Some(Duration::from_secs(20)));

        assert!(body.next().await.unwrap().is_ok());
        let waited = tokio::time::timeout(Duration::from_mins(1), body.next()).await;
        assert!(waited.is_err(), "a quiet body must stay open");
        assert_eq!(body.quiet_periods, 1, "one silence is logged once");

        tx.unbounded_send(last).unwrap();
        drop(tx);
        assert_eq!(body.next().await.unwrap().unwrap(), "0000");
        assert!(body.next().await.is_none());
        assert_eq!(body.quiet_periods, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_body_without_a_quiet_limit_counts_no_silence() {
        let inner = futures::stream::pending::<Chunk>();
        let mut body = watched(inner, None);

        let waited = tokio::time::timeout(Duration::from_mins(1), body.next()).await;

        assert!(waited.is_err());
        assert_eq!(body.quiet_periods, 0);
    }
}
