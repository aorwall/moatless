import { promises as fs } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { ShellCommand } from '@moatless/shell-utils'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'

describe('ShellCommand Integration', () => {
  let tempDir: string
  let logFile: string

  beforeEach(async () => {
    // Create a temporary directory for test files
    tempDir = await fs.mkdtemp(path.join(tmpdir(), 'shell-command-test-'))
    logFile = path.join(tempDir, 'command.log')
  })

  afterEach(async () => {
    // Clean up temporary directory
    try {
      await fs.rm(tempDir, { recursive: true, force: true })
    } catch (error) {
      console.warn('Failed to clean up temp dir:', error)
    }
  })

  describe('Basic Command Execution', () => {
    it('should execute a simple echo command', async () => {
      const cmd = new ShellCommand({})
      await cmd.execute('echo "Hello, World!"', {})

      expect(cmd.getStatus()).toBe('completed')
      expect(cmd.getOutputOnly()).toContain('Hello, World!')
      expect(cmd.getExitCode()).toBe(0)
    })

    it('should execute multiple commands with &&', async () => {
      const cmd = new ShellCommand({})
      await cmd.execute('echo "First" && echo "Second" && echo "Third"', {})

      expect(cmd.getStatus()).toBe('completed')
      const output = cmd.getOutputOnly()
      expect(output).toContain('First')
      expect(output).toContain('Second')
      expect(output).toContain('Third')
    })

    it('should handle command failure', async () => {
      const cmd = new ShellCommand({})
      await cmd.execute('nonexistentcommand123')

      expect(cmd.getStatus()).toBe('failed')
      expect(cmd.getExitCode()).not.toBe(0)
    })

    it('should capture stderr output', async () => {
      const cmd = new ShellCommand({})
      // Use a command that writes to stderr
      await cmd.execute('echo "Error message" >&2')

      const logs = await cmd.getLogs()
      const stderrLog = logs.find((log) => log.type === 'stderr')
      expect(stderrLog).toBeTruthy()
      expect(stderrLog?.text).toContain('Error message')
    })
  })

  describe('Background Execution', () => {
    it('should execute command without blocking', async () => {
      const cmd = new ShellCommand({})
      // Don't await - get the promise but continue immediately
      const promise = cmd.execute('sleep 0.5 && echo "Done"')

      // Command should be running immediately after execute (without await)
      expect(cmd.getStatus()).toBe('running')
      expect(cmd.isRunning()).toBe(true)
      expect(cmd.getPid()).toBeTruthy()

      // Wait for completion via the promise
      await promise

      expect(cmd.getStatus()).toBe('completed')
      expect(cmd.getOutputOnly()).toContain('Done')
    })

    it('should handle multiple concurrent commands', async () => {
      const cmd1 = new ShellCommand({})
      const cmd2 = new ShellCommand({})

      // Start both commands without waiting
      const promise1 = cmd1.execute('sleep 0.2 && echo "Command 1"')
      const promise2 = cmd2.execute('sleep 0.1 && echo "Command 2"')

      expect(cmd1.isRunning()).toBe(true)
      expect(cmd2.isRunning()).toBe(true)

      // Wait for both to complete
      await Promise.all([promise1, promise2])

      expect(cmd1.getStatus()).toBe('completed')
      expect(cmd2.getStatus()).toBe('completed')
      expect(cmd1.getOutputOnly()).toContain('Command 1')
      expect(cmd2.getOutputOnly()).toContain('Command 2')
    })
  })

  describe('Process Management', () => {
    it('should kill a running command', async () => {
      const cmd = new ShellCommand({})
      // Don't await - start the command but don't wait for completion
      const promise = cmd.execute('sleep 10')

      expect(cmd.isRunning()).toBe(true)

      const killed = await cmd.kill()
      expect(killed).toBe(true)

      // Wait for the promise to resolve after kill
      await promise

      expect(cmd.getStatus()).toBe('killed')
      expect(cmd.isRunning()).toBe(false)
    })

    it('should handle timeout', async () => {
      const cmd = new ShellCommand({
        timeout: 500, // 500ms timeout
      })
      await cmd.execute('sleep 10')

      expect(cmd.getStatus()).toBe('killed')
      expect(cmd.getError()).toContain('timed out')
    })

    it('should not allow executing more than once', async () => {
      const cmd = new ShellCommand({})
      // Start first command without waiting
      const promise = cmd.execute('sleep 1')

      // Now that ShellCommand is single-use, it will throw the single-use error
      await expect(cmd.execute('echo "Second"')).rejects.toThrow(
        'ShellCommand can only be executed once',
      )

      await cmd.kill()
      await promise
    })
  })

  describe('Working Directory', () => {
    it('should execute command in specified directory', async () => {
      const cmd = new ShellCommand({ workingDirectory: tempDir })
      await cmd.execute('pwd')

      expect(cmd.getStatus()).toBe('completed')
      // On macOS, /var is symlinked to /private/var, so we normalize the path
      const actualPath = cmd.getOutputOnly().trim()
      const normalizedActual = actualPath.replace('/private/var/', '/var/')
      const normalizedExpected = tempDir.replace('/private/var/', '/var/')
      expect(normalizedActual).toBe(normalizedExpected)
    })

    it('should create files in working directory', async () => {
      const testFile = 'test-file.txt'
      const cmd = new ShellCommand({ workingDirectory: tempDir })
      await cmd.execute(`echo "Test content" > ${testFile}`)

      expect(cmd.getStatus()).toBe('completed')

      // Check if file was created
      const filePath = path.join(tempDir, testFile)
      const fileExists = await fs
        .access(filePath)
        .then(() => true)
        .catch(() => false)
      expect(fileExists).toBe(true)

      const content = await fs.readFile(filePath, 'utf-8')
      expect(content).toContain('Test content')
    })
  })

  describe('Environment Variables', () => {
    it('should use custom environment variables', async () => {
      const cmd = new ShellCommand({
        env: { TEST_VAR: 'custom-value' },
      })
      await cmd.execute('echo "Value: $TEST_VAR"')

      expect(cmd.getStatus()).toBe('completed')
      expect(cmd.getOutputOnly()).toContain('Value: custom-value')
    })

    it('should inherit safe system environment variables', async () => {
      // Test that PATH is inherited (part of safe system environment)
      const cmd = new ShellCommand({})
      await cmd.execute('echo "PATH: $PATH"', {})

      expect(cmd.getStatus()).toBe('completed')
      expect(cmd.getOutputOnly()).toContain('PATH:')
      // PATH should have some value (not empty)
      expect(cmd.getOutputOnly().trim()).not.toBe('PATH:')
    })
  })

  describe('Log Management', () => {
    it('should persist logs to file', async () => {
      const cmd = new ShellCommand({ logFile })
      await cmd.execute('echo "Line 1" && echo "Line 2" && echo "Line 3"')

      expect(cmd.getStatus()).toBe('completed')

      // Wait a bit for async log writes to complete
      await new Promise((resolve) => setTimeout(resolve, 100))

      // Check if log file was created and contains output
      const logContent = await fs.readFile(logFile, 'utf-8')
      expect(logContent).toContain('Line 1')
      expect(logContent).toContain('Line 2')
      expect(logContent).toContain('Line 3')
      expect(logContent).toContain('[stdout]')
      expect(logContent).toContain('[system] Command started')
      expect(logContent).toContain('[system] Command completed')
    })

    it('should limit log retrieval with lines parameter', async () => {
      const cmd = new ShellCommand({})
      await cmd.execute('for i in {1..10}; do echo "Line $i"; done', {})

      const allLogs = await cmd.getLogs()
      const limitedLogs = await cmd.getLogs(5)

      // The limited logs should be less than or equal to 5
      // (may include system logs for start and end)
      expect(limitedLogs.length).toBeLessThanOrEqual(5)
      // Only check if there are more than 5 logs total
      if (allLogs.length > 5) {
        expect(limitedLogs.length).toBeLessThan(allLogs.length)
      }
    })

    it('should stream logs from running command', async () => {
      const cmd = new ShellCommand({})
      // Don't await - start command and stream logs concurrently
      const promise = cmd.execute('for i in {1..3}; do echo "Line $i"; sleep 0.1; done', {})

      const streamedLogs: string[] = []
      const logGenerator = cmd.streamLogs()

      // Collect logs for a short time
      const timeout = setTimeout(() => undefined, 1000)

      for await (const log of logGenerator) {
        streamedLogs.push(log.text)
        if (streamedLogs.length > 10) break // Safety limit
      }

      clearTimeout(timeout)

      // Wait for command to complete
      await promise

      // Should have captured some logs
      expect(streamedLogs.length).toBeGreaterThan(0)
      expect(streamedLogs.some((log) => log.includes('Line 1'))).toBe(true)
    })

    it('should format logs correctly', async () => {
      const cmd = new ShellCommand({})
      await cmd.execute('echo "Test output"')

      const formattedLogs = cmd.getFormattedLogs(true)
      expect(formattedLogs).toContain('[system]')
      expect(formattedLogs).toContain('[stdout]')
      expect(formattedLogs).toContain('Test output')

      const noSystemLogs = cmd.getFormattedLogs(false)
      expect(noSystemLogs).not.toContain('[system]')
      expect(noSystemLogs).toContain('[stdout]')
    })
  })

  describe('Command Sequences', () => {
    it('should execute sequence of commands', async () => {
      const cmd = new ShellCommand({})
      await cmd.execute(['echo "First"', 'echo "Second"', 'echo "Third"'], {})

      expect(cmd.getStatus()).toBe('completed')
      const output = cmd.getOutputOnly()
      expect(output).toContain('First')
      expect(output).toContain('Second')
      expect(output).toContain('Third')
    })

    it('should use custom separator for sequences', async () => {
      const cmd = new ShellCommand({})
      await cmd.execute(['echo "A"', 'echo "B"'], {
        separator: ' ; ',
      })

      expect(cmd.getStatus()).toBe('completed')
      const output = cmd.getOutputOnly()
      expect(output).toContain('A')
      expect(output).toContain('B')
    })

    it('should filter empty commands in sequence', async () => {
      const cmd = new ShellCommand({})
      await cmd.execute(['echo "Valid"', '', '  ', 'echo "Also valid"'], {})

      expect(cmd.getStatus()).toBe('completed')
      const output = cmd.getOutputOnly()
      expect(output).toContain('Valid')
      expect(output).toContain('Also valid')
    })

    it('should throw error for empty sequence', async () => {
      const cmd = new ShellCommand({})
      await expect(cmd.execute(['', '  '], {})).rejects.toThrow('No valid commands provided')
    })
  })

  describe('Execute with Result', () => {
    it('should return structured result', async () => {
      const cmd = new ShellCommand({})
      const result = await cmd.executeWithResult('echo "Success output"')

      expect(result.success).toBe(true)
      expect(result.exitCode).toBe(0)
      expect(result.output).toContain('Success output')
      expect(result.durationMs).toBeGreaterThan(0)
      expect(result.command).toBe('echo "Success output"')
      expect(result.error).toBeUndefined()
    })

    it('should return error in result for failed command', async () => {
      const cmd = new ShellCommand({})
      const result = await cmd.executeWithResult('exit 42')

      expect(result.success).toBe(false)
      expect(result.exitCode).toBe(42)
      expect(result.durationMs).toBeGreaterThan(0)
    })
  })

  describe('Platform-specific Features', () => {
    it('should handle shell selection based on platform', async () => {
      const cmd = new ShellCommand({})
      await cmd.execute('echo "Platform test"')

      if (process.platform === 'win32') {
        expect(cmd.getShell()).toContain('cmd.exe')
      } else {
        expect(cmd.getShell()).toContain('/bin/bash')
      }
    })

    it('should handle pipes and redirects', async () => {
      const cmd = new ShellCommand({})
      await cmd.execute('echo "Test" | tr "T" "t"')

      expect(cmd.getStatus()).toBe('completed')
      expect(cmd.getOutputOnly()).toContain('test')
    })

    it('should handle shell built-ins', async () => {
      const cmd = new ShellCommand({})

      if (process.platform !== 'win32') {
        // Test bash built-ins
        await cmd.execute('for i in 1 2 3; do echo $i; done', {})
        const output = cmd.getOutputOnly()
        expect(output).toContain('1')
        expect(output).toContain('2')
        expect(output).toContain('3')
      } else {
        // Test Windows built-ins
        await cmd.execute('for %i in (1 2 3) do @echo %i', {})
        const output = cmd.getOutputOnly()
        expect(output).toContain('1')
        expect(output).toContain('2')
        expect(output).toContain('3')
      }
    })
  })

  describe('NVM Integration', () => {
    it('should include nvm setup in commands (non-Windows)', async () => {
      if (process.platform === 'win32') {
        // Skip on Windows
        return
      }

      const cmd = new ShellCommand({})
      await cmd.execute('echo $NVM_DIR')

      // The command should have access to NVM_DIR
      const output = cmd.getOutputOnly()
      // NVM_DIR should be set (even if nvm is not installed, the variable is exported)
      expect(output).toBeTruthy()
    })
  })

  describe('Edge Cases', () => {
    it('should handle very long output', async () => {
      const cmd = new ShellCommand({})
      // Generate a lot of output
      await cmd.execute(
        'for i in {1..1000}; do echo "Line $i: This is a test line with some content"; done',
        {},
      )

      expect(cmd.getStatus()).toBe('completed')
      const output = cmd.getOutputOnly()
      expect(output).toContain('Line 1:')
      expect(output).toContain('Line 1000:')
    })

    it('should handle special characters in output', async () => {
      const cmd = new ShellCommand({})
      // Use single quotes to prevent shell variable expansion
      await cmd.execute("echo 'Special chars: $@#%^&*()'", {})

      expect(cmd.getStatus()).toBe('completed')
      expect(cmd.getOutputOnly()).toContain('Special chars: $@#%^&*()')
    })

    it('should handle empty output', async () => {
      const cmd = new ShellCommand({})
      await cmd.execute('true') // Command that produces no output

      expect(cmd.getStatus()).toBe('completed')
      expect(cmd.getOutputOnly()).toBe('')
      expect(cmd.getExitCode()).toBe(0)
    })

    it('should not allow executing a command more than once', async () => {
      const cmd = new ShellCommand({})
      await cmd.execute('echo "First"')

      expect(cmd.getStatus()).toBe('completed')
      expect(cmd.getOutputOnly()).toContain('First')

      // Second execution should fail
      await expect(cmd.execute('echo "Second"')).rejects.toThrow(
        'ShellCommand can only be executed once',
      )
    })

    it('should not allow executeWithResult more than once', async () => {
      const cmd = new ShellCommand({})
      const result1 = await cmd.executeWithResult('echo "First"')

      expect(result1.success).toBe(true)
      expect(result1.output).toContain('First')

      // Second execution should fail
      await expect(cmd.executeWithResult('echo "Second"')).rejects.toThrow(
        'ShellCommand can only be executed once',
      )
    })

    it('should not allow executeSequence more than once', async () => {
      const cmd = new ShellCommand({})
      await cmd.execute(['echo "First"'], {})

      expect(cmd.getStatus()).toBe('completed')

      // Second execution should fail
      await expect(cmd.execute(['echo "Second"'], {})).rejects.toThrow(
        'ShellCommand can only be executed once',
      )
    })
  })
})
