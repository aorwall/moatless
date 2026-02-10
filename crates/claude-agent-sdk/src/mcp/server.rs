use std::collections::HashMap;

use serde_json::Value;
use tracing::{debug, warn};

use super::tool::SdkMcpTool;

/// An in-process MCP server that hosts SDK-defined tools.
///
/// Build one via `SdkMcpServer::builder()`, add tools, then call `.build()`.
/// The server handles JSON-RPC messages from the CLI (tools/list, tools/call)
/// and dispatches to the registered tool handlers.
pub struct SdkMcpServer {
    pub(crate) name: String,
    pub(crate) version: String,
    tools: HashMap<String, SdkMcpTool>,
}

/// Builder for constructing an `SdkMcpServer`.
pub struct SdkMcpServerBuilder {
    name: String,
    version: String,
    tools: Vec<SdkMcpTool>,
}

impl SdkMcpServer {
    /// Create a builder for a new SDK MCP server with the given name.
    pub fn builder(name: impl Into<String>) -> SdkMcpServerBuilder {
        SdkMcpServerBuilder {
            name: name.into(),
            version: "1.0.0".to_string(),
            tools: Vec::new(),
        }
    }

    /// Get the server name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the list of tool names registered with this server.
    pub fn tool_names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// Handle an incoming JSON-RPC message and return the response.
    ///
    /// This is called by the orchestrator when a `McpMessage` control request
    /// is received for this server.
    pub async fn handle_message(&self, message: &Value) -> Value {
        let method = message.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let id = message.get("id").cloned().unwrap_or(Value::Null);

        match method {
            "initialize" => {
                debug!(server = %self.name, "MCP initialize");
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "result": {
                        "protocolVersion": "2024-11-05",
                        "capabilities": {
                            "tools": {}
                        },
                        "serverInfo": {
                            "name": self.name,
                            "version": self.version,
                        }
                    },
                    "id": id
                })
            }
            "notifications/initialized" => {
                // Notification — no response expected
                Value::Null
            }
            "tools/list" => {
                debug!(server = %self.name, "tools/list");
                let tools: Vec<Value> = self.tools.values().map(|t| t.to_tool_info()).collect();
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "result": {
                        "tools": tools
                    },
                    "id": id
                })
            }
            "tools/call" => {
                let params = message.get("params").cloned().unwrap_or(Value::Null);
                let tool_name = params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::json!({}));

                debug!(server = %self.name, tool = tool_name, "tools/call");

                if let Some(tool) = self.tools.get(tool_name) {
                    if let Some(handler) = &tool.handler {
                        let result = handler(arguments).await;
                        serde_json::json!({
                            "jsonrpc": "2.0",
                            "result": result.to_json(),
                            "id": id
                        })
                    } else {
                        warn!(server = %self.name, tool = tool_name, "Tool has no handler");
                        serde_json::json!({
                            "jsonrpc": "2.0",
                            "error": {
                                "code": -32603,
                                "message": format!("Tool '{tool_name}' has no handler")
                            },
                            "id": id
                        })
                    }
                } else {
                    warn!(server = %self.name, tool = tool_name, "Tool not found");
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "error": {
                            "code": -32601,
                            "message": format!("Tool '{tool_name}' not found")
                        },
                        "id": id
                    })
                }
            }
            "ping" => {
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "result": {},
                    "id": id
                })
            }
            _ => {
                warn!(server = %self.name, method, "Unknown MCP method");
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "error": {
                        "code": -32601,
                        "message": format!("Method '{method}' not found")
                    },
                    "id": id
                })
            }
        }
    }
}

impl SdkMcpServerBuilder {
    /// Set the server version (default: "1.0.0").
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// Add a tool to the server.
    pub fn tool(mut self, tool: SdkMcpTool) -> Self {
        self.tools.push(tool);
        self
    }

