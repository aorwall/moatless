//! The read seam the Binding domain publishes.
//!
//! An Adapter that only *asks about* Bindings reads [`BindingProbe`]. It is
//! declared in the Adapter kernel, which every consumer already names for the
//! Adapter contract; the Binding store implements it in the `moatless-loops`
//! crate, beside the table it reads. An Adapter that *drives* the Binding
//! lifecycle names that store directly — a narrow read port beside a store a
//! crate already holds is a second surface, not a seam.

use async_trait::async_trait;

use moatless_types::store::StoreError;

/// Whether a subject already has a Binding. Answers a yes/no; the rows behind
/// it stay in the crate that owns the table.
#[async_trait]
pub trait BindingProbe: Send + Sync {
    /// Whether any Binding tracks this subject.
    async fn is_bound(
        &self,
        adapter_kind: &str,
        subject_kind: &str,
        subject_key: &str,
    ) -> Result<bool, StoreError>;
}
