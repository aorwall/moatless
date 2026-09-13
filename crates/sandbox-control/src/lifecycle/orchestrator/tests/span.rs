//! The span a detached operation runs under: that it is a root, and that it
//! links back to the caller that asked for the transition.

use super::super::*;
use super::harness::*;

/// A detached operation must not run under the span of whoever asked for it:
/// see [`Lifecycle::detached_span`] for why. Asserted on the span tree, since
/// that's where the fault actually appears.
#[tokio::test]
async fn a_detached_operation_gets_a_root_span_linked_back_to_the_caller() {
    use std::sync::Mutex as StdMutex;
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::registry::LookupSpan;

    #[derive(Default)]
    struct Seen {
        /// Every span opened, as `(name, parent name)`.
        opened: Vec<(String, Option<String>)>,
        /// Every `follows_from`, as `(name, followed name)`.
        links: Vec<(String, String)>,
    }

    struct Recorder(Arc<StdMutex<Seen>>);

    impl<S> tracing_subscriber::Layer<S> for Recorder
    where
        S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    {
        fn on_new_span(
            &self,
            attrs: &tracing::span::Attributes<'_>,
            id: &tracing::Id,
            ctx: Context<'_, S>,
        ) {
            let parent = ctx
                .span(id)
                .and_then(|span| span.parent().map(|p| p.name().to_owned()));
            self.0
                .lock()
                .expect("lock")
                .opened
                .push((attrs.metadata().name().to_owned(), parent));
        }

        fn on_follows_from(&self, id: &tracing::Id, follows: &tracing::Id, ctx: Context<'_, S>) {
            let name = |id| {
                ctx.span(id)
                    .map(|s| s.name().to_owned())
                    .unwrap_or_default()
            };
            self.0
                .lock()
                .expect("lock")
                .links
                .push((name(id), name(follows)));
        }
    }

    let seen = Arc::new(StdMutex::new(Seen::default()));
    let subscriber = tracing_subscriber::registry().with(Recorder(Arc::clone(&seen)));
    let _guard = tracing::subscriber::set_default(subscriber);

    // Refused on eligibility, which is enough: the span is opened before
    // the operation is spawned, so what it hangs off does not depend on
    // the outcome.
    let h = harness(closed_removed());
    let caller = tracing::info_span!("caller");
    let _ = async { h.lifecycle.start("t").await }
        .instrument(caller)
        .await;

    let seen = seen.lock().expect("lock");
    let parent = seen
        .opened
        .iter()
        .find(|(name, _)| name == "sandbox.lifecycle")
        .map(|(_, parent)| parent.clone())
        .expect("the operation opened a span");
    assert_eq!(parent, None, "the operation adopted the caller's span");
    assert!(
        seen.links
            .iter()
            .any(|(from, to)| from == "sandbox.lifecycle" && to == "caller"),
        "the operation is not findable from the caller: {:?}",
        seen.links
    );
}
