import { promises as fs } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { ShellCommand } from '../shell-command.js'

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
      const cmd = new ShellCommand()
      await cmd.execute('echo "Hello, World!"', { background: false })

      expect(cmd.getStatus()).toBe('completed')
      expect(cmd.getOutputOnly()).toContain('Hello, World!')

      const info = cmd.getInfo()
      expect(info).toBeTruthy()
      expect(info?.exitCode).toBe(0)
    })

    it('should execute multiple commands with &&', async () => {
      const cmd = new ShellCommand()
      await cmd.execute('echo "First" && echo "Second" && echo "Third"', { background: false })

      expect(cmd.getStatus()).toBe('completed')
      const output = cmd.getOutputOnly()
      expect(output).toContain('First')
      expect(output).toContain('Second')
      expect(output).toContain('Third')
    })

    it('should handle command failure', async () => {
      const cmd = new ShellCommand()
      await cmd.execute('nonexistentcommand123', { background: false })

      expect(cmd.getStatus()).toBe('failed')
      expect(cmd.getInfo()?.exitCode).not.toBe(0)
    })

    it('should capture stderr output', async () => {
      const cmd = new ShellCommand()
      // Use a command that writes to stderr
      await cmd.execute('echo "Error message" >&2', { background: false })

      const logs = await cmd.getLogs()
      const stderrLog = logs.find((log) => log.type === 'stderr')
      expect(stderrLog).toBeTruthy()
      expect(stderrLog?.text).toContain('Error message')
    })
  })

  describe('Background Execution', () => {
    it('should execute command in background', async () => {
      const cmd = new ShellCommand()
      await cmd.execute('sleep 0.5 && echo "Done"', { background: true })

      // Command should be running immediately after execute
      expect(cmd.getStatus()).toBe('running')
      expect(cmd.isRunning()).toBe(true)
      expect(cmd.getPid()).toBeTruthy()

      // Wait for completion
      await new Promise((resolve) => setTimeout(resolve, 1000))

      expect(cmd.getStatus()).toBe('completed')
      expect(cmd.getOutputOnly()).toContain('Done')
    })

    it('should handle multiple background commands', async () => {
      const cmd1 = new ShellCommand()
      const cmd2 = new ShellCommand()

      await cmd1.execute('sleep 0.2 && echo "Command 1"', { background: true })
      await cmd2.execute('sleep 0.1 && echo "Command 2"', { background: true })

      expect(cmd1.isRunning()).toBe(true)
      expect(cmd2.isRunning()).toBe(true)

      // Wait for both to complete
      await new Promise((resolve) => setTimeout(resolve, 500))

      expect(cmd1.getStatus()).toBe('completed')
      expect(cmd2.getStatus()).toBe('completed')
      expect(cmd1.getOutputOnly()).toContain('Command 1')
      expect(cmd2.getOutputOnly()).toContain('Command 2')
    })
  })

  describe('Process Management', () => {
    it('should kill a running command', async () => {
      const cmd = new ShellCommand()
      await cmd.execute('sleep 10', { background: true })

      expect(cmd.isRunning()).toBe(true)

      const killed = await cmd.kill()
      expect(killed).toBe(true)

      // Give it a moment to register the kill
      await new Promise((resolve) => setTimeout(resolve, 100))

      expect(cmd.getStatus()).toBe('killed')
      expect(cmd.isRunning()).toBe(false)
    })

    it('should handle timeout', async () => {
      const cmd = new ShellCommand()
      await cmd.execute('sleep 10', {
        background: false,
        timeout: 500, // 500ms timeout
      })

      expect(cmd.getStatus()).toBe('killed')
      expect(cmd.getInfo()?.error).toContain('timed out')
    })

    it('should not allow executing when already running', async () => {
      const cmd = new ShellCommand()
      await cmd.execute('sleep 1', { background: true })

      // Now that ShellCommand is single-use, it will throw the single-use error
      await expect(cmd.execute('echo "Second"')).rejects.toThrow(
        'ShellCommand can only be executed once',
      )

      await cmd.kill()
    })
  })

  describe('Working Directory', () => {
    it('should execute command in specified directory', async () => {
      const cmd = new ShellCommand()
      await cmd.execute('pwd', {
        background: false,
        workingDirectory: tempDir,
      })

      expect(cmd.getStatus()).toBe('completed')
      // On macOS, /var is symlinked to /private/var, so we normalize the path
      const actualPath = cmd.getOutputOnly().trim()
      const normalizedActual = actualPath.replace('/private/var/', '/var/')
      const normalizedExpected = tempDir.replace('/private/var/', '/var/')
      expect(normalizedActual).toBe(normalizedExpected)
    })

    it('should create files in working directory', async () => {
      const testFile = 'test-file.txt'
      const cmd = new ShellCommand()
      await cmd.execute(`echo "Test content" > ${testFile}`, {
        background: false,
        workingDirectory: tempDir,
      })

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
      const cmd = new ShellCommand()
      await cmd.execute('echo "Value: $TEST_VAR"', {
        background: false,
        env: { TEST_VAR: 'custom-value' },
      })

      expect(cmd.getStatus()).toBe('completed')
      expect(cmd.getOutputOnly()).toContain('Value: custom-value')
    })

    it('should inherit process environment', async () => {
      // Set a temporary environment variable
      process.env.INHERITED_VAR = 'inherited-value'

      const cmd = new ShellCommand()
      await cmd.execute('echo "Inherited: $INHERITED_VAR"', {
        background: false,
      })

      expect(cmd.getStatus()).toBe('completed')
      expect(cmd.getOutputOnly()).toContain('Inherited: inherited-value')

      // Clean up
      delete process.env.INHERITED_VAR
    })
  })

  describe('Log Management', () => {
    it('should persist logs to file', async () => {
      const cmd = new ShellCommand()
      await cmd.execute('echo "Line 1" && echo "Line 2" && echo "Line 3"', {
        background: false,
        logFile,
      })

      expect(cmd.getStatus()).toBe('completed')

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
      const cmd = new ShellCommand()
      await cmd.execute('for i in {1..10}; do echo "Line $i"; done', {
        background: false,
      })

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
      const cmd = new ShellCommand()
      await cmd.execute('for i in {1..3}; do echo "Line $i"; sleep 0.1; done', {
        background: true,
      })

      const streamedLogs: string[] = []
      const logGenerator = cmd.streamLogs()

      // Collect logs for a short time
      const timeout = setTimeout(() => {}, 1000)

      for await (const log of logGenerator) {
        streamedLogs.push(log.text)
        if (streamedLogs.length > 10) break // Safety limit
      }

      clearTimeout(timeout)

      // Should have captured some logs
      expect(streamedLogs.length).toBeGreaterThan(0)
      expect(streamedLogs.some((log) => log.includes('Line 1'))).toBe(true)
    })

    it('should format logs correctly', async () => {
      const cmd = new ShellCommand()
      await cmd.execute('echo "Test output"', { background: false })

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
      const cmd = new ShellCommand()
      await cmd.executeSequence(['echo "First"', 'echo "Second"', 'echo "Third"'], {
        background: false,
      })

      expect(cmd.getStatus()).toBe('completed')
      const output = cmd.getOutputOnly()
      expect(output).toContain('First')
      expect(output).toContain('Second')
      expect(output).toContain('Third')
    })

    it('should use custom separator for sequences', async () => {
      const cmd = new ShellCommand()
      await cmd.executeSequence(['echo "A"', 'echo "B"'], { background: false, separator: ' ; ' })

      expect(cmd.getStatus()).toBe('completed')
      const output = cmd.getOutputOnly()
      expect(output).toContain('A')
      expect(output).toContain('B')
    })

    it('should filter empty commands in sequence', async () => {
      const cmd = new ShellCommand()
      await cmd.executeSequence(['echo "Valid"', '', '  ', 'echo "Also valid"'], {
        background: false,
      })

      expect(cmd.getStatus()).toBe('completed')
      const output = cmd.getOutputOnly()
      expect(output).toContain('Valid')
      expect(output).toContain('Also valid')
    })

    it('should throw error for empty sequence', async () => {
      const cmd = new ShellCommand()
      await expect(cmd.executeSequence(['', '  '], { background: false })).rejects.toThrow(
        'No valid commands provided',
      )
    })
  })

  describe('Execute with Result', () => {
    it('should return structured result', async () => {
      const cmd = new ShellCommand()
      const result = await cmd.executeWithResult('echo "Success output"')

      expect(result.success).toBe(true)
      expect(result.exitCode).toBe(0)
      expect(result.output).toContain('Success output')
      expect(result.durationMs).toBeGreaterThan(0)
      expect(result.command).toBe('echo "Success output"')
      expect(result.error).toBeUndefined()
    })

    it('should return error in result for failed command', async () => {
      const cmd = new ShellCommand()
      const result = await cmd.executeWithResult('exit 42')

      expect(result.success).toBe(false)
      expect(result.exitCode).toBe(42)
      expect(result.durationMs).toBeGreaterThan(0)
    })
  })

  describe('Platform-specific Features', () => {
    it('should handle shell selection based on platform', async () => {
      const cmd = new ShellCommand()
      await cmd.execute('echo "Platform test"', { background: false })

      const info = cmd.getInfo()
      if (process.platform === 'win32') {
        expect(info?.shell).toContain('cmd.exe')
      } else {
        expect(info?.shell).toContain('/bin/bash')
      }
    })

    it('should handle pipes and redirects', async () => {
      const cmd = new ShellCommand()
      await cmd.execute('echo "Test" | tr "T" "t"', { background: false })

      expect(cmd.getStatus()).toBe('completed')
      expect(cmd.getOutputOnly()).toContain('test')
    })

    it('should handle shell built-ins', async () => {
      const cmd = new ShellCommand()

      if (process.platform !== 'win32') {
        // Test bash built-ins
        await cmd.execute('for i in 1 2 3; do echo $i; done', { background: false })
        const output = cmd.getOutputOnly()
        expect(output).toContain('1')
        expect(output).toContain('2')
        expect(output).toContain('3')
      } else {
        // Test Windows built-ins
        await cmd.execute('for %i in (1 2 3) do @echo %i', { background: false })
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

      const cmd = new ShellCommand()
      await cmd.execute('echo $NVM_DIR', { background: false })

      // The command should have access to NVM_DIR
      const output = cmd.getOutputOnly()
      // NVM_DIR should be set (even if nvm is not installed, the variable is exported)
      expect(output).toBeTruthy()
    })
  })

  describe('Edge Cases', () => {
    it('should handle very long output', async () => {
      const cmd = new ShellCommand()
      // Generate a lot of output
      await cmd.execute(
        'for i in {1..1000}; do echo "Line $i: This is a test line with some content"; done',
        {
          background: false,
        },
      )

      expect(cmd.getStatus()).toBe('completed')
      const output = cmd.getOutputOnly()
      expect(output).toContain('Line 1:')
      expect(output).toContain('Line 1000:')
    })

    it('should handle special characters in output', async () => {
      const cmd = new ShellCommand()
      // Use single quotes to prevent shell variable expansion
      await cmd.execute("echo 'Special chars: $@#%^&*()'", { background: false })

      expect(cmd.getStatus()).toBe('completed')
      expect(cmd.getOutputOnly()).toContain('Special chars: $@#%^&*()')
    })

    it('should handle empty output', async () => {
      const cmd = new ShellCommand()
      await cmd.execute('true', { background: false }) // Command that produces no output

      expect(cmd.getStatus()).toBe('completed')
      expect(cmd.getOutputOnly()).toBe('')
      expect(cmd.getInfo()?.exitCode).toBe(0)
    })

    it('should not allow executing a command more than once', async () => {
      const cmd = new ShellCommand()
      await cmd.execute('echo "First"', { background: false })

      expect(cmd.getStatus()).toBe('completed')
      expect(cmd.getOutputOnly()).toContain('First')

      // Second execution should fail
      await expect(cmd.execute('echo "Second"', { background: false })).rejects.toThrow(
        'ShellCommand can only be executed once',
      )
    })

    it('should not allow executeWithResult more than once', async () => {
      const cmd = new ShellCommand()
      const result1 = await cmd.executeWithResult('echo "First"')

      expect(result1.success).toBe(true)
      expect(result1.output).toContain('First')

      // Second execution should fail
      await expect(cmd.executeWithResult('echo "Second"')).rejects.toThrow(
        'ShellCommand can only be executed once',
      )
    })

    it('should not allow executeSequence more than once', async () => {
      const cmd = new ShellCommand()
      await cmd.executeSequence(['echo "First"'], { background: false })

      expect(cmd.getStatus()).toBe('completed')

      // Second execution should fail
      await expect(cmd.executeSequence(['echo "Second"'], { background: false })).rejects.toThrow(
        'ShellCommand can only be executed once',
      )
    })
  })
})
