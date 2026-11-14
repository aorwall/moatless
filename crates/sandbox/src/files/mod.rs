//! Filesystem and search RPCs served over gRPC.

pub mod content_search;
pub mod git;
pub mod read_write;
pub mod review;
pub mod search;
pub mod tree;
pub mod uploads;

mod error;
mod path;
pub use error::FileError;
pub use path::validate_path;
