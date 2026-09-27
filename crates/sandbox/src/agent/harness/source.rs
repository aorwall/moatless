//! Producer role for a content block read back from a session file.

/// The producer role of a native content block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Assistant,
    User,
    System,
}
