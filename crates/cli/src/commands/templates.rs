//! Task template commands: list, get, create, update, delete.

use clap::Subcommand;
use moatless_api_schemas::task_template::{
    CreateTaskTemplateRequest, TaskTemplateResponse, UpdateTaskTemplateRequest,
};

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum TemplateAction {
    /// List task templates
    List {
        /// Filter by scope: user, team, or global
        #[arg(long)]
        scope: Option<String>,
        /// Show only featured templates
        #[arg(long)]
        featured: bool,
    },
    /// Get task template details
    Get { template_id: String },
    /// Create a task template (use -f/--file for YAML or JSON)
    Create {
        /// YAML or JSON file with template fields (extension selects parser)
        #[arg(long, short = 'f')]
        file: Option<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        repository_id: Option<String>,
        #[arg(long)]
        agent_type: Option<String>,
        #[arg(long)]
        agent_mode: Option<String>,
        #[arg(long)]
        default_model: Option<String>,
        #[arg(long)]
        initial_prompt: Option<String>,
        #[arg(long)]
        icon: Option<String>,
        #[arg(long)]
        color: Option<String>,
        /// Skill name (repeatable: --skill testing-guidelines --skill frontend-dev-guidelines)
        #[arg(long = "skill")]
        skills: Vec<String>,
        /// Scope: user, team, or global
        #[arg(long)]
        scope: Option<String>,
        #[arg(long)]
        team_id: Option<String>,
        #[arg(long)]
        featured: bool,
        #[arg(long)]
        display_order: Option<i32>,
    },
    /// Update a task template
    Update {
        template_id: String,
        /// YAML or JSON file with template fields (extension selects parser)
        #[arg(long, short = 'f')]
        file: Option<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        repository_id: Option<String>,
        #[arg(long)]
        agent_type: Option<String>,
        #[arg(long)]
        agent_mode: Option<String>,
        #[arg(long)]
        default_model: Option<String>,
        #[arg(long)]
        initial_prompt: Option<String>,
        #[arg(long)]
        icon: Option<String>,
        #[arg(long)]
        color: Option<String>,
        /// Skill name (repeatable)
        #[arg(long = "skill")]
        skills: Vec<String>,
        /// Scope: user, team, or global
        #[arg(long)]
        scope: Option<String>,
        #[arg(long)]
        team_id: Option<String>,
        #[arg(long)]
        featured: Option<bool>,
        #[arg(long)]
        display_order: Option<i32>,
    },
    /// Delete a task template
    Delete {
        template_id: String,
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
}

pub async fn run(
    client: &MoatClient,
    action: TemplateAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        TemplateAction::List { scope: _, featured } => {
            let templates = if featured {
                client.api().list_featured_task_templates(6).await?
            } else {
                client.api().list_task_templates().await?
            };
            if json {
                output::print_json_typed(&templates);
            } else {
                format_template_list(&templates);
            }
        }

        TemplateAction::Get { template_id } => {
            let template = client.api().get_task_template(&template_id).await?;
            if json {
                output::print_json_typed(&template);
            } else {
                format_template_detail(&template);
            }
        }

        TemplateAction::Create {
            file,
            name,
            description,
            repository_id,
            agent_type,
            agent_mode,
            default_model,
            initial_prompt,
            icon,
            color,
            skills,
            scope,
            team_id,
            featured,
            display_order,
        } => {
            let req = load_create_request(
                file.as_deref(),
                name,
                description,
                repository_id,
                agent_type,
                agent_mode,
                default_model,
                initial_prompt,
                icon,
                color,
                skills,
                scope,
                team_id,
                featured,
                display_order,
            )?;
            let created = client.api().create_task_template(&req).await?;
            if json {
                output::print_json_typed(&created);
            } else {
                println!("Task template created: {}", created.id);
            }
        }

        TemplateAction::Update {
            template_id,
            file,
            name,
            description,
            repository_id,
            agent_type,
            agent_mode,
            default_model,
            initial_prompt,
            icon,
            color,
            skills,
            scope,
            team_id,
            featured,
            display_order,
        } => {
            let req = load_update_request(
                file.as_deref(),
                name,
                description,
                repository_id,
                agent_type,
                agent_mode,
                default_model,
                initial_prompt,
                icon,
                color,
                skills,
                scope,
                team_id,
                featured,
                display_order,
            )?;
            let updated = client
                .api()
                .update_task_template(&template_id, &req)
                .await?;
            if json {
                output::print_json_typed(&updated);
            } else {
                println!("Task template {} updated.", updated.id);
            }
        }

        TemplateAction::Delete { template_id, yes } => {
            let template = client.api().get_task_template(&template_id).await?;
            let name = template.name.clone();
            let id = template.id.clone();

            if !yes {
                use std::io::{IsTerminal, Write};
                if !std::io::stdin().is_terminal() {
                    return Err(ClientError::Other(
                        "Refusing to delete without --yes in non-interactive mode".to_string(),
                    ));
                }
                print!("Delete template '{name}' ({id})? This cannot be undone. [y/N] ");
                std::io::stdout()
                    .flush()
                    .map_err(|e| ClientError::Other(format!("Failed to flush stdout: {e}")))?;
                let mut line = String::new();
                std::io::stdin()
                    .read_line(&mut line)
                    .map_err(|e| ClientError::Other(format!("Failed to read stdin: {e}")))?;
                let answer = line.trim().to_lowercase();
                if answer != "y" && answer != "yes" {
                    return Err(ClientError::Other("Aborted.".to_string()));
                }
            }

            client.api().delete_task_template(&id).await?;
            if json {
                output::print_json_typed(&serde_json::json!({ "id": id, "deleted": true }));
            } else {
                println!("Deleted template {name} ({id})");
            }
        }
    }
    Ok(())
}

