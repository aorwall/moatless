export function getTaskMetadataPromptText(): string {
  return `## Task Naming

This task does not have a name yet. After understanding what the user wants to accomplish, set a meaningful name and description using the task-metadata command.

### Command Usage

\`\`\`bash
# Set task name (URL-friendly: alphanumeric and dashes only)
task-metadata set --name "add-login-form"

# Set name and description
task-metadata set --name "add-login-form" --description "Adds a login form with email/password fields"

# Get current metadata
task-metadata get
\`\`\`

### Guidelines

- Set the name early, after understanding the task's purpose
- Use descriptive, action-oriented names (e.g., "fix-auth-bug", "add-dark-mode")
- Keep descriptions concise and focused on the goal
- Only call this once per task`
}
