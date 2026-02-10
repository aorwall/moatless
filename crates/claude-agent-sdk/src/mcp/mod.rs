//! SDK MCP Server support — enables in-process tool registration.
//!
//! Instead of running a separate MCP server process, you can register tools
//! directly in your Rust application and they'll be handled in-process.
//!
//! # Example
//!
//! ```no_run
//! use claude_agent_sdk::mcp::{SdkMcpServer, SdkMcpTool, ToolResult};
//! use serde_json::json;
//!
//! let server = SdkMcpServer::builder("calculator")
//!     .tool(
//!         SdkMcpTool::new("add", "Add two numbers")
//!             .input_schema(json!({
//!                 "type": "object",
//!                 "properties": {
//!                     "a": {"type": "number"},
//!                     "b": {"type": "number"}
//!                 },
//!                 "required": ["a", "b"]
//!             }))
//!             .handler(|args| Box::pin(async move {
//!                 let a = args["a"].as_f64().unwrap_or(0.0);
//!                 let b = args["b"].as_f64().unwrap_or(0.0);
//!                 ToolResult::text(format!("{a} + {b} = {}", a + b))
//!             })),
//!     )
//!     .build();
//! ```

mod server;
mod tool;

pub use server::SdkMcpServer;
pub use tool::{SdkMcpTool, ToolHandler, ToolResult};