// =============================================================================
// Helpers
// =============================================================================

#[allow(clippy::too_many_arguments)]
fn load_create_request(
    file: Option<&str>,
    name: Option<String>,
    description: Option<String>,
    repository_id: Option<String>,
    agent_type: Option<String>,
    agent_mode: Option<String>,
    default_model: Option<String>,
    initial_prompt: Option<String>,
    icon: Option<String>,
    color: Option<String>,
    skills: Vec<String>,
    scope: Option<String>,
    team_id: Option<String>,
    featured: bool,
    display_order: Option<i32>,
) -> Result<CreateTaskTemplateRequest, ClientError> {
    if let Some(path) = file {
        let content = std::fs::read_to_string(path)
            .map_err(|e| ClientError::Other(format!("Failed to read file '{path}': {e}")))?;

        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let req: CreateTaskTemplateRequest = if ext == "yaml" || ext == "yml" {
            serde_yaml::from_str(&content)
                .map_err(|e| ClientError::Other(format!("Invalid YAML in '{path}': {e}")))?
        } else {
            serde_json::from_str(&content)
                .map_err(|e| ClientError::Other(format!("Invalid JSON in '{path}': {e}")))?
        };
        return Ok(req);
    }

    // Build from flags — validate required fields.
    let name = name.ok_or_else(|| {
        ClientError::Other("--name is required when not using --file".to_string())
    })?;
    let repository_id = repository_id.ok_or_else(|| {
        ClientError::Other("--repository-id is required when not using --file".to_string())
    })?;
    let agent_type = agent_type.ok_or_else(|| {
        ClientError::Other("--agent-type is required when not using --file".to_string())
    })?;

    Ok(CreateTaskTemplateRequest {
        name,
        description,
        icon,
        color,
        repository_id,
        agent_type,
        agent_mode,
        default_model,
        initial_prompt,
        skills: if skills.is_empty() {
            None
        } else {
            Some(skills)
        },
        scope: scope.as_deref().and_then(parse_scope),
        team_id,
        featured: if featured { Some(true) } else { None },
        display_order,
    })
}

