---
name: preview-server
description: Manage the preview server for live preview. Use when starting, stopping, restarting the dev server, checking logs, or troubleshooting server issues.
---

# Preview Server Management

The preview server runs your application for live preview on a configured port (default 3000).
Users see the preview in a live iframe on the right side of their interface.

## Commands

```bash
# Start the preview server
/opt/moatless/bin/preview-server start

# Stop the preview server
/opt/moatless/bin/preview-server stop

# Restart (stop + start)
/opt/moatless/bin/preview-server restart

# Check server status (JSON output)
/opt/moatless/bin/preview-server status

# View logs (last 100 lines)
/opt/moatless/bin/preview-server logs

# View more lines
/opt/moatless/bin/preview-server logs -n 500

# Follow logs in real-time
/opt/moatless/bin/preview-server logs -f
```

## Filesystem Layout

All preview server state is stored in `/opt/moatless/data/`:

| File | Purpose |
|------|---------|
| `/opt/moatless/data/preview-server.config.json` | Server configuration (commands, port, timeout) |
| `/opt/moatless/data/preview-server.pid` | Process ID of the running server |
| `/opt/moatless/data/preview-server.status.json` | Current server status (JSON) |
| `/opt/moatless/data/preview-server.log` | Server stdout/stderr output |

### Config file format (`preview-server.config.json`)

```json
{
  "serverCommands": ["npm run dev -- --port {{PORT}}"],
  "port": 3000,
  "startupTimeoutSeconds": 120,
  "workspacePath": "/opt/moatless/repo"
}
```

- `serverCommands`: Commands to start the dev server. `{{PORT}}` is replaced with the configured port.
- `port`: The port the server listens on (default 3000). Also exported as `PORT` env var.
- `startupTimeoutSeconds`: How long to wait for the server to respond to health checks.
- `workspacePath`: Working directory for the server commands.

### Status file format (`preview-server.status.json`)

```json
{
  "status": "started",
  "port": 3000,
  "pid": 12345,
  "startedAt": "2025-01-15T10:30:00Z"
}
```

Status values: `starting`, `started`, `stopped`, `failed`
When failed: includes `"error": "description of failure"`

## How It Works

1. The `start` command reads `preview-server.config.json`
2. Replaces `{{PORT}}` in server commands with the configured port
3. Runs the commands with `PORT=<port>` in the environment
4. Logs stdout/stderr to `preview-server.log`
5. Polls `http://localhost:<port>` until the server responds (up to timeout)
6. Writes status to `preview-server.status.json`

## Troubleshooting

If the server fails to start:

1. **Check logs**: `/opt/moatless/bin/preview-server logs` -- look for error messages
2. **Check status**: `/opt/moatless/bin/preview-server status` -- see the error field
3. **Fix the config**: Edit `/opt/moatless/data/preview-server.config.json` directly to fix commands
   - Change `serverCommands` to the correct start command for this project
   - Adjust `port` if needed
   - Adjust `startupTimeoutSeconds` if the server needs more time
4. **Restart**: `/opt/moatless/bin/preview-server restart`
5. **Check the process**: If PID file exists but server seems dead, stop first then start

If the script itself has issues:
- The script is at `/opt/moatless/bin/preview-server` -- you can read and fix it directly
- After fixing, restart with the corrected script

## Important Rules

- DO NOT start dev servers manually (npm start, yarn dev, bun dev, etc.)
- Always use `/opt/moatless/bin/preview-server` commands
- The preview server auto-starts when the container boots (if configured)
- After code changes, restart if the framework doesn't support hot reload
