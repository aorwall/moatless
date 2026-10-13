//! Owner-reference construction for cascading K8s garbage collection.
//!
//! A sandbox's Service and Traefik `IngressRoutes` are children of the task's
//! Deployment via `metadata.ownerReferences`: deleting the Deployment cascades
//! GC to every dependent (including the Traefik CRD), so a failure
//! mid-`cleanup()` cannot orphan one.
//!
//! The PVC is deliberately NOT owner-referenced: it must survive a
//! stop/redeploy, which deletes the Deployment but keeps workspace data.

use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;

/// Build an `OwnerReference` pointing a dependent at its owning Deployment.
///
/// `controller` is `false` (no controller manages these dependents) and
/// `block_owner_deletion` is unset, so the Deployment delete returns promptly
/// and GC cleans up dependents asynchronously.
pub fn deployment_owner_reference(deployment_name: &str, deployment_uid: &str) -> OwnerReference {
    OwnerReference {
        api_version: "apps/v1".to_string(),
        kind: "Deployment".to_string(),
        name: deployment_name.to_string(),
        uid: deployment_uid.to_string(),
        controller: Some(false),
        block_owner_deletion: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_deployment_owner_reference() {
        let owner = deployment_owner_reference("moatless-ws-abc", "uid-123");
        assert_eq!(owner.api_version, "apps/v1");
        assert_eq!(owner.kind, "Deployment");
        assert_eq!(owner.name, "moatless-ws-abc");
        assert_eq!(owner.uid, "uid-123");
        assert_eq!(owner.controller, Some(false));
        assert_eq!(owner.block_owner_deletion, None);
    }

    #[test]
    fn serializes_into_metadata_owner_references_array() {
        // The create paths inject the owner reference into a `metadata` JSON
        // object; verify it serializes to the shape the K8s API expects.
        let owner = deployment_owner_reference("moatless-ws-abc", "uid-123");
        let json = serde_json::json!({ "ownerReferences": [owner] });
        let arr = json["ownerReferences"].as_array().expect("array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["apiVersion"], "apps/v1");
        assert_eq!(arr[0]["kind"], "Deployment");
        assert_eq!(arr[0]["name"], "moatless-ws-abc");
        assert_eq!(arr[0]["uid"], "uid-123");
        assert_eq!(arr[0]["controller"], false);
        // block_owner_deletion is None → omitted from the serialized output.
        assert!(arr[0].get("blockOwnerDeletion").is_none());
    }
}
