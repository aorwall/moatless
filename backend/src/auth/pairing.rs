//! Device pairing — how a client on another device comes to act as a person.
//!
//! A browser proves who it is with a session cookie, and a script with an API
//! key it was given by hand. A client on a phone can do neither: it has no
//! browser to log in with, and asking someone to type a 64-character key into
//! one is not a flow anybody completes. So it is given a short code instead.
//!
//! The two credentials are deliberately different things. The **code** is
//! bootstrap: short, typeable, single-use, alive for minutes, and worth nothing
//! once exchanged. The **key** is the session: opaque, long, durable, and the
//! only thing the client keeps. A code never becomes the credential a device
//! holds, which is what keeps something short enough to read aloud from being
//! something worth stealing.
//!
//! The key an exchange mints is an ordinary Moatless API key. That is the whole
//! reason this module is small: `Authorization: Bearer mvk_…` is already
//! accepted everywhere by [`crate::auth::middleware`], paired devices appear in
//! the API-keys list a person already has, and revoking one there is what
//! un-pairs the device. Nothing here needed a second credential type.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use rand::RngExt;
use tracing::{info, warn};

use super::api_key::{self, hash_key};
use super::api_keys::ApiKeyDao;
use super::error::AuthError;
use crate::db::DbPool;

/// How long a freshly minted code can still be exchanged.
///
/// Long enough to walk to the other device and type it, short enough that a
/// code read over a shoulder is usually already dead. Matches what T3 Code's
/// own server allows for the same flow.
pub const PAIRING_CODE_TTL: Duration = Duration::minutes(5);

/// How long the key an exchange mints stays valid.
///
/// A paired device has no way to renew: the code is spent, and the client holds
/// exactly one credential with no refresh flow behind it. So this is the
/// interval at which someone has to pair again, and it is bounded rather than
/// endless because the alternative is a phone that keeps full account access
/// forever after it is lost.
pub const PAIRED_DEVICE_KEY_TTL: Duration = Duration::days(90);

/// Finished codes are deleted once they are this far past expiry.
///
/// Not a retention policy — just enough of a delay that a person who exchanged
/// a code seconds ago can still be told *why* a second attempt failed by the
/// row still being there.
const FINISHED_CODE_RETENTION: Duration = Duration::days(1);

