interface ServerInfo {
  name: string
  label?: string | undefined
  port?: number | undefined
}

/**
 * Generate server management prompt text for all configured servers.
 *
 * @param servers - List of configured servers (name + optional label)
 * @param externalPreviewUrl - Optional external URL for the preview server
 */
export function getServerManagementPromptText(
  servers: ServerInfo[],
  externalPreviewUrl?: string,
): string {
  if (servers.length === 0) {
    return ''
  }

  const urlSection = externalPreviewUrl
    ? `\n\nThe user sees the preview at: ${externalPreviewUrl}\nThis URL is mapped to the internal server running on port 3000 inside the container.`
    : ''

  // Build server table
  const serverRows = servers
    .map((s) => `| ${s.name} | ${s.label || s.name} | ${s.port ?? '—'} |`)
    .join('\n')

  return `## Server Management

**Interface Layout**: On the left side is a chat window. On the right is a panel showing files, preview, or other server views.

Multiple servers are configured for this workspace. Use the \`server\` command to manage them.

### Configured Servers

| Server | Label | Port |
|--------|-------|------|
${serverRows}

### Commands

\`\`\`bash
# Manage any server by name
/opt/moatless/bin/server <name> start
/opt/moatless/bin/server <name> stop
/opt/moatless/bin/server <name> restart
/opt/moatless/bin/server <name> status
/opt/moatless/bin/server <name> logs
/opt/moatless/bin/server <name> logs -n 500
/opt/moatless/bin/server <name> logs -f
\`\`\`

### Filesystem Layout

All server state is stored in \`/opt/moatless/data/\`:

| File | Purpose |
|------|---------|
| \`/opt/moatless/data/{name}-server.config.json\` | Server configuration (commands, port, timeout) |
| \`/opt/moatless/data/{name}-server.pid\` | Process ID of the running server |
| \`/opt/moatless/data/{name}-server.status.json\` | Current server status (JSON) |
| \`/opt/moatless/data/{name}-server.log\` | Server stdout/stderr output |

### Config File Format (\`{name}-server.config.json\`)

\`\`\`json
{
  "startCommands": ["npm run dev"],
  "port": 3000,
  "startupTimeoutSeconds": 120,
  "workspacePath": "/opt/moatless/repo"
}
\`\`\`

- \`startCommands\`: Commands to start the dev server.
- \`port\`: The port the server listens on
- \`startupTimeoutSeconds\`: How long to wait for the server to respond to health checks.
- \`workspacePath\`: Working directory for the server commands.

### How It Works

1. The \`start\` command reads \`{name}-server.config.json\`
2. Runs the commands with \`HOST=0.0.0.0\` set in the environment
3. Logs stdout/stderr to \`{name}-server.log\`
4. Polls \`http://localhost:<port>\` until the server responds (up to timeout)
5. Writes status to \`{name}-server.status.json\`

### Troubleshooting

If a server fails to start:

1. **Check logs**: \`/opt/moatless/bin/server <name> logs\` -- look for error messages
2. **Check status**: \`/opt/moatless/bin/server <name> status\` -- see the error field
3. **Fix the config**: Edit \`/opt/moatless/data/<name>-server.config.json\` directly to fix commands
   - Change \`startCommands\` to the correct start command for this project
   - Adjust \`port\` if needed
   - Adjust \`startupTimeoutSeconds\` if the server needs more time
4. **Restart**: \`/opt/moatless/bin/server <name> restart\`
5. **Check the process**: If PID file exists but server seems dead, stop first then start

If the script itself has issues:
- The script is at \`/opt/moatless/bin/server\` -- you can read and fix it directly
- After fixing, restart with the corrected script

### Critical Rules

- **NEVER** start dev servers manually (npm start, yarn dev, bun dev, etc.). Always use \`/opt/moatless/bin/server <name>\` commands.
- Servers auto-start when the container boots (if configured).
- After code changes, restart if the framework doesn't support hot reload.${urlSection}`
}

/**
 * Legacy wrapper — kept for backward compatibility.
 * @deprecated Use getServerManagementPromptText instead.
 */
export function getPreviewServerPromptText(externalPreviewUrl?: string): string {
  return getServerManagementPromptText([{ name: 'preview', label: 'Preview' }], externalPreviewUrl)
}
