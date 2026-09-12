//! One function per UI-RPC method, grouped by the surface it answers for.
//!
//! [`routing`] holds both dispatch matches and no method body.

mod orchestration;
mod preview;
mod projects;
mod routing;
mod sandbox;
mod server;
mod streams;
mod terminal;
mod vcs;
