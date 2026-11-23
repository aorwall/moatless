//! Classifying the errors PostgreSQL returns.

/// Whether `error` is PostgreSQL cancelling a statement (`57014`), which is
/// what a query's own `statement_timeout` raises when it runs out.
pub fn is_statement_timeout(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(error) if error.code().as_deref() == Some("57014"))
}
