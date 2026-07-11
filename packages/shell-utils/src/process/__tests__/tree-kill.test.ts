import { spawn } from 'node:child_process'
import { describe, expect, it } from 'vitest'
import { treeKill, treeKillAsync } from '../tree-kill.js'

describe('tree-kill', () => {
  describe('treeKill (callback-based)', () => {
    it('should kill a simple process', async () => {
      // Spawn a long-running process
      const child = spawn('sleep', ['10'])
      const pid = child.pid

      if (!pid) {
        throw new Error('Failed to get PID')
      }

      // Kill it
      await new Promise<void>((resolve, reject) => {
        treeKill(pid, 'SIGTERM', (error) => {
          if (error) {
            reject(error)
          } else {
            resolve()
          }
        })
      })

      // Wait a bit for the process to be killed
      await new Promise((resolve) => setTimeout(resolve, 100))

      // Try to kill it again - should fail with ESRCH (no such process)
      try {
        process.kill(pid, 0) // Signal 0 checks if process exists
        // If we get here, process still exists (shouldn't happen)
        expect.fail('Process should have been killed')
      } catch (error) {
        const err = error as NodeJS.ErrnoException
        expect(err.code).toBe('ESRCH')
      }
    })

    it('should kill a process tree', async () => {
      // Spawn a process that spawns a child
      const shell = process.platform === 'win32' ? 'cmd.exe' : '/bin/bash'
      const args =
        process.platform === 'win32'
          ? ['/c', 'timeout /t 10']
          : ['-c', 'sleep 10 & sleep 10 & wait']

      const parent = spawn(shell, args)
      const parentPid = parent.pid

      if (!parentPid) {
        throw new Error('Failed to get parent PID')
      }

      // Wait a bit for children to spawn
      await new Promise((resolve) => setTimeout(resolve, 200))

      // Kill the process tree
      await new Promise<void>((resolve, reject) => {
        treeKill(parentPid, 'SIGTERM', (error) => {
          if (error) {
            reject(error)
          } else {
            resolve()
          }
        })
      })

      // Wait for cleanup
      await new Promise((resolve) => setTimeout(resolve, 200))

      // Verify parent is killed
      try {
        process.kill(parentPid, 0)
        expect.fail('Parent process should have been killed')
      } catch (error) {
        const err = error as NodeJS.ErrnoException
        expect(err.code).toBe('ESRCH')
      }
    })

    it('should handle non-existent PID gracefully', async () => {
      const fakePid = 999_999

      await new Promise<void>((resolve) => {
        treeKill(fakePid, 'SIGTERM', (_error) => {
          // Should complete without error (ESRCH is ignored)
          resolve()
        })
      })
    })

    it('should default to SIGTERM when signal not specified', async () => {
      const child = spawn('sleep', ['10'])
      const pid = child.pid

      if (!pid) {
        throw new Error('Failed to get PID')
      }

      await new Promise<void>((resolve) => {
        treeKill(pid, resolve)
      })

      await new Promise((resolve) => setTimeout(resolve, 100))

      try {
        process.kill(pid, 0)
        expect.fail('Process should have been killed')
      } catch (error) {
        const err = error as NodeJS.ErrnoException
        expect(err.code).toBe('ESRCH')
      }
    })

    it('should accept numeric signal', async () => {
      const child = spawn('sleep', ['10'])
      const pid = child.pid

      if (!pid) {
        throw new Error('Failed to get PID')
      }

      await new Promise<void>((resolve, reject) => {
        treeKill(pid, 15, (error) => {
          // Signal 15 is SIGTERM
          if (error) {
            reject(error)
          } else {
            resolve()
          }
        })
      })

      await new Promise((resolve) => setTimeout(resolve, 100))

      try {
        process.kill(pid, 0)
        expect.fail('Process should have been killed')
      } catch (error) {
        const err = error as NodeJS.ErrnoException
        expect(err.code).toBe('ESRCH')
      }
    })
  })

  describe('treeKillAsync (promise-based)', () => {
    it('should kill a process and resolve', async () => {
      const child = spawn('sleep', ['10'])
      const pid = child.pid

      if (!pid) {
        throw new Error('Failed to get PID')
      }

      await treeKillAsync(pid, 'SIGTERM')

      await new Promise((resolve) => setTimeout(resolve, 100))

      try {
        process.kill(pid, 0)
        expect.fail('Process should have been killed')
      } catch (error) {
        const err = error as NodeJS.ErrnoException
        expect(err.code).toBe('ESRCH')
      }
    })

    it('should handle errors as promise rejection', async () => {
      const fakePid = 999_999

      // Should resolve even for non-existent PID (ESRCH is ignored)
      await expect(treeKillAsync(fakePid, 'SIGTERM')).resolves.toBeUndefined()
    })

    it('should default to SIGTERM', async () => {
      const child = spawn('sleep', ['10'])
      const pid = child.pid

      if (!pid) {
        throw new Error('Failed to get PID')
      }

      await treeKillAsync(pid)

      await new Promise((resolve) => setTimeout(resolve, 100))

      try {
        process.kill(pid, 0)
        expect.fail('Process should have been killed')
      } catch (error) {
        const err = error as NodeJS.ErrnoException
        expect(err.code).toBe('ESRCH')
      }
    })

    it('should kill process tree with async version', async () => {
      const shell = process.platform === 'win32' ? 'cmd.exe' : '/bin/bash'
      const args =
        process.platform === 'win32'
          ? ['/c', 'timeout /t 10']
          : ['-c', 'sleep 10 & sleep 10 & wait']

      const parent = spawn(shell, args)
      const parentPid = parent.pid

      if (!parentPid) {
        throw new Error('Failed to get parent PID')
      }

      await new Promise((resolve) => setTimeout(resolve, 200))

      await treeKillAsync(parentPid, 'SIGTERM')

      await new Promise((resolve) => setTimeout(resolve, 200))

      try {
        process.kill(parentPid, 0)
        expect.fail('Parent process should have been killed')
      } catch (error) {
        const err = error as NodeJS.ErrnoException
        expect(err.code).toBe('ESRCH')
      }
    })
  })
})
