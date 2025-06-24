import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { ProcessManager } from '../process-utils.js'

describe('ProcessManager', () => {
  let processManager: ProcessManager

  beforeEach(() => {
    processManager = new ProcessManager()
  })

  afterEach(async () => {
    await processManager.cleanup()
  })

  describe('execute', () => {
    it('should execute command and return result', async () => {
      const result = await processManager.execute('echo', ['Hello, World!'])

      expect(result.exitCode).toBe(0)
      expect(result.stdout.trim()).toBe('Hello, World!')
      expect(result.stderr).toBe('')
      expect(result.timedOut).toBe(false)
    })

    it('should handle command errors', async () => {
      try {
        await processManager.execute('nonexistentcommand123', [])
        expect.fail('Should have thrown an error')
      } catch (error) {
        expect(error).toBeDefined()
      }
    })

    it('should handle timeout', async () => {
      const result = await processManager.execute('sleep', ['5'], {
        timeout: 100, // 100ms timeout
      })

      expect(result.timedOut).toBe(true)
    })

    it('should emit stdout events', async () => {
      const stdoutData: string[] = []

      processManager.on('stdout', (_processId, data) => {
        stdoutData.push(data)
      })

      await processManager.execute('echo', ['Line 1\nLine 2'])

      expect(stdoutData.length).toBeGreaterThan(0)
      expect(stdoutData.join('')).toContain('Line 1')
    })
  })

  describe('executeShell', () => {
    it('should execute shell command', async () => {
      const result = await processManager.executeShell('echo "Hello from shell"')

      expect(result.exitCode).toBe(0)
      expect(result.stdout.trim()).toBe('Hello from shell')
    })

    it('should handle shell command errors', async () => {
      const result = await processManager.executeShell('exit 42')

      expect(result.exitCode).toBe(42)
    })
  })

  describe('getActiveProcessCount', () => {
    it('should track active processes', async () => {
      expect(processManager.getActiveProcessCount()).toBe(0)

      // Start a long-running process
      const promise = processManager.execute('sleep', ['1'])

      // Give it time to start
      await new Promise((resolve) => setTimeout(resolve, 50))

      expect(processManager.getActiveProcessCount()).toBe(1)

      // Wait for completion
      await promise

      expect(processManager.getActiveProcessCount()).toBe(0)
    })
  })

  describe('cleanup', () => {
    it('should kill all active processes', async () => {
      // Start multiple processes
      const promises = [
        processManager.execute('sleep', ['5']),
        processManager.execute('sleep', ['5']),
        processManager.execute('sleep', ['5']),
      ]

      // Give them time to start
      await new Promise((resolve) => setTimeout(resolve, 50))

      expect(processManager.getActiveProcessCount()).toBe(3)

      // Cleanup should kill all
      await processManager.cleanup()

      expect(processManager.getActiveProcessCount()).toBe(0)

      // All promises should resolve (with timeout or kill signal)
      const results = await Promise.all(promises)
      results.forEach((result) => {
        expect(result.timedOut || result.signal).toBeTruthy()
      })
    })
  })
})
