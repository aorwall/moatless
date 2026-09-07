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

use super::api_key;
use super::api_keys::ApiKeyDao;

pub const PAIRING_CODE_TTL: Duration = Duration::minutes(5);
pub const PAIRED_DEVICE_KEY_TTL: Duration = Duration::days(90);

const FINISHED_CODE_RETENTION: Duration = Duration::days(1);
const CODE_ALPHABET: &[u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
const CODE_LENGTH: usize = 12;

#[derive(Debug, Clone)]
pub struct PairingCode {
    pub id: String,
    pub code_hash: String,
    pub user_id: String,
    pub group_id: Option<String>,
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[async_trait]
pub trait PairingCodeDao: Send + Sync {
    async fn insert_pairing_code(&self, code: &PairingCode) -> Result<(), sqlx::Error>;
    async fn consume_pairing_code(
        &self,
        code_hash: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<PairingCode>, sqlx::Error>;
    async fn delete_finished_pairing_codes(
        &self,
        cutoff: DateTime<Utc>,
    ) -> Result<u64, sqlx::Error>;
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

#[async_trait]
pub trait PairingApiKeyIssuer: Send + Sync {
    async fn issue_api_key(
        &self,
        user_id: &str,
        name: &str,
        scopes: Vec<String>,
        expires_at: DateTime<Utc>,
        group_id: Option<&str>,
    ) -> Result<PairedDevice, PairingError>;
}

pub struct DevicePairing {
    codes: Arc<dyn PairingCodeDao>,
    api_keys: Arc<dyn PairingApiKeyIssuer>,
}

impl DevicePairing {
    pub fn new(codes: Arc<dyn PairingCodeDao>, api_keys: Arc<dyn PairingApiKeyIssuer>) -> Self {
        Self { codes, api_keys }
    }

    pub async fn issue_code(
        &self,
        user_id: &str,
        group_id: Option<&str>,
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
                group_id: group_id.map(String::from),
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
            .issue_api_key(
                &claimed.user_id,
                &name,
                scopes,
                expires_at,
                claimed.group_id.as_deref(),
            )
            .await?;

        info!(
            user_id = %claimed.user_id,
            pairing_code_id = %claimed.id,
            key_id = %device.key_id,
            "Paired a device"
        );
        Ok(device)
    }
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

pub use super::pairing_dao::PgPairingCodeDao;

pub struct PgPairingApiKeyIssuer(pub Arc<dyn ApiKeyDao>);

#[async_trait]
impl PairingApiKeyIssuer for PgPairingApiKeyIssuer {
    async fn issue_api_key(
        &self,
        user_id: &str,
        name: &str,
        scopes: Vec<String>,
        expires_at: DateTime<Utc>,
        group_id: Option<&str>,
    ) -> Result<PairedDevice, PairingError> {
        let key = api_key::generate_key(
            self.0.as_ref(),
            user_id,
            name,
            Some(scopes.clone()),
            Some(expires_at),
            group_id,
        )
        .await
        .map_err(|error| PairingError::Storage(error.to_string()))?;

        Ok(PairedDevice {
            key_id: key.id,
            token: key.key,
            expires_at,
            scopes,
        })
    }
}
