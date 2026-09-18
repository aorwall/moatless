//! Agent catalog commands: list agents and models.

use clap::Subcommand;
use moatless_api_schemas::agent::ListAgentsResponse;

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum AgentAction {
    /// List agent harnesses with their model IDs
    List,
    /// List available models grouped by agent harness
    Models,
}

pub async fn run(client: &MoatClient, action: AgentAction, json: bool) -> Result<(), ClientError> {
    let agents = client.api().list_agents().await?;
    if json {
        output::print_json_typed(&agents);
        return Ok(());
    }

    match action {
        AgentAction::List => format_agent_list(&agents),
        AgentAction::Models => format_agent_models(&agents),
    }

    Ok(())
}

fn format_agent_list(resp: &ListAgentsResponse) {
    let rows: Vec<Vec<String>> = resp
        .agents
        .iter()
        .map(|agent| {
            vec![
                agent.id.clone(),
                agent.label.clone(),
                installed_label(agent.installed).to_string(),
                agent.default_model.clone(),
                agent
                    .models
                    .iter()
                    .map(|model| model.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            ]
        })
        .collect();

    output::print_table(&["AGENT", "LABEL", "INSTALLED", "DEFAULT", "MODELS"], &rows);
}

fn format_agent_models(resp: &ListAgentsResponse) {
    let rows = agent_model_rows(resp);
    output::print_table(&["AGENT", "MODEL", "LABEL", "DEFAULT", "INSTALLED"], &rows);
}

fn agent_model_rows(resp: &ListAgentsResponse) -> Vec<Vec<String>> {
    resp.agents
        .iter()
        .flat_map(|agent| {
            agent.models.iter().map(move |model| {
                vec![
                    agent.id.clone(),
                    model.id.clone(),
                    model.label.clone(),
                    if model.id == agent.default_model {
                        "yes".to_string()
                    } else {
                        String::new()
                    },
                    installed_label(agent.installed).to_string(),
                ]
            })
        })
        .collect()
}

fn installed_label(installed: bool) -> &'static str {
    if installed { "yes" } else { "no" }
}

#[cfg(test)]
mod tests {
    use moatless_api_schemas::agent::{AgentDescriptor, AgentModel};

    use super::*;

    #[test]
    fn model_rows_are_grouped_by_agent() {
        let resp = ListAgentsResponse {
            agents: vec![
                AgentDescriptor {
                    id: "agent-a".to_string(),
                    label: "Agent A".to_string(),
                    installed: true,
                    default_model: "default".to_string(),
                    models: vec![
                        AgentModel {
                            id: "default".to_string(),
                            label: "Default".to_string(),
                        },
                        AgentModel {
                            id: "larger".to_string(),
                            label: "Larger".to_string(),
                        },
                    ],
                    missing_credential_hint: None,
                },
                AgentDescriptor {
                    id: "agent-b".to_string(),
                    label: "Agent B".to_string(),
                    installed: false,
                    default_model: "compact".to_string(),
                    models: vec![AgentModel {
                        id: "compact".to_string(),
                        label: "Compact".to_string(),
                    }],
                    missing_credential_hint: Some("configure credentials".to_string()),
                },
            ],
        };

        assert_eq!(
            agent_model_rows(&resp),
            vec![
                vec!["agent-a", "default", "Default", "yes", "yes"],
                vec!["agent-a", "larger", "Larger", "", "yes"],
                vec!["agent-b", "compact", "Compact", "yes", "no"],
            ]
        );
    }
}
