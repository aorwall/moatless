/**
 * TypeScript types for Claude Code API
 */

export type PermissionMode = 'default' | 'acceptEdits' | 'bypassPermissions';

export interface ClaudeCodeToolInfo {
  name: string;
  description: string;
  permission_required: boolean;
}

export interface ClaudeCodeSettingsResponse {
  // Model configuration
  model: string | null;
  max_turns: number | null;
  max_thinking_tokens: number;

  // System prompts
  system_prompt: string | null;
  append_system_prompt: string | null;

  // Tool configuration
  allowed_tools: Array<string>;
  disallowed_tools: Array<string>;
  permission_mode: PermissionMode | null;

  // MCP configuration
  mcp_servers: Record<string, Record<string, unknown>>;
  mcp_tools: Array<string>;

  // Metadata
  created_at: string | null;
  updated_at: string | null;
}

export interface UpdateClaudeCodeSettingsRequest {
  // Model configuration
  model?: string | null;
  max_turns?: number | null;
  max_thinking_tokens?: number;

  // System prompts
  system_prompt?: string | null;
  append_system_prompt?: string | null;

  // Tool configuration
  allowed_tools?: Array<string>;
  disallowed_tools?: Array<string>;
  permission_mode?: PermissionMode | null;

  // MCP configuration
  mcp_servers?: Record<string, Record<string, unknown>>;
  mcp_tools?: Array<string>;
}

export interface ClaudeCodeToolsResponse {
  tools: Array<ClaudeCodeToolInfo>;
}
