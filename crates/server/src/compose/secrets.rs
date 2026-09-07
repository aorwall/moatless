use std::sync::Arc;

use moatless_core::{
    config::Config,
    db::{self, DbPool},
};
use moatless_identity::secret::{
    audit::PgSecretAuditDao,
    dao::PgSecretDao,
    service::{LocalEnvelopeSecretCipher, SecretService},
};

pub(crate) async fn build(
    config: &Config,
) -> Result<(DbPool, SecretService), Box<dyn std::error::Error>> {
    let url = config
        .database_url
        .as_deref()
        .ok_or("DATABASE_URL must be set for secrets CLI")?;
    let pool = db::create_pool(url, config.database.max_connections).await?;
    let secrets_repo = Arc::new(PgSecretDao(pool.clone()));
    let audit_repo = Arc::new(PgSecretAuditDao(pool.clone()));
    let cipher = LocalEnvelopeSecretCipher::from_config(&config.secrets)
        .ok_or("SECRETS_ENCRYPTION_KEY must be set (base64 or raw 32 bytes) for secrets CLI")?;
    let service = SecretService::new(secrets_repo, audit_repo, Some(cipher));
    Ok((pool, service))
}