    /// Build the server.
    pub fn build(self) -> SdkMcpServer {
        let mut tools = HashMap::new();
        for tool in self.tools {
            tools.insert(tool.name.clone(), tool);
        }
        SdkMcpServer {
            name: self.name,
            version: self.version,
            tools,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::tool::ToolResult;

    fn build_calculator_server() -> SdkMcpServer {
        SdkMcpServer::builder("calculator")
            .version("2.0.0")
            .tool(
                SdkMcpTool::new("add", "Add two numbers")
                    .input_schema(serde_json::json!({
                        "type": "object",
                        "properties": {
                            "a": {"type": "number"},
                            "b": {"type": "number"}
                        },
                        "required": ["a", "b"]
                    }))
                    .handler(|args| {
                        Box::pin(async move {
                            let a = args["a"].as_f64().unwrap_or(0.0);
                            let b = args["b"].as_f64().unwrap_or(0.0);
                            ToolResult::text(format!("{a} + {b} = {}", a + b))
                        })
                    }),
            )
            .tool(
                SdkMcpTool::new("multiply", "Multiply two numbers")
                    .input_schema(serde_json::json!({
                        "type": "object",
                        "properties": {
                            "a": {"type": "number"},
                            "b": {"type": "number"}
                        },
                        "required": ["a", "b"]
                    }))
                    .handler(|args| {
                        Box::pin(async move {
                            let a = args["a"].as_f64().unwrap_or(0.0);
                            let b = args["b"].as_f64().unwrap_or(0.0);
                            ToolResult::text(format!("{a} * {b} = {}", a * b))
                        })
                    }),
            )
            .build()
    }

    #[tokio::test]
    async fn handle_initialize() {
        let server = build_calculator_server();
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "initialize",
            "params": {},
            "id": 1
        });

        let resp = server.handle_message(&msg).await;
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["serverInfo"]["name"], "calculator");
        assert_eq!(resp["result"]["serverInfo"]["version"], "2.0.0");
    }

    #[tokio::test]
    async fn handle_tools_list() {
        let server = build_calculator_server();
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tools/list",
            "id": 2
        });

        let resp = server.handle_message(&msg).await;
        let tools = resp["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2);

        let names: Vec<&str> = tools
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"add"));
        assert!(names.contains(&"multiply"));
    }

    #[tokio::test]
    async fn handle_tools_call_success() {
        let server = build_calculator_server();
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": {
                "name": "add",
                "arguments": {"a": 3, "b": 4}
            },
            "id": 3
        });

        let resp = server.handle_message(&msg).await;
        assert_eq!(resp["id"], 3);
        let content = &resp["result"]["content"][0];
        assert_eq!(content["type"], "text");
        assert_eq!(content["text"], "3 + 4 = 7");
    }

    #[tokio::test]
    async fn handle_tools_call_not_found() {
        let server = build_calculator_server();
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": {
                "name": "divide",
                "arguments": {"a": 1, "b": 2}
            },
            "id": 4
        });

        let resp = server.handle_message(&msg).await;
        assert_eq!(resp["id"], 4);
        assert!(resp["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not found"));
    }

    #[tokio::test]
    async fn handle_ping() {
        let server = build_calculator_server();
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "ping",
            "id": 5
        });

        let resp = server.handle_message(&msg).await;
        assert_eq!(resp["id"], 5);
        assert!(resp["result"].is_object());
    }

    #[tokio::test]
    async fn handle_unknown_method() {
        let server = build_calculator_server();
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "unknown/method",
            "id": 6
        });

        let resp = server.handle_message(&msg).await;
        assert!(resp["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not found"));
    }

    #[test]
    fn server_metadata() {
        let server = build_calculator_server();
        assert_eq!(server.name(), "calculator");
        let names = server.tool_names();
        assert_eq!(names.len(), 2);
    }

    #[tokio::test]
    async fn notifications_return_null() {
        let server = build_calculator_server();
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });

        let resp = server.handle_message(&msg).await;
        assert!(resp.is_null());
    }
}
