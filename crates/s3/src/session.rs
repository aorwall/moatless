//! Short-lived S3 credentials scoped to one key prefix.
//!
//! The counterpart to [`crate::presign`]: a presigned URL grants one operation
//! on one key, and this grants a whole prefix. A caller that needs to LIST has
//! no other option — a signature covers a single key, and a prefix listing is
//! not one.

use crate::S3Error;

/// The one hour AWS allows.
///
/// A session minted from the backend's own role is role chaining, which caps at
/// an hour whatever is asked for. It is also the floor a holder can work with:
/// the sandbox's credential refresher runs on a 900 s period, the STS minimum,
/// so a session that short would expire in step with the thing meant to renew
/// it.
const SESSION_SECONDS: i32 = 3600;

/// Credentials that reach only one prefix, and when they stop working.
#[derive(Debug, Clone)]
pub struct ScopedSession {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// Assume `role_arn`, narrowed by a session policy to read, write, delete and
/// list under `prefix` in `bucket`, plus read and list under `read_only_prefix`.
///
/// The narrowing is the session policy, not the role: the role stays
/// bucket-wide and every session is scoped, so adding a prefix needs no IAM
/// change. A session policy can only subtract, so a bug here fails closed.
pub async fn assume_role_for_prefix(
    client: &aws_sdk_sts::Client,
    role_arn: &str,
    session_name: &str,
    bucket: &str,
    prefix: &str,
    read_only_prefix: Option<&str>,
) -> Result<ScopedSession, S3Error> {
    let policy = prefix_policy(bucket, prefix, read_only_prefix);

    let response = client
        .assume_role()
        .role_arn(role_arn)
        .role_session_name(sanitize_session_name(session_name))
        .policy(policy)
        .duration_seconds(SESSION_SECONDS)
        .send()
        .await
        .map_err(|e| S3Error::AssumeRole(Box::new(e)))?;

    let credentials = response.credentials().ok_or(S3Error::SessionWithoutKeys)?;
    let expires_at = chrono::DateTime::from_timestamp(
        credentials.expiration().secs(),
        credentials.expiration().subsec_nanos(),
    )
    .ok_or(S3Error::SessionExpiryOutOfRange)?;

    Ok(ScopedSession {
        access_key_id: credentials.access_key_id().to_string(),
        secret_access_key: credentials.secret_access_key().to_string(),
        session_token: credentials.session_token().to_string(),
        expires_at,
    })
}

/// The session policy granting one writable prefix and one readable one.
///
/// `s3:ListBucket` is a bucket-level action, so it is granted on the bucket and
/// then held to the prefixes by a condition on the `prefix` query parameter.
/// Object-level permissions alone would let every read through and every
/// listing fail.
///
/// `read_only_prefix` gets `s3:GetObject` and listing and no write of any kind.
/// The caller pointing a session at a prefix it does not own is the whole reason
/// the two are separate statements rather than two entries on one.
fn prefix_policy(bucket: &str, prefix: &str, read_only_prefix: Option<&str>) -> String {
    // A leading or trailing slash would produce `//*`, which matches no key and
    // fails every listing.
    let prefix = prefix.trim_matches('/');
    let read_only_prefix = read_only_prefix.map(|p| p.trim_matches('/'));

    let listable: Vec<String> = std::iter::once(prefix)
        .chain(read_only_prefix)
        .map(|p| format!("{p}/*"))
        .collect();

    let mut statements = vec![
        serde_json::json!({
            "Sid": "ListRepositoryPrefix",
            "Effect": "Allow",
            "Action": "s3:ListBucket",
            "Resource": format!("arn:aws:s3:::{bucket}"),
            "Condition": {
                "StringLike": { "s3:prefix": listable }
            }
        }),
        serde_json::json!({
            "Sid": "ReadWriteRepositoryPrefix",
            "Effect": "Allow",
            "Action": ["s3:GetObject", "s3:PutObject", "s3:DeleteObject"],
            "Resource": format!("arn:aws:s3:::{bucket}/{prefix}/*")
        }),
    ];
    if let Some(read_only_prefix) = read_only_prefix {
        statements.push(serde_json::json!({
            "Sid": "ReadSourceRepositoryPrefix",
            "Effect": "Allow",
            "Action": "s3:GetObject",
            "Resource": format!("arn:aws:s3:::{bucket}/{read_only_prefix}/*")
        }));
    }

    serde_json::json!({ "Version": "2012-10-17", "Statement": statements }).to_string()
}

/// STS accepts `[\w+=,.@-]` and at most 64 characters in a session name, and
/// rejects the whole call otherwise. A task id is not held to that.
fn sanitize_session_name(name: &str) -> String {
    let mut cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '+' | '=' | ',' | '.' | '@' | '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    cleaned.truncate(64);
    if cleaned.is_empty() {
        "moatless".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_policy_lists_by_prefix_condition_and_reads_by_resource() {
        let policy = prefix_policy("bucket", "workspaces/task-1/repo", None);
        let parsed: serde_json::Value = serde_json::from_str(&policy).unwrap();
        let statements = parsed["Statement"].as_array().unwrap();

        assert_eq!(statements.len(), 2);
        assert_eq!(statements[0]["Action"], "s3:ListBucket");
        assert_eq!(statements[0]["Resource"], "arn:aws:s3:::bucket");
        assert_eq!(
            statements[0]["Condition"]["StringLike"]["s3:prefix"][0],
            "workspaces/task-1/repo/*"
        );
        assert_eq!(
            statements[1]["Resource"],
            "arn:aws:s3:::bucket/workspaces/task-1/repo/*"
        );
    }

    /// The whole point of the second prefix: a fork reads its source's
    /// repository and must not be able to write over it.
    #[test]
    fn a_read_only_prefix_is_listable_and_gettable_and_nothing_else() {
        let policy = prefix_policy(
            "bucket",
            "workspaces/fork-1/repo",
            Some("workspaces/task-1/repo"),
        );
        let parsed: serde_json::Value = serde_json::from_str(&policy).unwrap();
        let statements = parsed["Statement"].as_array().unwrap();

        assert_eq!(
            statements[0]["Condition"]["StringLike"]["s3:prefix"],
            serde_json::json!(["workspaces/fork-1/repo/*", "workspaces/task-1/repo/*"])
        );
        assert_eq!(
            statements[1]["Resource"],
            "arn:aws:s3:::bucket/workspaces/fork-1/repo/*"
        );
        assert_eq!(statements[2]["Action"], "s3:GetObject");
        assert_eq!(
            statements[2]["Resource"],
            "arn:aws:s3:::bucket/workspaces/task-1/repo/*"
        );
        assert_eq!(statements.len(), 3);
    }

    /// A prefix that arrived with slashes must not produce `//*`, which matches
    /// nothing and would fail every listing.
    #[test]
    fn a_prefix_is_scoped_the_same_however_its_slashes_are_written() {
        let bare = prefix_policy(
            "bucket",
            "workspaces/task-1/repo",
            Some("workspaces/x/repo"),
        );
        assert_eq!(
            bare,
            prefix_policy(
                "bucket",
                "/workspaces/task-1/repo/",
                Some("/workspaces/x/repo/")
            )
        );

        // A non-fork trims too: nothing else covers the `None` arm's own prefix.
        assert_eq!(
            prefix_policy("bucket", "workspaces/task-1/repo", None),
            prefix_policy("bucket", "/workspaces/task-1/repo/", None)
        );
    }

    #[test]
    fn a_session_name_is_reduced_to_what_sts_accepts() {
        assert_eq!(sanitize_session_name("task/with space"), "task-with-space");
        assert_eq!(sanitize_session_name("").len(), 8);
        assert_eq!(sanitize_session_name(&"x".repeat(200)).len(), 64);
    }
}
