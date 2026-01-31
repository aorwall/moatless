export function getTaskMetadataPromptText(): string {
  return `## Task Naming — REQUIRED

This task does not have a name yet. You MUST call \`mcp__moatless-tools__updateTaskMetadata\` to set a name and description BEFORE making any other tool calls. This is a mandatory first step for every task.

### Tool Usage

Use the \`mcp__moatless-tools__updateTaskMetadata\` tool with these parameters:
- \`name\` (required string): URL-friendly task name (alphanumeric and dashes only, e.g. "add-login-form")
- \`description\` (required string): Brief description of the task goal

Example:
\`\`\`json
{
  "name": "add-login-form",
  "description": "Adds a login form with email/password fields"
}
\`\`\`

### Guidelines

- ALWAYS call this tool first, before any Read, Write, Edit, or Bash calls
- Use descriptive, action-oriented names (e.g., "fix-auth-bug", "add-dark-mode")
- Keep descriptions concise and focused on the goal
- Only call this once per task`
}
