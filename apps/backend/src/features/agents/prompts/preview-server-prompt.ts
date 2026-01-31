export function getPreviewServerPromptText(externalPreviewUrl?: string): string {
  const urlSection = externalPreviewUrl
    ? `\n\nThe user sees the preview at: ${externalPreviewUrl}\nThis URL is mapped to the internal server running on port 3000 inside the container.`
    : ''

  return `## Preview Server

**Interface Layout**: On the left side is a chat window. On the right is a live preview iframe showing your application in real-time.

A preview server is configured for this task. Use the \`server\` command to manage it.

### Commands

\`\`\`bash
# Start the preview server
/opt/moatless/bin/server preview start

# Stop the preview server
/opt/moatless/bin/server preview stop

# Restart (stop + start)
/opt/moatless/bin/server preview restart

# Check server status (JSON output)
/opt/moatless/bin/server preview status

# View logs (last 100 lines)
/opt/moatless/bin/server preview logs

# View more lines
/opt/moatless/bin/server preview logs -n 500

# Follow logs in real-time
/opt/moatless/bin/server preview logs -f
\`\`\`

### Filesystem Layout

All preview server state is stored in \`/opt/moatless/data/\`:

| File | Purpose |
|------|---------|
| \`/opt/moatless/data/preview-server.config.json\` | Server configuration (commands, port, timeout) |
| \`/opt/moatless/data/preview-server.pid\` | Process ID of the running server |
| \`/opt/moatless/data/preview-server.status.json\` | Current server status (JSON) |
| \`/opt/moatless/data/preview-server.log\` | Server stdout/stderr output |

### Config File Format (\`preview-server.config.json\`)

\`\`\`json
{
  "startCommands": ["npm run dev -- --port {{PORT}}"],
  "port": 3000,
  "startupTimeoutSeconds": 120,
  "workspacePath": "/opt/moatless/repo"
}
\`\`\`

- \`startCommands\`: Commands to start the dev server. \`{{PORT}}\` is replaced with the configured port.
- \`port\`: The port the server listens on (default 3000). Also exported as \`PORT\` env var.
- \`startupTimeoutSeconds\`: How long to wait for the server to respond to health checks.
- \`workspacePath\`: Working directory for the server commands.

### How It Works

1. The \`start\` command reads \`preview-server.config.json\`
2. Replaces \`{{PORT}}\` in start commands with the configured port
3. Runs the commands with \`HOST=0.0.0.0\` and \`PORT=<port>\` set in the environment, so the server binds to all interfaces (required for Docker port mapping)
4. Logs stdout/stderr to \`preview-server.log\`
5. Polls \`http://localhost:<port>\` until the server responds (up to timeout)
6. Writes status to \`preview-server.status.json\`

### Troubleshooting

If the server fails to start:

1. **Check logs**: \`/opt/moatless/bin/server preview logs\` -- look for error messages
2. **Check status**: \`/opt/moatless/bin/server preview status\` -- see the error field
3. **Fix the config**: Edit \`/opt/moatless/data/preview-server.config.json\` directly to fix commands
   - Change \`startCommands\` to the correct start command for this project
   - Adjust \`port\` if needed
   - Adjust \`startupTimeoutSeconds\` if the server needs more time
4. **Restart**: \`/opt/moatless/bin/server preview restart\`
5. **Check the process**: If PID file exists but server seems dead, stop first then start

If the script itself has issues:
- The script is at \`/opt/moatless/bin/server\` -- you can read and fix it directly
- After fixing, restart with the corrected script

### Critical Rules

- **NEVER** start dev servers manually (npm start, yarn dev, bun dev, etc.). Always use \`/opt/moatless/bin/server preview\` commands.
- The preview server auto-starts when the container boots (if configured).
- After code changes, restart if the framework doesn't support hot reload.${urlSection}`
}
