# @moatless/shell-utils

Shell, CLI, and process management utilities for Node.js.

## Installation

```bash
bun add @moatless/shell-utils
```

## ShellCommand

Execute shell commands with logging, streaming, and process management.

```typescript
import { ShellCommand } from '@moatless/shell-utils'

const cmd = new ShellCommand()

// Basic execution
await cmd.execute('npm install', {
  workingDirectory: '/path/to/project',
  background: false,
})

// With shell setup (e.g., NVM)
await cmd.execute('node --version', {
  shellSetup: 'export NVM_DIR="$HOME/.nvm"; [ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh"',
  background: false,
})

// With log file
await cmd.execute('npm run build', {
  logFile: '/var/log/build.log',
  timeout: 300000,
})

// Stream logs
for await (const log of cmd.streamLogs()) {
  console.log(`[${log.type}] ${log.text}`)
}

// Get result
const result = await cmd.executeWithResult('ls -la')
console.log(result.output, result.exitCode, result.success)

// Execute sequence
await cmd.executeSequence(['npm install', 'npm build', 'npm test'], {
  separator: ' && ',
})
```

### Options
- `workingDirectory` - Where to execute
- `env` - Environment variables (merged with safe system env)
- `timeout` - Timeout in milliseconds
- `background` - Run in background (default: true)
- `shell` - Custom shell
- `shellSetup` - Shell init commands (string or array)
- `logFile` - Log file path

### Methods

#### `execute(command, options)`
Execute a shell command. Returns immediately if `background: true`, otherwise waits for completion.

#### `executeWithResult(command, options)`
Execute and return `ShellCommandResult` with `{ command, output, exitCode, durationMs, success, error? }`.

#### `executeSequence(commands, options)`
Execute multiple commands joined by separator (default: `' && '`). Stops on first error unless using `;` separator.

#### `kill()`
Kill the running command and all child processes. Returns `true` if killed, `false` if not running.

#### `getStatus()`
Returns current status: `'idle' | 'running' | 'completed' | 'failed' | 'killed'`.

#### `getLogs(lines?)`
Returns array of `ShellCommandLogEvent` objects. Optionally limit to last N lines.

#### `streamLogs()`
Returns async generator that yields log events in real-time. Yields existing logs first, then streams new ones.

#### `getFormattedLogs(includeSystem?, lines?)`
Returns formatted log string with timestamps. Set `includeSystem: false` to exclude start/exit logs.

#### `getOutputOnly()`
Returns stdout/stderr output without system logs or timestamps.

#### `getPid()`
Returns process ID if running, otherwise `undefined`.

#### `getExitCode()`
Returns exit code of completed command, otherwise `undefined`.

#### `getDuration()`
Returns command duration in milliseconds, otherwise `undefined`.

#### `getError()`
Returns error message if command failed, otherwise `undefined`.

#### `getStartTime()`
Returns command start time as ISO string, otherwise `undefined`.

#### `getEndTime()`
Returns command end time as ISO string, otherwise `undefined`.

#### `getWorkingDirectory()`
Returns working directory where command was executed, otherwise `undefined`.

#### `getShell()`
Returns shell used for command execution, otherwise `undefined`.

#### `getCommand()`
Returns the executed command string, otherwise `undefined`.

#### `isRunning()`
Returns `true` if command is currently running.

## Network Utilities

```typescript
import {
  getPort,
  isPortAvailable,
  findAnyAvailablePort,
  pingPort,
  waitForPort
} from '@moatless/shell-utils'

const port = await getPort()
const available = await isPortAvailable(3000)
const ready = await waitForPort(3000, 10000)
```

#### `isPortAvailable(port, timeoutMs?)`
Check if a port is available (not in use). Tests both IPv4 and IPv6 loopback. Returns `true` if port is free.

#### `getPort()`
Find a random available port in the ephemeral range (49152-65535). Throws if no port found after 10 attempts.

#### `findAnyAvailablePort()`
Get any available port using OS assignment (port 0). Returns OS-assigned port number.

#### `pingPort(port, timeout?)`
Check if a port is responsive by attempting connection. Returns `true` if connection succeeds.

#### `waitForPort(port, maxWait?, callbacks?)`
Wait for a port to become responsive. Polls every 500ms. Returns `true` if port becomes ready, `false` on timeout.

Callbacks: `{ onWaiting?, onReady?, onTimeout? }`

## Process Management

```typescript
import { treeKill, treeKillAsync } from '@moatless/shell-utils'

// Callback-based
treeKill(pid, 'SIGTERM', (err) => {
  if (err) console.error(err)
})

// Promise-based
await treeKillAsync(pid, 'SIGTERM')
```

#### `treeKill(pid, signal?, callback?)`
Kill a process and all its children. Callback-based. Signal defaults to `SIGTERM`.

- Windows: Uses `taskkill /T /F`
- Unix: Builds process tree from `ps` and kills all descendants

#### `treeKillAsync(pid, signal?)`
Promise-based version of `treeKill`. Returns promise that resolves when kill completes.

## Environment Variables

```typescript
import { getSafeSystemEnvironment } from '@moatless/shell-utils'

const safeEnv = getSafeSystemEnvironment()
```

#### `getSafeSystemEnvironment()`
Extract safe system environment variables for child processes. Excludes backend-specific variables, includes only essential system variables.

Returns: `PATH`, `HOME`, `USER`, `SHELL`, `TERM`, `LANG`, `TMPDIR`, `NVM_DIR`, `PYENV_ROOT`, `RBENV_ROOT`, `SDKMAN_DIR`, `CI`, and other system essentials.