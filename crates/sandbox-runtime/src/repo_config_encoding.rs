//! The configuration envelope passed to a sandbox through `REPO_CONFIG`.

use serde_json::{Map, Value, json};

use crate::provisioning::RepoConfig;

/// Encode the configuration a sandbox reads, or `None` when nothing is set.
pub fn encode_repo_config(config: &RepoConfig) -> Option<String> {
    let mut values = Map::new();
    if let Some(prompt) = &config.system_prompt {
        values.insert("systemPrompt".into(), json!(prompt));
    }
    if let Some(execution) = &config.code_execution {
        values.insert("codeExecution".into(), execution.clone());
    }
    if let Some(plugins) = &config.plugins {
        values.insert("plugins".into(), json!(plugins));
    }
    if let Some(servers) = &config.language_servers {
        values.insert("languageServers".into(), json!(servers));
    }
    (!values.is_empty()).then(|| Value::Object(values).to_string())
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "wire fixtures must parse")]
mod tests {
    use super::*;

    #[test]
    fn envelope_distinguishes_absent_and_explicitly_empty_configuration() {
        let mut config: RepoConfig =
            serde_json::from_value(json!({"name":"repo"})).expect("config");
        assert!(encode_repo_config(&config).is_none());
        config.system_prompt = Some(String::new());
        let encoded = encode_repo_config(&config).expect("present");
        assert_eq!(
            serde_json::from_str::<Value>(&encoded).expect("json"),
            json!({"systemPrompt":""})
        );
    }

    #[test]
    fn language_servers_travel_as_their_ids() {
        let mut config: RepoConfig =
            serde_json::from_value(json!({"name":"repo"})).expect("config");
        config.language_servers = Some(vec![
            moatless_workspace_types::workspace::LanguageServer::Rust,
        ]);
        let encoded = encode_repo_config(&config).expect("present");
        assert_eq!(
            serde_json::from_str::<Value>(&encoded).expect("json"),
            json!({"languageServers":["rust"]})
        );
    }
}
