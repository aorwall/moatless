mod contracts;
mod stdin;
pub(crate) mod subprocess;
#[cfg(test)]
pub(crate) mod mock;

pub(crate) use contracts::{Transport, StdinWriter};
pub(crate) use stdin::ChildStdinWriter;
pub(crate) use subprocess::SubprocessTransport;
