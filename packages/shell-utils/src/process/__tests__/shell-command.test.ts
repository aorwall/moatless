import { describe, expect, it } from 'vitest'
import { ShellCommand } from '../shell-command.js'

describe('ShellCommand', () => {
  describe('execute', () => {
    it('should execute a simple command', async () => {
      const cmd = new ShellCommand({})
      await cmd.execute('echo "hello"', { background: false })

      expect(cmd.getStatus()).toBe('completed')
      expect(cmd.getExitCode()).toBe(0)

      const output = cmd.getOutputOnly()
      expect(output).toContain('hello')
    })

    it('should execute command in background by default', async () => {
      const cmd = new ShellCommand({})
      await cmd.execute('echo "test"')

      // Should return immediately
      expect(cmd.isRunning()).toBe(true)

      // Wait for completion
      await new Promise((resolve) => setTimeout(resolve, 500))

      expect(cmd.getStatus()).toBe('completed')
    })

    it('should capture stdout in logs', async () => {
      const cmd = new ShellCommand({})

      await cmd.execute('echo "test output"', {
        background: false,
      })

      const logs = cmd.getLogs()
      const stdoutLogs = logs.filter((log) => log.type === 'stdout')
      expect(stdoutLogs.length).toBeGreaterThan(0)
      expect(stdoutLogs.map((l) => l.text).join('')).toContain('test output')
    })

    it('should capture stderr in logs', async () => {
      const cmd = new ShellCommand({})

      await cmd.execute('echo "error" >&2', {
        background: false,
      })

      const logs = cmd.getLogs()
      const stderrLogs = logs.filter((log) => log.type === 'stderr')
      expect(stderrLogs.length).toBeGreaterThan(0)
      expect(stderrLogs.map((l) => l.text).join('')).toContain('error')
    })

    it('should log command start', async () => {
      const cmd = new ShellCommand({})

      await cmd.execute('echo "test"', {
        background: false,
      })

      const logs = cmd.getLogs()
      const startLogs = logs.filter(
        (log) => log.type === 'system' && log.text.includes('Command started'),
      )
      expect(startLogs.length).toBe(1)
      expect(startLogs[0].text).toContain('echo "test"')
      expect(startLogs[0].text).toContain('PID:')
    })

    it('should log command exit', async () => {
      const cmd = new ShellCommand({})

      await cmd.execute('echo "test"', {
        background: false,
      })

      const logs = cmd.getLogs()
      const exitLogs = logs.filter((log) => log.type === 'system' && log.text.includes('exit code'))
      expect(exitLogs.length).toBe(1)
      expect(exitLogs[0].text).toContain('completed')
      expect(exitLogs[0].text).toContain('exit code 0')
    })

    it('should handle command failure', async () => {
      const cmd = new ShellCommand({})

      await cmd.execute('exit 1', { background: false })

      expect(cmd.getStatus()).toBe('failed')
      expect(cmd.getExitCode()).toBe(1)
    })

    it('should handle process spawn errors', async () => {
      const cmd = new ShellCommand({})

      // Try to execute a non-existent command
      await cmd.execute('/nonexistent/command', {
        background: false,
      })

      // Command should still complete (spawn doesn't fail, the process does)
      const status = cmd.getStatus()
      expect(['failed', 'completed']).toContain(status)
    })

    it('should prevent multiple executions', async () => {
      const cmd = new ShellCommand({})

      await cmd.execute('echo "first"', { background: false })

      // Try to execute again
      await expect(cmd.execute('echo "second"')).rejects.toThrow(
        'ShellCommand can only be executed once',
      )
    })

    it('should use custom working directory', async () => {
      const cmd = new ShellCommand({})

      await cmd.execute('pwd', {
        background: false,
        workingDirectory: '/tmp',
      })

      const output = cmd.getOutputOnly()
      expect(output).toContain('/tmp')
    })

    it('should use custom environment variables', async () => {
      const cmd = new ShellCommand({})

      await cmd.execute('echo $TEST_VAR', {
        background: false,
        env: {
          TEST_VAR: 'custom_value',
        },
      })

      const output = cmd.getOutputOnly()
      expect(output).toContain('custom_value')
    })

    it('should apply shell setup commands', async () => {
      const cmd = new ShellCommand({})

      await cmd.execute('echo $MY_VAR', {
        background: false,
        shellSetup: 'export MY_VAR="from_setup"',
      })

      const output = cmd.getOutputOnly()
      expect(output).toContain('from_setup')
    })

    it('should apply multiple shell setup commands', async () => {
      const cmd = new ShellCommand({})

      await cmd.execute('echo $VAR1 $VAR2', {
        background: false,
        shellSetup: ['export VAR1="first"', 'export VAR2="second"'],
      })

      const output = cmd.getOutputOnly()
      expect(output).toContain('first')
      expect(output).toContain('second')
    })

    it('should timeout long-running commands', async () => {
      const cmd = new ShellCommand({})

      await cmd.execute('sleep 10', {
        background: true,
        timeout: 500,
      })

      // Wait for timeout
      await new Promise((resolve) => setTimeout(resolve, 700))

      expect(cmd.getStatus()).toBe('killed')
      expect(cmd.getError()).toBe('Command timed out')
    })
  })

  describe('kill', () => {
    it('should kill a running command', async () => {
      const cmd = new ShellCommand({})

      await cmd.execute('sleep 10', { background: true })

      // Wait a bit for process to start
      await new Promise((resolve) => setTimeout(resolve, 100))

      const killed = await cmd.kill()
      expect(killed).toBe(true)

      await new Promise((resolve) => setTimeout(resolve, 200))

      expect(cmd.getStatus()).toBe('killed')
    })

    it('should return false when no process is running', async () => {
      const cmd = new ShellCommand({})
      const killed = await cmd.kill()
      expect(killed).toBe(false)
    })

    it('should return false when command already completed', async () => {
      const cmd = new ShellCommand({})

      await cmd.execute('echo "test"', { background: false })

      const killed = await cmd.kill()
      expect(killed).toBe(false)
    })
  })

  describe('getStatus', () => {
    it('should return idle initially', () => {
      const cmd = new ShellCommand({})
      expect(cmd.getStatus()).toBe('idle')
    })

    it('should return running during execution', async () => {
      const cmd = new ShellCommand({})

      await cmd.execute('sleep 1', { background: true })

      expect(cmd.getStatus()).toBe('running')
    })

    it('should return completed after successful execution', async () => {
      const cmd = new ShellCommand({})

      await cmd.execute('echo "test"', { background: false })

      expect(cmd.getStatus()).toBe('completed')
    })

    it('should return failed after failed execution', async () => {
      const cmd = new ShellCommand({})

      await cmd.execute('exit 1', { background: false })

      expect(cmd.getStatus()).toBe('failed')
    })
  })

  describe('getPid', () => {
    it('should return undefined before execution', () => {
      const cmd = new ShellCommand({})
      expect(cmd.getPid()).toBeUndefined()
    })

    it('should return PID during execution', async () => {
      const cmd = new ShellCommand({})

      await cmd.execute('sleep 1', { background: true })

      const pid = cmd.getPid()
      expect(pid).toBeTypeOf('number')
      expect(pid).toBeGreaterThan(0)
    })
  })

  describe('isRunning', () => {
    it('should return false initially', () => {
      const cmd = new ShellCommand({})
      expect(cmd.isRunning()).toBe(false)
    })

    it('should return true during execution', async () => {
      const cmd = new ShellCommand({})

      await cmd.execute('sleep 1', { background: true })

      expect(cmd.isRunning()).toBe(true)
    })

    it('should return false after completion', async () => {
      const cmd = new ShellCommand({})

      await cmd.execute('echo "test"', { background: false })

      expect(cmd.isRunning()).toBe(false)
    })
  })

  describe('getOutputOnly', () => {
    it('should return empty string before execution', () => {
      const cmd = new ShellCommand({})
      expect(cmd.getOutputOnly()).toBe('')
    })

    it('should return combined stdout and stderr', async () => {
      const cmd = new ShellCommand({})

      await cmd.execute('echo "out" && echo "err" >&2', { background: false })

      const output = cmd.getOutputOnly()
      expect(output).toContain('out')
      expect(output).toContain('err')
    })

    it('should not include system logs', async () => {
      const cmd = new ShellCommand({})

      await cmd.execute('echo "test"', { background: false })

      const output = cmd.getOutputOnly()
      expect(output).not.toContain('Command started')
      expect(output).not.toContain('PID:')
      expect(output).not.toContain('exit code')
    })
  })

  describe('executeWithResult', () => {
    it('should return structured result', async () => {
      const cmd = new ShellCommand({})

      const result = await cmd.executeWithResult('echo "test"')

      expect(result.command).toBe('echo "test"')
      expect(result.exitCode).toBe(0)
      expect(result.success).toBe(true)
      expect(result.output).toContain('test')
      expect(result.durationMs).toBeGreaterThan(0)
    })

    it('should indicate failure in result', async () => {
      const cmd = new ShellCommand({})

      const result = await cmd.executeWithResult('exit 1')

      expect(result.exitCode).toBe(1)
      expect(result.success).toBe(false)
    })

    it('should handle failure in result', async () => {
      const cmd = new ShellCommand({})

      const result = await cmd.executeWithResult('exit 99')

      expect(result.success).toBe(false)
      expect(result.exitCode).toBe(99)
    })

    it('should wait for completion automatically', async () => {
      const cmd = new ShellCommand({})

      const result = await cmd.executeWithResult('sleep 0.1 && echo "done"')

      expect(result.output).toContain('done')
      expect(result.success).toBe(true)
    })
  })

  describe('getLogs', () => {
    it('should return all logs', async () => {
      const cmd = new ShellCommand({})
      await cmd.execute('echo "test"', { background: false })

      const logs = cmd.getLogs()
      expect(logs.length).toBeGreaterThan(0)

      // Should have start, stdout, and exit logs
      expect(logs.some((log) => log.type === 'system' && log.text.includes('started'))).toBe(true)
      expect(logs.some((log) => log.type === 'stdout')).toBe(true)
      expect(logs.some((log) => log.type === 'system' && log.text.includes('exit code'))).toBe(true)
    })

    it('should limit logs by number of lines', async () => {
      const cmd = new ShellCommand({})
      await cmd.execute('echo "test"', { background: false })

      const allLogs = cmd.getLogs()
      const limitedLogs = cmd.getLogs(1)

      expect(limitedLogs.length).toBe(1)
      expect(limitedLogs[0]).toEqual(allLogs[allLogs.length - 1])
    })
  })

  describe('execute', () => {
    it('should execute multiple commands', async () => {
      const cmd = new ShellCommand({})

      await cmd.execute(['echo "first"', 'echo "second"'], { background: false })

      const output = cmd.getOutputOnly()
      expect(output).toContain('first')
      expect(output).toContain('second')
    })

    it('should use custom separator', async () => {
      const cmd = new ShellCommand({})

      // Use ; separator to execute both commands
      await cmd.execute(['echo "one"', 'echo "two"'], {
        background: false,
        separator: '; ',
      })

      const output = cmd.getOutputOnly()
      expect(output).toContain('one')
      expect(output).toContain('two')
    })

    it('should filter empty commands', async () => {
      const cmd = new ShellCommand({})

      await cmd.execute(['echo "test"', '', '  '], { background: false })

      const output = cmd.getOutputOnly()
      expect(output).toContain('test')
    })

    it('should throw on no valid commands', async () => {
      const cmd = new ShellCommand({})

      await expect(cmd.execute(['', '  '], { background: false })).rejects.toThrow(
        'No valid commands provided',
      )
    })
  })

  describe('getFormattedLogs', () => {
    it('should format logs with timestamps', async () => {
      const cmd = new ShellCommand({})
      await cmd.execute('echo "test"', { background: false })

      const formatted = cmd.getFormattedLogs(true)
      expect(formatted).toContain('[')
      expect(formatted).toContain(']')
      expect(formatted).toContain('[system]')
      expect(formatted).toContain('[stdout]')
    })

    it('should exclude system logs when requested', async () => {
      const cmd = new ShellCommand({})
      await cmd.execute('echo "test"', { background: false })

      const formatted = cmd.getFormattedLogs(false)
      expect(formatted).not.toContain('[system]')
      expect(formatted).toContain('[stdout]')
    })

    it('should limit number of lines', async () => {
      const cmd = new ShellCommand({})
      await cmd.execute('echo "test"', { background: false })

      const allFormatted = cmd.getFormattedLogs(true)
      const limitedFormatted = cmd.getFormattedLogs(true, 1)

      expect(limitedFormatted.length).toBeLessThan(allFormatted.length)
    })
  })
})
