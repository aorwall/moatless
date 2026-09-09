//! Environment setup requirements — what's still missing before the app works.

pub mod evaluator;
pub mod github;
pub mod schemas;

pub use self::evaluator::{SetupContext, evaluate_setup};
