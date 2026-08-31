//! The concrete self-echo filter behind `message::send::SelfEchoGuard`.
//!
//! It holds the adapter registry ([`Adapters`]) and the in-memory race-window
//! cache ([`RecentlySent`], down in `bindings`) and answers the two questions
//! the message-send flows ask before treating an inbound event as a genuine
//! external message rather than an echo of our own outbound. It lives beside
//! the registry it reads — not under `bindings`, which must not reach up into
//! adapters (Step 7) — and the port it implements is declared down in the tasks
//! band (`task`/`message`), which is what makes both references point downward.

use std::sync::Arc;

use moatless_adapter_model::registry::Adapters;
use crate::bindings::recently_sent::RecentlySent;
use moatless_tasks::message::send::SelfEchoGuard;

pub struct SelfEchoFilter {
    recently_sent: Arc<RecentlySent>,
    adapters: Option<Arc<Adapters>>,
}

impl SelfEchoFilter {
    pub fn new(recently_sent: Arc<RecentlySent>, adapters: Option<Arc<Adapters>>) -> Self {
        Self {
            recently_sent,
            adapters,
        }
    }
}

impl SelfEchoGuard for SelfEchoFilter {
    fn sent_recently(&self, task_id: &str, ext_id: &str) -> bool {
        self.recently_sent.contains(task_id, ext_id)
    }

    fn candidate_ids(&self, adapter: Option<&str>, ext_id: &str) -> Vec<String> {
        adapter
            .and_then(|provider_name| {
                self.adapters
                    .as_ref()
                    .and_then(|providers| providers.inbound_presentation_by_name(provider_name))
            })
            .map_or_else(
                || vec![ext_id.to_string()],
                |presentation| presentation.external_id_aliases(ext_id),
            )
    }
}
