//! The task command surface, one child per subject: `moat tasks`, plus the
//! `set-name` and `set-tags` a task runs on itself.

mod create;
mod dispatch;
mod files;
mod format;
mod logs;
mod metadata;
mod resolve;
mod result;
mod sandbox;
mod scripts;

pub use dispatch::{TaskAction, run};
pub use files::FileAction;
pub use metadata::{set_name, set_tags};
pub use scripts::TaskScriptAction;
