use super::dao::PluginDao;
use super::error::PluginError;
use super::models::Plugin;
use super::schemas::CreatePluginRequest;
use moatless_types::access_control::ScopeContext;

pub(super) fn ensure_admin(auth: &ScopeContext, action: &str) -> Result<(), PluginError> {
    if auth.is_admin {
        Ok(())
    } else {
        Err(PluginError::Forbidden(format!(
            "Admin role required to {action}"
        )))
    }
}

/// Mutate the plugin registry after checking administrator access.
pub async fn create_plugin(
    plugins: &dyn PluginDao,
    auth: &ScopeContext,
    req: CreatePluginRequest,
) -> Result<Plugin, PluginError> {
    ensure_admin(auth, "manage the plugin registry")?;
    let id = uuid::Uuid::new_v4().to_string();
    let row = Plugin {
        id: id.clone(),
        name: req.name,
        git_url: req.git_url,
        synced_version: None,
        synced_at: None,
        created_at: None,
        updated_at: None,
    };

    if let Err(e) = plugins.insert(&row).await {
        // Check for unique constraint violation on name
        if let sqlx::Error::Database(ref db_err) = e {
            if db_err.constraint().is_some() {
                return Err(PluginError::Conflict(
                    "A plugin with that name already exists".to_string(),
                ));
            }
        }
        return Err(e.into());
    }

    Ok(row)
}

/// Mutate the plugin registry after checking administrator access.
pub async fn delete_plugin(
    plugins: &dyn PluginDao,
    auth: &ScopeContext,
    id: String,
) -> Result<(), PluginError> {
    ensure_admin(auth, "manage the plugin registry")?;
    let deleted = plugins.delete(&id).await?;
    if !deleted {
        return Err(PluginError::NotFound(format!("Plugin '{id}' not found")));
    }
    Ok(())
}