#[allow(clippy::too_many_arguments)]
fn load_update_request(
    file: Option<&str>,
    name: Option<String>,
    description: Option<String>,
    repository_id: Option<String>,
    agent_type: Option<String>,
    agent_mode: Option<String>,
    default_model: Option<String>,
    initial_prompt: Option<String>,
    icon: Option<String>,
    color: Option<String>,
    skills: Vec<String>,
    scope: Option<String>,
    team_id: Option<String>,
    featured: Option<bool>,
    display_order: Option<i32>,
) -> Result<UpdateTaskTemplateRequest, ClientError> {
    if let Some(path) = file {
        let content = std::fs::read_to_string(path)
            .map_err(|e| ClientError::Other(format!("Failed to read file '{path}': {e}")))?;

        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let req: UpdateTaskTemplateRequest = if ext == "yaml" || ext == "yml" {
            serde_yaml::from_str(&content)
                .map_err(|e| ClientError::Other(format!("Invalid YAML in '{path}': {e}")))?
        } else {
            serde_json::from_str(&content)
                .map_err(|e| ClientError::Other(format!("Invalid JSON in '{path}': {e}")))?
        };
        return Ok(req);
    }

    Ok(UpdateTaskTemplateRequest {
        name,
        description,
        icon,
        color,
        repository_id,
        agent_type,
        agent_mode,
        default_model,
        initial_prompt,
        skills: if skills.is_empty() {
            None
        } else {
            Some(skills)
        },
        scope: scope.as_deref().and_then(parse_scope),
        team_id,
        featured,
        display_order,
    })
}

fn parse_scope(s: &str) -> Option<moatless_api_schemas::enums::Scope> {
    use moatless_api_schemas::enums::Scope;
    match s {
        "global" => Some(Scope::Global),
        "team" => Some(Scope::Team),
        "user" => Some(Scope::User),
        _ => None,
    }
}

fn format_template_list(templates: &[TaskTemplateResponse]) {
    let rows: Vec<Vec<String>> = templates
        .iter()
        .map(|t| {
            vec![
                t.id.clone(),
                t.name.clone(),
                t.scope.to_string(),
                t.featured.to_string(),
                t.display_order.to_string(),
                t.agent_type.clone(),
            ]
        })
        .collect();

    output::print_table(
        &["ID", "NAME", "SCOPE", "FEATURED", "ORDER", "AGENT"],
        &rows,
    );
}

fn format_template_detail(t: &TaskTemplateResponse) {
    println!("ID:           {}", t.id);
    println!("Name:         {}", t.name);
    if let Some(ref desc) = t.description {
        println!("Description:  {desc}");
    }
    println!("Repository:   {}", t.repository_id);
    println!("Agent type:   {}", t.agent_type);
    if let Some(ref mode) = t.agent_mode {
        println!("Agent mode:   {mode}");
    }
    if let Some(ref model) = t.default_model {
        println!("Model:        {model}");
    }
    println!("Scope:        {}", t.scope);
    if let Some(ref tid) = t.team_id {
        println!("Team ID:      {tid}");
    }
    println!("Featured:     {}", t.featured);
    println!("Display order:{}", t.display_order);
    if let Some(ref icon) = t.icon {
        println!("Icon:         {icon}");
    }
    if let Some(ref color) = t.color {
        println!("Color:        {color}");
    }
    if let Some(ref skills) = t.skills {
        if !skills.is_empty() {
            println!("Skills:       {}", skills.join(", "));
        }
    }
    if let Some(ref prompt) = t.initial_prompt {
        let display = if prompt.len() > 120 {
            format!("{}…", &prompt[..120])
        } else {
            prompt.clone()
        };
        println!("Initial prompt:\n{display}");
    }
    if let Some(ref cb) = t.created_by {
        println!("Created by:   {cb}");
    }
    println!("Created at:   {}", t.created_at);
}
