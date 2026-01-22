import { beforeEach, describe, expect, test, vi } from 'vitest'

// Create mock API at module level
const mockCoreV1Api = {
  listNamespace: vi.fn(),
  createNamespacedPod: vi.fn(),
  deleteNamespacedPod: vi.fn(),
  listNamespacedPod: vi.fn(),
  readNamespacedPodLog: vi.fn(),
}

const mockKubeConfig = {
  loadFromDefault: vi.fn(),
  loadFromFile: vi.fn(),
  loadFromCluster: vi.fn(),
  makeApiClient: vi.fn(() => mockCoreV1Api),
}

// Mock the @kubernetes/client-node module
vi.mock('@kubernetes/client-node', () => ({
  KubeConfig: vi.fn(() => mockKubeConfig),
  CoreV1Api: vi.fn(),
}))

import { KubernetesService } from '../kubernetes.service'
import type { SettingsService } from '../settings.service'

describe('KubernetesService', () => {
  let service: KubernetesService
  let mockSettingsService: SettingsService

  beforeEach(() => {
    vi.clearAllMocks()

    // Create mock settings service
    mockSettingsService = {
      getKubernetesSettings: vi.fn().mockReturnValue({
        enabled: true,
        namespace: 'test-namespace',
        image: 'moatless/worker:test',
        podStartupTimeoutSeconds: 60,
        healthCheckIntervalSeconds: 10,
        resourceLimits: { cpu: '1', memory: '1Gi' },
        resourceRequests: { cpu: '100m', memory: '256Mi' },
      }),
    } as unknown as SettingsService

    service = new KubernetesService(mockSettingsService)
  })

  describe('generatePodName', () => {
    test('generates valid kubernetes pod name', () => {
      const name = service.generatePodName('my-repo', 'task-123')

      expect(name).toMatch(/^moatless-my-repo-task-123-[a-z0-9]+$/)
      expect(name.length).toBeLessThanOrEqual(63)
    })

    test('sanitizes special characters', () => {
      const name = service.generatePodName('My_Repo.Name', 'task@123')

      expect(name).toMatch(/^moatless-my-repo-name-task-123-[a-z0-9]+$/)
      expect(name).not.toMatch(/[_@.]/)
    })

    test('truncates long names', () => {
      const longRepoName = 'a'.repeat(50)
      const longTaskId = 'b'.repeat(50)
      const name = service.generatePodName(longRepoName, longTaskId)

      expect(name.length).toBeLessThanOrEqual(63)
    })
  })

  describe('isKubernetesAvailable', () => {
    test('returns true when kubernetes is accessible', async () => {
      mockCoreV1Api.listNamespace.mockResolvedValue({ items: [] })

      const available = await service.isKubernetesAvailable()

      expect(available).toBe(true)
      expect(mockKubeConfig.loadFromDefault).toHaveBeenCalled()
    })

    test('returns false when kubernetes fails', async () => {
      mockCoreV1Api.listNamespace.mockRejectedValue(new Error('Connection refused'))

      const available = await service.isKubernetesAvailable()

      expect(available).toBe(false)
    })
  })

  describe('createPod', () => {
    const taskId = 'test-task-123'
    const config = {
      repositoryName: 'test-repo',
      repositoryCloneUrl: 'https://github.com/test/repo.git',
      repositoryBranch: 'main',
      workerApiUrl: 'http://localhost:8000/api/v1',
    }

    test('creates pod with correct configuration', async () => {
      mockCoreV1Api.listNamespace.mockResolvedValue({ items: [] })
      mockCoreV1Api.createNamespacedPod.mockResolvedValue({})

      const result = await service.createPod(taskId, config)

      expect(result.podName).toMatch(/^moatless-test-repo-test-task-123/)
      expect(result.podNamespace).toBe('test-namespace')
      expect(mockCoreV1Api.createNamespacedPod).toHaveBeenCalledWith(
        expect.objectContaining({
          namespace: 'test-namespace',
          body: expect.objectContaining({
            metadata: expect.objectContaining({
              labels: expect.objectContaining({
                'moatless.io/task-id': taskId,
              }),
            }),
          }),
        }),
      )
    })

    test('includes environment variables in pod spec', async () => {
      mockCoreV1Api.listNamespace.mockResolvedValue({ items: [] })
      mockCoreV1Api.createNamespacedPod.mockResolvedValue({})

      await service.createPod(taskId, config)

      const createCall = mockCoreV1Api.createNamespacedPod.mock.calls[0]?.[0] as any
      expect(createCall).toBeDefined()
      const container = createCall.body.spec.containers[0]
      const envNames = container.env.map((e: { name: string }) => e.name)

      expect(envNames).toContain('WORKER_MODE')
      expect(envNames).toContain('WORKER_TASK_ID')
      expect(envNames).toContain('WORKER_API_URL')
      expect(envNames).toContain('GIT_CLONE_URL')
      expect(envNames).toContain('GIT_BRANCH')
    })

    test('throws error when pod creation fails', async () => {
      mockCoreV1Api.listNamespace.mockResolvedValue({ items: [] })
      mockCoreV1Api.createNamespacedPod.mockRejectedValue(new Error('Quota exceeded'))

      await expect(service.createPod(taskId, config)).rejects.toThrow('Failed to create pod')
    })
  })

  describe('deletePod', () => {
    test('deletes pod by task label', async () => {
      mockCoreV1Api.listNamespace.mockResolvedValue({ items: [] })
      mockCoreV1Api.listNamespacedPod.mockResolvedValue({
        items: [{ metadata: { name: 'pod-to-delete' } }],
      })
      mockCoreV1Api.deleteNamespacedPod.mockResolvedValue({})

      await service.deletePod('task-123')

      expect(mockCoreV1Api.listNamespacedPod).toHaveBeenCalledWith(
        expect.objectContaining({
          labelSelector: 'moatless.io/task-id=task-123',
        }),
      )
      expect(mockCoreV1Api.deleteNamespacedPod).toHaveBeenCalledWith(
        expect.objectContaining({
          name: 'pod-to-delete',
        }),
      )
    })

    test('handles no pod found gracefully', async () => {
      mockCoreV1Api.listNamespace.mockResolvedValue({ items: [] })
      mockCoreV1Api.listNamespacedPod.mockResolvedValue({ items: [] })

      // Should not throw
      await expect(service.deletePod('nonexistent')).resolves.toBeUndefined()
    })
  })

  describe('getPodStatus', () => {
    test('returns pod status with IP and phase', async () => {
      mockCoreV1Api.listNamespace.mockResolvedValue({ items: [] })
      mockCoreV1Api.listNamespacedPod.mockResolvedValue({
        items: [
          {
            status: {
              phase: 'Running',
              podIP: '10.0.0.5',
              conditions: [{ type: 'Ready', status: 'True' }],
              containerStatuses: [{ name: 'worker', ready: true, restartCount: 0 }],
            },
          },
        ],
      })

      const status = await service.getPodStatus('task-123')

      expect(status).toEqual({
        phase: 'Running',
        podIp: '10.0.0.5',
        conditions: [{ type: 'Ready', status: 'True' }],
        containerStatuses: [{ name: 'worker', ready: true, restartCount: 0, state: 'unknown' }],
      })
    })

    test('returns null when pod not found', async () => {
      mockCoreV1Api.listNamespace.mockResolvedValue({ items: [] })
      mockCoreV1Api.listNamespacedPod.mockResolvedValue({ items: [] })

      const status = await service.getPodStatus('nonexistent')

      expect(status).toBeNull()
    })
  })

  describe('checkPodHealth', () => {
    test('returns true when health check succeeds', async () => {
      const mockFetch = vi.fn().mockResolvedValue({ ok: true })
      ;(global as any).fetch = mockFetch

      const healthy = await service.checkPodHealth('10.0.0.5', 8001)

      expect(healthy).toBe(true)
      expect(mockFetch).toHaveBeenCalledWith('http://10.0.0.5:8001/health', expect.any(Object))
    })

    test('returns false when health check fails', async () => {
      const mockFetch = vi.fn().mockRejectedValue(new Error('Connection refused'))
      ;(global as any).fetch = mockFetch

      const healthy = await service.checkPodHealth('10.0.0.5', 8001)

      expect(healthy).toBe(false)
    })
  })
})
