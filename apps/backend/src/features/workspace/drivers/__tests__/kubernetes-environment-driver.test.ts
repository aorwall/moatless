import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { EventService } from '../../../events/services/event.service'
import type { KubernetesExecutionTarget } from '../../types/workspace-runtime.types'
import { KubernetesEnvironmentDriver } from '../kubernetes-environment-driver'

describe('KubernetesEnvironmentDriver', () => {
  let driver: KubernetesEnvironmentDriver
  let mockFetch: ReturnType<typeof vi.fn>
  let mockEventService: EventService

  const mockTarget: KubernetesExecutionTarget = {
    mode: 'kubernetes',
    podName: 'test-pod',
    podNamespace: 'test-namespace',
    podIp: '10.0.0.5',
    podPhase: 'Running',
    repositoryCloneUrl: 'https://github.com/test/repo.git',
    workerPort: 8001,
  }

  beforeEach(() => {
    mockEventService = {
      subscribe: vi.fn().mockReturnValue(() => {
        /* noop unsubscribe */
      }),
      emitEvent: vi.fn(),
    } as unknown as EventService
    driver = new KubernetesEnvironmentDriver(mockEventService)
    mockFetch = vi.fn()
    ;(global as any).fetch = mockFetch
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  describe('registerTarget / unregisterTarget', () => {
    test('registers and retrieves target', () => {
      driver.registerTarget('task-123', mockTarget)

      const target = driver.getTarget('task-123')

      expect(target).toEqual(mockTarget)
    })

    test('unregisters target', () => {
      driver.registerTarget('task-123', mockTarget)
      driver.unregisterTarget('task-123')

      const target = driver.getTarget('task-123')

      expect(target).toBeUndefined()
    })
  })

  describe('runCommand', () => {
    beforeEach(() => {
      driver.registerTarget('task-123', mockTarget)
    })

    test('sends command to pod and yields stdout', async () => {
      mockFetch.mockResolvedValue({
        json: () =>
          Promise.resolve({
            exit_code: 0,
            stdout: 'command output',
            stderr: '',
          }),
      })

      const events: any[] = []
      for await (const event of driver.runCommand('/workspace', 'echo hello')) {
        events.push(event)
      }

      expect(mockFetch).toHaveBeenCalledWith(
        'http://10.0.0.5:8001/command',
        expect.objectContaining({
          method: 'POST',
          body: expect.stringContaining('"command":"echo hello"'),
        }),
      )
      expect(events).toHaveLength(2) // stdout + system
      expect(events[0]).toMatchObject({ type: 'stdout', text: 'command output' })
      expect(events[1]).toMatchObject({ type: 'system', text: 'Command exited with code 0' })
    })

    test('yields stderr when command fails', async () => {
      mockFetch.mockResolvedValue({
        json: () =>
          Promise.resolve({
            exit_code: 1,
            stdout: '',
            stderr: 'command not found',
          }),
      })

      const events: any[] = []
      for await (const event of driver.runCommand('/workspace', 'invalid')) {
        events.push(event)
      }

      expect(events).toHaveLength(2) // stderr + system
      expect(events[0]).toMatchObject({ type: 'stderr', text: 'command not found' })
      expect(events[1]).toMatchObject({ type: 'system', text: 'Command exited with code 1' })
    })

    test('handles fetch error', async () => {
      mockFetch.mockRejectedValue(new Error('Network error'))

      const events: any[] = []
      for await (const event of driver.runCommand('/workspace', 'echo hello')) {
        events.push(event)
      }

      expect(events).toHaveLength(1)
      expect(events[0]).toMatchObject({ type: 'stderr', text: 'Network error' })
    })

    test('throws when no target registered', async () => {
      driver.unregisterTarget('task-123')

      const generator = driver.runCommand('/workspace', 'echo hello')

      await expect(generator.next()).rejects.toThrow('No task found for workspace path')
    })
  })

  describe('createAgent', () => {
    beforeEach(() => {
      driver.registerTarget('task-123', mockTarget)
    })

    test('creates RemoteAgentProxy with correct config', () => {
      const agent = driver.createAgent('task-123', 'claude-code', { model: 'sonnet' })

      expect(agent.getAgentType()).toBe('claude-code')
    })

    test('throws when no target registered', () => {
      expect(() => driver.createAgent('unknown-task', 'claude-code')).toThrow(
        'No execution target registered',
      )
    })

    test('throws when pod IP not available', () => {
      const targetWithoutIp: KubernetesExecutionTarget = {
        ...mockTarget,
        podIp: undefined,
      }
      driver.registerTarget('task-no-ip', targetWithoutIp)

      expect(() => driver.createAgent('task-no-ip', 'claude-code')).toThrow('Pod IP not available')
    })
  })

  describe('stopAgent', () => {
    beforeEach(() => {
      driver.registerTarget('task-123', mockTarget)
    })

    test('sends stop request to pod', async () => {
      mockFetch.mockResolvedValue({ ok: true })

      await driver.stopAgent('task-123')

      expect(mockFetch).toHaveBeenCalledWith(
        'http://10.0.0.5:8001/stop-agent',
        expect.objectContaining({
          method: 'POST',
          body: expect.stringContaining('"taskId":"task-123"'),
        }),
      )
    })

    test('handles stop failure gracefully', async () => {
      mockFetch.mockRejectedValue(new Error('Connection refused'))

      // Should not throw
      await expect(driver.stopAgent('task-123')).resolves.toBeUndefined()
    })

    test('handles no target gracefully', async () => {
      // Should not throw when no target
      await expect(driver.stopAgent('unknown')).resolves.toBeUndefined()
    })
  })

  describe('readFile', () => {
    beforeEach(() => {
      driver.registerTarget('task-123', mockTarget)
    })

    test('reads file from pod', async () => {
      mockFetch.mockResolvedValue({
        ok: true,
        json: () => Promise.resolve({ content: 'file contents' }),
      })

      const content = await driver.readFile('/workspace', 'package.json')

      expect(content).toBe('file contents')
      expect(mockFetch).toHaveBeenCalledWith(
        'http://10.0.0.5:8001/read-file',
        expect.objectContaining({
          method: 'POST',
          body: expect.stringContaining('"filePath":"package.json"'),
        }),
      )
    })

    test('throws when file not found', async () => {
      mockFetch.mockResolvedValue({
        ok: false,
        statusText: 'Not Found',
        json: () => Promise.resolve({ error: 'File not found' }),
      })

      await expect(driver.readFile('/workspace', 'missing.txt')).rejects.toThrow(
        'Failed to read file: File not found',
      )
    })
  })
})