/// Characters a code is built from.
///
/// Digits and uppercase letters minus `0`, `1`, `I` and `O`: the pairs someone
/// reading a code off one screen and typing it into another confuses. Exactly
/// 32 characters, which is also what makes [`generate_code`] unbiased.
const CODE_ALPHABET: &[u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";

/// Characters per code. 12 of a 32-character alphabet is 60 bits, against a
/// code that is single-use and lives five minutes.
const CODE_LENGTH: usize = 12;

/// A minted pairing code, before anyone has exchanged it.
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

/// DAO trait for pairing code persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait PairingCodeDao: Send + Sync {
    async fn insert_pairing_code(&self, code: &PairingCode) -> Result<(), sqlx::Error>;

    /// Claim a code by hash, atomically. Returns the row only if this call is
    /// the one that consumed it: not previously consumed, and not expired at
    /// `now`.
    async fn consume_pairing_code(
        &self,
        code_hash: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<PairingCode>, sqlx::Error>;

    /// Delete codes that expired before `cutoff`, consumed or not.
    async fn delete_finished_pairing_codes(&self, cutoff: DateTime<Utc>)
        -> Result<u64, sqlx::Error>;
}

/// PostgreSQL implementation of [`PairingCodeDao`].
pub struct PgPairingCodeDao(pub DbPool);

#[async_trait]
impl PairingCodeDao for PgPairingCodeDao {
    async fn insert_pairing_code(&self, code: &PairingCode) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO device_pairing_codes
                (id, code_hash, user_id, group_id, label, created_at, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
        )
        .bind(&code.id)
        .bind(&code.code_hash)
        .bind(&code.user_id)
        .bind(&code.group_id)
        .bind(&code.label)
        .bind(code.created_at)
        .bind(code.expires_at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn consume_pairing_code(
        &self,
        code_hash: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<PairingCode>, sqlx::Error> {
        // One statement, so two backends racing the same code produce one
        // winner: the loser's UPDATE matches no row and returns nothing.
        sqlx::query_as::<_, (String, String, String, Option<String>, Option<String>, DateTime<Utc>, DateTime<Utc>)>(
            r#"
            UPDATE device_pairing_codes
               SET consumed_at = $2
             WHERE code_hash = $1
               AND consumed_at IS NULL
               AND expires_at > $2
            RETURNING id, code_hash, user_id, group_id, label, created_at, expires_at
            "#,
        )
        .bind(code_hash)
        .bind(now)
        .fetch_optional(&self.0)
        .await
        .map(|row| {
            row.map(
                |(id, code_hash, user_id, group_id, label, created_at, expires_at)| PairingCode {
                    id,
                    code_hash,
                    user_id,
                    group_id,
                    label,
                    created_at,
                    expires_at,
                },
            )
        })
    }

    async fn delete_finished_pairing_codes(
        &self,
        cutoff: DateTime<Utc>,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query("DELETE FROM device_pairing_codes WHERE expires_at < $1")
            .bind(cutoff)
            .execute(&self.0)
            .await?;
        Ok(result.rows_affected())
    }
}

/// A code, in the one moment it is readable.
pub struct IssuedPairingCode {
    pub id: String,
    /// The plaintext. Returned to the person who minted it and never stored.
    pub code: String,
    pub label: Option<String>,
    pub expires_at: DateTime<Utc>,
}

/// What a client says about itself while exchanging a code.
///
/// All of it is unverified, and none of it decides anything: it only names the
/// key in a list so its owner can tell which device to revoke.
#[derive(Debug, Default)]
pub struct PairingClient {
    pub label: Option<String>,
    pub os: Option<String>,
}

/// The credential a device leaves the exchange holding.
pub struct PairedDevice {
    pub key_id: String,
    pub token: String,
    pub expires_at: DateTime<Utc>,
    pub scopes: Vec<String>,
}

/// Why an exchange did not produce a credential.
#[derive(Debug, thiserror::Error)]
pub enum PairingError {
    /// No code matched: never minted, already exchanged, or expired. The three
    /// are deliberately one answer — telling them apart tells someone guessing
    /// codes which guesses were close.
    #[error("The pairing code is not valid")]
    InvalidCode,

    #[error("Failed to redeem the pairing code")]
    Storage(#[source] AuthError),
}

impl From<sqlx::Error> for PairingError {
    fn from(error: sqlx::Error) -> Self {
        Self::Storage(AuthError::Database(error))
    }
}

/// Minting and exchanging pairing codes.
///
/// Holds both sides of the trade — the codes it mints, and the API keys it
/// turns them into — because the exchange is one operation and splitting it
/// would leave a consumed code with no key to show for it.
pub struct DevicePairing {
    codes: Arc<dyn PairingCodeDao>,
    api_keys: Arc<dyn ApiKeyDao>,
}

impl DevicePairing {
    pub fn new(codes: Arc<dyn PairingCodeDao>, api_keys: Arc<dyn ApiKeyDao>) -> Self {
        Self { codes, api_keys }
    }

    /// Mint a code for a signed-in person.
    ///
    /// The caller has already been authenticated; this does not check anything
    /// about them, it binds the code to them.
    pub async fn issue_code(
        &self,
        user_id: &str,
        group_id: Option<&str>,
        label: Option<&str>,
    ) -> Result<IssuedPairingCode, AuthError> {
        let now = Utc::now();
        // Best effort, and deliberately not fatal: a code nobody can prune is
        // still a code that works.
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

    /// Exchange a code for the key a device keeps.
    ///
    /// The code is consumed first and the key minted second. That order is what
    /// makes a replayed code useless; its cost is that a failure to mint the key
    /// spends the code, which is why the caller is told to ask for a new one
    /// rather than retry the same exchange.
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
        let key = api_key::generate_key(
            self.api_keys.as_ref(),
            &claimed.user_id,
            &name,
            Some(scopes.clone()),
            Some(expires_at),
            claimed.group_id.as_deref(),
        )
        .await
        .map_err(PairingError::Storage)?;

        info!(
            user_id = %claimed.user_id,
            pairing_code_id = %claimed.id,
            key_id = %key.id,
            "Paired a device"
        );
        Ok(PairedDevice {
            key_id: key.id,
            token: key.key,
            expires_at,
            scopes,
        })
    }
}

/// What the paired device is called in the API-keys list.
///
/// The name the person chose when they minted the code wins over anything the
/// client says about itself: they picked it to recognize the device by, and the
/// client's own label is the same string on every install of it.
fn key_name(code: &PairingCode, client: &PairingClient) -> String {
    let chosen = code
        .label
        .as_deref()
        .or(client.label.as_deref())
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .unwrap_or("Paired device");
    match client.os.as_deref().map(str::trim).filter(|os| !os.is_empty()) {
        Some(os) => format!("{chosen} ({os})"),
        None => chosen.to_string(),
    }
}

/// Read a typed code the way a person meant it.
///
/// Case is not part of a code — the alphabet is uppercase — so a client that
/// sends what someone typed in lower case is sending the same code. Interior
/// spacing is not either: a code shown in groups may well be typed back that
/// way.
fn normalize_code(code: &str) -> String {
    code.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .flat_map(char::to_uppercase)
        .collect()
}

/// A fresh code.
///
/// `byte % 32` is uniform here rather than nearly so: 256 is a whole number of
/// alphabets, so no byte value maps to a character any other value does not.
fn generate_code() -> String {
    let bytes: [u8; CODE_LENGTH] = rand::rng().random();
    bytes
        .iter()
        .map(|byte| CODE_ALPHABET[usize::from(byte % 32)] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_is_typeable() {
        let code = generate_code();
        assert_eq!(code.len(), CODE_LENGTH);
        assert!(
            code.bytes().all(|b| CODE_ALPHABET.contains(&b)),
            "code {code} left the alphabet"
        );
        // The characters a reader confuses are the ones that are not there.
        assert!(!code.contains(['0', '1', 'I', 'O']));
    }

    #[test]
    fn two_codes_differ() {
        assert_ne!(generate_code(), generate_code());
    }

    #[test]
    fn a_code_is_read_the_way_it_was_typed() {
        assert_eq!(normalize_code("abcd efgh jklm"), "ABCDEFGHJKLM");
        assert_eq!(normalize_code("ABCD-EFGH-JKLM"), "ABCDEFGHJKLM");
        assert_eq!(normalize_code("  AbCd  "), "ABCD");
    }

    #[test]
    fn the_name_the_person_chose_wins() {
        let code = PairingCode {
            id: "id".into(),
            code_hash: "hash".into(),
            user_id: "user".into(),
            group_id: None,
            label: Some("Bedside phone".into()),
            created_at: Utc::now(),
            expires_at: Utc::now(),
        };
        let client = PairingClient {
            label: Some("T3 Code Mobile".into()),
            os: Some("iOS".into()),
        };
        assert_eq!(key_name(&code, &client), "Bedside phone (iOS)");

        let unlabelled = PairingCode {
            label: None,
            ..code.clone()
        };
        assert_eq!(key_name(&unlabelled, &client), "T3 Code Mobile (iOS)");
        assert_eq!(
            key_name(&unlabelled, &PairingClient::default()),
            "Paired device"
        );
    }
}
