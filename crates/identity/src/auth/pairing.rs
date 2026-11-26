//! Device pairing: the flow, and the adapters that persist it.
//!
//! A pairing code is traded for the API key a device then keeps. The flow is
//! storage-agnostic and reaches its two ports; the Postgres implementations of
//! those ports are below it, beside the table and the key service they use.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use rand::RngExt;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

pub const PAIRING_CODE_TTL: Duration = Duration::minutes(5);
/// How long a paired device's key lasts unused; each use renews it.
pub const PAIRED_DEVICE_KEY_TTL: Duration = Duration::days(90);

const FINISHED_CODE_RETENTION: Duration = Duration::days(1);
const CODE_ALPHABET: &[u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
const CODE_LENGTH: usize = 12;

#[derive(Debug, Clone)]
pub struct PairingCode {
    pub id: String,
    pub code_hash: String,
    pub user_id: String,
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct IssuedPairingCode {
    pub id: String,
    pub code: String,
    pub label: Option<String>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub struct PairingClient {
    pub label: Option<String>,
    pub os: Option<String>,
}

pub struct PairedDevice {
    pub key_id: String,
    pub token: String,
    pub expires_at: DateTime<Utc>,
    pub scopes: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum PairingError {
    #[error("The pairing code is not valid")]
    InvalidCode,
    #[error("Failed to redeem the pairing code: {0}")]
    Storage(String),
}

impl From<sqlx::Error> for PairingError {
    fn from(error: sqlx::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

/// A paired device's API key, as its owner sees it in order to revoke it.
#[derive(Debug, Clone)]
pub struct PairedDeviceKey {
    pub id: String,
    /// The key's name, which [`key_name`] built from the label and client OS.
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

/// The API keys pairing mints, and the owner's view of them.
///
/// A paired key is one with a sliding expiry (`api_keys.idle_ttl_secs`), which
/// only pairing sets; a key a person minted is never listed or revoked here.
#[async_trait]
pub trait PairedDeviceKeys: Send + Sync {
    async fn issue_api_key(
        &self,
        user_id: &str,
        name: &str,
        scopes: Vec<String>,
        expires_at: DateTime<Utc>,
    ) -> Result<PairedDevice, PairingError>;

    /// `user_id`'s paired keys that have not expired at `now`.
    async fn list_paired_keys(
        &self,
        user_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<PairedDeviceKey>, PairingError>;

    /// Deletes `key_id` if it is one of `user_id`'s paired keys.
    async fn delete_paired_key(&self, user_id: &str, key_id: &str) -> Result<bool, PairingError>;
}

pub struct DevicePairing {
    codes: Arc<dyn super::pairing_dao::PairingCodeDao>,
    api_keys: Arc<dyn PairedDeviceKeys>,
}

impl DevicePairing {
    pub(crate) fn new(
        codes: Arc<dyn super::pairing_dao::PairingCodeDao>,
        api_keys: Arc<dyn PairedDeviceKeys>,
    ) -> Self {
        Self { codes, api_keys }
    }

    pub async fn issue_code(
        &self,
        user_id: &str,
        label: Option<&str>,
    ) -> Result<IssuedPairingCode, PairingError> {
        let now = Utc::now();
        if let Err(error) = self
            .codes
            .delete_finished_pairing_codes(now - FINISHED_CODE_RETENTION)
            .await
        {
            warn!(%error, "Failed to prune finished device pairing codes");
        }

        let code = generate_code();
        let id = uuid::Uuid::new_v4().to_string();
        let label = label.map(str::trim).filter(|l| !l.is_empty());
        let expires_at = now + PAIRING_CODE_TTL;
        self.codes
            .insert_pairing_code(&PairingCode {
                id: id.clone(),
                code_hash: hash_key(&code),
                user_id: user_id.to_string(),
                label: label.map(String::from),
                created_at: now,
                expires_at,
            })
            .await?;

        info!(user_id = %user_id, pairing_code_id = %id, "Issued device pairing code");
        Ok(IssuedPairingCode {
            id,
            code,
            label: label.map(String::from),
            expires_at,
        })
    }

    pub async fn redeem_code(
        &self,
        code: &str,
        client: &PairingClient,
        scopes: Vec<String>,
    ) -> Result<PairedDevice, PairingError> {
        let now = Utc::now();
        let claimed = self
            .codes
            .consume_pairing_code(&hash_key(&normalize_code(code)), now)
            .await?
            .ok_or(PairingError::InvalidCode)?;

        let name = key_name(&claimed, client);
        let expires_at = now + PAIRED_DEVICE_KEY_TTL;
        let device = self
            .api_keys
            .issue_api_key(&claimed.user_id, &name, scopes, expires_at)
            .await?;

        info!(
            user_id = %claimed.user_id,
            pairing_code_id = %claimed.id,
            key_id = %device.key_id,
            "Paired a device"
        );
        Ok(device)
    }

    /// `user_id`'s codes that are neither redeemed nor expired.
    pub async fn pending_codes(&self, user_id: &str) -> Result<Vec<PairingCode>, PairingError> {
        Ok(self
            .codes
            .list_pending_pairing_codes(user_id, Utc::now())
            .await?)
    }

    /// Withdraws one of `user_id`'s unredeemed codes. False when there is none
    /// by that id, including one already redeemed.
    pub async fn revoke_code(&self, user_id: &str, code_id: &str) -> Result<bool, PairingError> {
        let revoked = self
            .codes
            .delete_pending_pairing_code(user_id, code_id)
            .await?;
        if revoked {
            info!(user_id = %user_id, pairing_code_id = %code_id, "Revoked device pairing code");
        }
        Ok(revoked)
    }

    /// `user_id`'s paired devices, by the key each one holds.
    pub async fn devices(&self, user_id: &str) -> Result<Vec<PairedDeviceKey>, PairingError> {
        self.api_keys.list_paired_keys(user_id, Utc::now()).await
    }

    /// Unpairs one of `user_id`'s devices by deleting its key.
    ///
    /// The backend's auth cache can still accept the key for up to
    /// `AUTH_CACHE_TTL_SECS`, and a socket the device already holds stays open
    /// until it reconnects; see `docs/deferred-changes.md`.
    pub async fn revoke_device(&self, user_id: &str, key_id: &str) -> Result<bool, PairingError> {
        let revoked = self.api_keys.delete_paired_key(user_id, key_id).await?;
        if revoked {
            info!(user_id = %user_id, key_id = %key_id, "Unpaired a device");
        }
        Ok(revoked)
    }

    /// Unpairs every one of `user_id`'s devices, as [`Self::revoke_device`]
    /// does one. Returns how many it unpaired.
    pub async fn revoke_all_devices(&self, user_id: &str) -> Result<usize, PairingError> {
        let mut revoked = 0;
        for device in self.devices(user_id).await? {
            if self.revoke_device(user_id, &device.id).await? {
                revoked += 1;
            }
        }
        Ok(revoked)
    }
}

/// A [`DevicePairing`] whose only redeemable code is `code`, for a consumer's
/// tests. Reaches no database. Pruning and issuing report success.
#[cfg(any(test, feature = "test-support"))]
pub fn device_pairing_over(
    code: Option<PairingCode>,
    api_keys: Arc<dyn PairedDeviceKeys>,
) -> DevicePairing {
    struct CannedCode(Option<PairingCode>);

    #[async_trait]
    impl super::pairing_dao::PairingCodeDao for CannedCode {
        async fn insert_pairing_code(&self, _code: &PairingCode) -> Result<(), sqlx::Error> {
            Ok(())
        }

        async fn consume_pairing_code(
            &self,
            code_hash: &str,
            _now: DateTime<Utc>,
        ) -> Result<Option<PairingCode>, sqlx::Error> {
            Ok(self
                .0
                .as_ref()
                .filter(|code| code.code_hash == code_hash)
                .cloned())
        }

        async fn delete_finished_pairing_codes(
            &self,
            _cutoff: DateTime<Utc>,
        ) -> Result<u64, sqlx::Error> {
            Ok(0)
        }

        async fn list_pending_pairing_codes(
            &self,
            user_id: &str,
            _now: DateTime<Utc>,
        ) -> Result<Vec<PairingCode>, sqlx::Error> {
            Ok(self
                .0
                .iter()
                .filter(|code| code.user_id == user_id)
                .cloned()
                .collect())
        }

        async fn delete_pending_pairing_code(
            &self,
            user_id: &str,
            code_id: &str,
        ) -> Result<bool, sqlx::Error> {
            Ok(self
                .0
                .as_ref()
                .is_some_and(|code| code.user_id == user_id && code.id == code_id))
        }
    }

    DevicePairing::new(Arc::new(CannedCode(code)), api_keys)
}

fn key_name(code: &PairingCode, client: &PairingClient) -> String {
    let chosen = code
        .label
        .as_deref()
        .or(client.label.as_deref())
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .unwrap_or("Paired device");
    match client
        .os
        .as_deref()
        .map(str::trim)
        .filter(|os| !os.is_empty())
    {
        Some(os) => format!("{chosen} ({os})"),
        None => chosen.to_string(),
    }
}

fn normalize_code(code: &str) -> String {
    code.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .flat_map(char::to_uppercase)
        .collect()
}

fn generate_code() -> String {
    let bytes: [u8; CODE_LENGTH] = rand::rng().random();
    bytes
        .iter()
        .map(|byte| CODE_ALPHABET[usize::from(byte % 32)] as char)
        .collect()
}

pub fn hash_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hex::encode(hasher.finalize())
}
