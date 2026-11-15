//! One function per UI-RPC method, grouped by the surface it answers for. Each
//! group registers its methods in a
//! [`MethodTable`](crate::rpc::table::MethodTable); [`methods()`] merges them.

mod artifacts;
mod methods;
mod orchestration;
mod orchestration_v2;
mod preview;
mod projects;
mod sandbox;
mod secrets;
mod server;
mod streams;
mod terminal;
mod thread_reads;
mod vcs;

pub(crate) use methods::methods;
