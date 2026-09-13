use serde_json::Value;
use std::future::Future;
use std::pin::Pin;

/// A handler function for an SDK MCP tool.
///
/// Receives the tool arguments as a JSON value and returns a `ToolResult`.
pub type ToolHandler =
    Box<dyn Fn(Value) -> Pin<Box<dyn Future<Output = ToolResult> + Send>> + Send + Sync>;

/// Result of a tool invocation.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: Vec<ToolContent>,
    pub is_error: bool,
}

/// A content block within a tool result.
#[derive(Debug, Clone)]
pub enum ToolContent {
    Text(String),
    Image { data: String, mime_type: String },
}

impl ToolResult {
    /// Create a successful text result.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContent::Text(text.into())],
            is_error: false,
        }
    }

    /// Create an error result.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContent::Text(message.into())],
            is_error: true,
        }
    }

    /// Convert to JSON format expected by MCP protocol.
    pub fn to_json(&self) -> Value {
        let content: Vec<Value> = self
            .content
            .iter()
            .map(|c| match c {
                ToolContent::Text(text) => serde_json::json!({
                    "type": "text",
                    "text": text,
                }),
                ToolContent::Image { data, mime_type } => serde_json::json!({
                    "type": "image",
                    "data": data,
                    "mimeType": mime_type,
                }),
            })
            .collect();

        let mut result = serde_json::json!({ "content": content });
        if self.is_error {
            result["isError"] = Value::Bool(true);
        }
        result
    }
}

/// An SDK MCP tool definition with name, description, input schema, and
/// handler.
pub struct SdkMcpTool {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) input_schema: Value,
    pub(crate) handler: Option<ToolHandler>,
}

impl SdkMcpTool {
    /// Create a new tool with a name and description.
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            handler: None,
        }
    }

    /// Set the JSON Schema for the tool's input parameters.
    pub fn input_schema(mut self, schema: Value) -> Self {
        self.input_schema = schema;
        self
    }

    /// Set the handler function for this tool.
    ///
    /// Receives the tool arguments as a JSON value and must return a
    /// `ToolResult`; wrap the body in `Box::pin(async move { ... })`.
    pub fn handler<F>(mut self, f: F) -> Self
    where
        F: Fn(Value) -> Pin<Box<dyn Future<Output = ToolResult> + Send>> + Send + Sync + 'static,
    {
        self.handler = Some(Box::new(f));
        self
    }

    /// Get the tool's JSON schema for listing.
    pub(crate) fn to_tool_info(&self) -> Value {
        serde_json::json!({
            "name": self.name,
            "description": self.description,
            "inputSchema": self.input_schema,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_result_text() {
        let result = ToolResult::text("Hello!");
        let json = result.to_json();
        assert_eq!(json["content"][0]["type"], "text");
        assert_eq!(json["content"][0]["text"], "Hello!");
        assert!(json.get("isError").is_none());
    }

    #[test]
    fn tool_result_error() {
        let result = ToolResult::error("Something went wrong");
        let json = result.to_json();
        assert_eq!(json["content"][0]["text"], "Something went wrong");
        assert_eq!(json["isError"], true);
    }

    #[test]
    fn tool_info_serialization() {
        let tool = SdkMcpTool::new("add", "Add two numbers").input_schema(serde_json::json!({
            "type": "object",
            "properties": {
                "a": {"type": "number"},
                "b": {"type": "number"}
            },
            "required": ["a", "b"]
        }));

        let info = tool.to_tool_info();
        assert_eq!(info["name"], "add");
        assert_eq!(info["description"], "Add two numbers");
        assert!(info["inputSchema"]["properties"]["a"].is_object());
    }

    #[tokio::test]
    async fn tool_handler_invocation() {
        let tool = SdkMcpTool::new("greet", "Greet someone")
            .input_schema(serde_json::json!({
                "type": "object",
                "properties": {"name": {"type": "string"}},
                "required": ["name"]
            }))
            .handler(|args| {
                Box::pin(async move {
                    let name = args["name"].as_str().unwrap_or("world");
                    ToolResult::text(format!("Hello, {name}!"))
                })
            });

        let handler = tool.handler.as_ref().unwrap();
        let result = handler(serde_json::json!({"name": "Alice"})).await;
        assert_eq!(result.content.len(), 1);
        match &result.content[0] {
            ToolContent::Text(text) => assert_eq!(text, "Hello, Alice!"),
            _ => panic!("Expected text content"),
        }
    }
}
