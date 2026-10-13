//! `moat set-name` and `moat set-tags`: a task renaming and tagging itself from
//! inside its own sandbox.

use crate::client::MoatClient;
use crate::config::Config;
use crate::output;
use moatless_api_schemas::task::UpdateTaskMetadataRequest;

pub async fn set_name(
    moat: &MoatClient,
    cfg: &Config,
    name: String,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let task_id = cfg.require_task_id()?;
    let req = UpdateTaskMetadataRequest {
        name: Some(name.clone()),
        description: None,
        model: None,
        agent_profile: None,
    };
    let task = moat.api().update_task_metadata(&task_id, &req).await?;
    output::emit(json, &task, || println!("Task name set to: {name}"));
    Ok(())
}

pub async fn set_tags(
    moat: &MoatClient,
    cfg: &Config,
    tags: Vec<String>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let task_id = cfg.require_task_id()?;
    let req = moatless_api_schemas::task::SetTagsRequest { tags: tags.clone() };
    moat.api().set_task_tags(&task_id, &req).await?;
    output::emit(json, &tags, || println!("Tags set: {}", tags.join(", ")));
    Ok(())
}
