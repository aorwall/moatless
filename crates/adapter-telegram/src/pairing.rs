//! Telegram pairing — code-based DM access approval.
#![expect(
    dead_code,
    reason = "the approval half of pairing (look a code up, list what is \
              pending, approve it, prune) has no caller since this module \
              stopped being `pub`: it was reachable only from outside the \
              Adapter, and nothing outside calls it. Kept rather than deleted \
              because pairing without approval is a dead end for the user who \
              was just handed a code; deleting it is the pairing UI's call, \
              not this breakup's."
)]

mod approval;
mod dao;
mod models;
mod request;

pub use request::upsert_pairing_request;
