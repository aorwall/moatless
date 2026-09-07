//! The configuration envelope passed to a sandbox through `REPO_CONFIG`.

use serde_json::{Map, Value, json};

use crate::provisioning::RepoConfig;
use moatless_types::workspace::{Port, ServerConfig};

/// Encode shared configuration with the runtime's additional server fields.
pub fn encode_repo_config(
    config: &RepoConfig,
    server_fields: impl Fn(&ServerConfig) -> Map<String, Value>,
) -> Option<String> {
    let mut values = Map::new();
    if let Some(prompt) = &config.system_prompt {
        values.insert("systemPrompt".into(), json!(prompt));
    }
    if let Some(execution) = &config.code_execution {
        values.insert("codeExecution".into(), execution.clone());
    }
    if let Some(preview) = &config.preview_config {
        values.insert("previewConfig".into(), preview.clone());
    }
    if let Some(servers) = &config.servers {
        let servers: Vec<_> = servers
            .iter()
            .map(|server| {
                let mut fields = server_fields(server);
                fields.insert("name".into(), json!(server.name));
                fields.insert("port".into(), json!(server.port.map(Port::get)));
                Value::Object(fields)
            })
            .collect();
        values.insert("servers".into(), json!(servers));
    }
    if let Some(plugins) = &config.plugins {
        values.insert("plugins".into(), json!(plugins));
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
        assert!(encode_repo_config(&config, |_| Map::new()).is_none());
        config.servers = Some(Vec::new());
        config.system_prompt = Some(String::new());
        let encoded = encode_repo_config(&config, |_| Map::new()).expect("present");
        assert_eq!(
            serde_json::from_str::<Value>(&encoded).expect("json"),
            json!({"servers":[],"systemPrompt":""})
        );
    }
}
