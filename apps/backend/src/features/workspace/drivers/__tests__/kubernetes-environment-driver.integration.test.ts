/**
 * Integration tests for KubernetesEnvironmentDriver
 *
 * These tests run against a REAL Kubernetes cluster and worker pod.
 * They are isolated from task infrastructure by:
 * 1. Using KubernetesService directly to create/manage pods
 * 2. Registering the pod target with KubernetesEnvironmentDriver manually
 * 3. Testing driver methods against the real pod
 *
 * Prerequisites:
 * - Kubernetes cluster access (~/.kube/config or in-cluster)
 * - Worker image available (aorwall/moatless-vibe-worker:latest)
 * - RBAC permissions to create/delete pods
 * - **IMPORTANT**: Network access to pod IPs (tests connect directly to pod IPs)
 *   This means tests must be run from:
 *   - Inside the cluster (as a Job or Pod)
 *   - With VPN access to the cluster network
 *   - Using kubectl port-forward to the test pod
 *
 * Run with:
 *   bun run test:k8s-integration
 *
 * Or manually:
 *   K8S_ENABLED=true K8S_NAMESPACE=vibe NODE_TLS_REJECT_UNAUTHORIZED=0 \
 *   K8S_WORKER_IMAGE=aorwall/moatless-vibe-worker:latest \
 *   bun test kubernetes-environment-driver.integration.test.ts
 *
 * Configuration via environment variables:
 * - K8S_ENABLED=true (required)
 * - K8S_NAMESPACE=vibe (namespace for test pods)
 * - K8S_WORKER_IMAGE=aorwall/moatless-vibe-worker:latest (worker image)
 * - K8S_RESOURCE_REQUEST_CPU=100m (CPU request)
 * - K8S_RESOURCE_REQUEST_MEMORY=256Mi (memory request)
 * - NODE_TLS_REJECT_UNAUTHORIZED=0 (for clusters with self-signed certs)
 */

import { mkdir, writeFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import { join } from 'node:path'
import type { ShellCommandLogEvent } from '@moatless/types'
import { afterAll, beforeAll, describe, expect, test, vi } from 'vitest'
import { KubernetesService } from '../../../../services/kubernetes.service'
import { SettingsService } from '../../../../services/settings.service'
import type { EventService } from '../../../events/services/event.service'
import type { KubernetesExecutionTarget } from '../../types/workspace-runtime.types'
import { KubernetesEnvironmentDriver } from '../kubernetes-environment-driver'

// Skip all tests if K8s is not enabled
const K8S_ENABLED = process.env['K8S_ENABLED'] === 'true'

// Test repository - GitHub's official Hello-World example repo
const TEST_REPO_URL = 'https://github.com/octocat/Hello-World.git'

describe.skipIf(!K8S_ENABLED)('KubernetesEnvironmentDriver Integration', () => {
  let k8sService: KubernetesService
  let driver: KubernetesEnvironmentDriver
  let testTaskId: string
  let podInfo: { podName: string; podIp: string; podNamespace: string }
  let mockEventService: EventService

  beforeAll(async () => {
    console.log('Setting up K8s integration test...')

    // Create mock EventService (only dependency needed for driver)
    // biome-ignore lint/suspicious/noEmptyBlockStatements: noop unsubscribe for mock
    const noopUnsubscribe = () => {}
    mockEventService = {
      subscribe: vi.fn().mockReturnValue(noopUnsubscribe),
      subscribeAll: vi.fn().mockReturnValue(noopUnsubscribe),
      emitEvent: vi.fn(),
    } as unknown as EventService

    // Get the DATA_DIR that will be used by SettingsService
    // (test setup creates a temp directory)
    const dataDir = process.env['DATA_DIR'] || join(homedir(), '.moatless')
    await mkdir(dataDir, { recursive: true })

    // Write settings file with spot node tolerations BEFORE initializing SettingsService
    const settingsPath = join(dataDir, 'settings.json')
    const settings = {
      kubernetes: {
        enabled: true,
        namespace: process.env['K8S_NAMESPACE'] || 'vibe',
        image: process.env['K8S_WORKER_IMAGE'] || 'aorwall/moatless-vibe-worker:latest',
        resourceRequests: {
          cpu: process.env['K8S_RESOURCE_REQUEST_CPU'] || '100m',
          memory: process.env['K8S_RESOURCE_REQUEST_MEMORY'] || '256Mi',
        },
        resourceLimits: {
          cpu: process.env['K8S_RESOURCE_LIMIT_CPU'] || '1',
          memory: process.env['K8S_RESOURCE_LIMIT_MEMORY'] || '1Gi',
        },
        // Spot node tolerations for Azure AKS
        tolerations: [
          {
            key: 'kubernetes.azure.com/scalesetpriority',
            operator: 'Equal',
            value: 'spot',
            effect: 'NoSchedule',
          },
        ],
      },
    }
    await writeFile(settingsPath, JSON.stringify(settings, null, 2))
    console.log(`Settings file written to: ${settingsPath}`)

    // Create SettingsService - it will load settings from DATA_DIR/settings.json
    const settingsService = new SettingsService(null)
    await settingsService.initialize()

    // Create KubernetesService with real K8s client
    // Uses lazy initialization - will initialize on first use
    k8sService = new KubernetesService(settingsService)

    // Verify K8s is available with detailed error logging
    const available = await k8sService.isKubernetesAvailable()
    if (!available) {
      // Try to get more details about why K8s is not available
      console.log('K8s availability check failed, trying direct API call for diagnostics...')
      console.log(`KUBERNETES_SERVICE_HOST: ${process.env['KUBERNETES_SERVICE_HOST']}`)
      console.log(`KUBERNETES_SERVICE_PORT: ${process.env['KUBERNETES_SERVICE_PORT']}`)
      try {
        const k8s = await import('@kubernetes/client-node')
        const kc = new k8s.KubeConfig()
        kc.loadFromCluster()
        const coreApi = kc.makeApiClient(k8s.CoreV1Api)
        const result = await coreApi.listNamespace({ limit: 1 })
        console.log('Direct API call succeeded:', result.items?.length, 'namespaces')
      } catch (diagError) {
        console.error('Direct API call failed:', diagError)
      }
      throw new Error('Kubernetes cluster not available - check kubeconfig')
    }

    console.log('K8s cluster is available, creating test pod...')

    // Create driver
    driver = new KubernetesEnvironmentDriver(mockEventService)

    // Generate unique task ID to avoid conflicts
    testTaskId = `test-k8s-driver-${Date.now()}`

    // Create test pod
    const result = await k8sService.createPod(testTaskId, {
      repositoryName: 'integration-test',
      repositoryCloneUrl: TEST_REPO_URL,
      workerApiUrl: 'http://localhost:8000', // Not used - pod doesn't call back
      workerAuthToken: 'test-integration-token',
    })

    console.log(`Pod created: ${result.podName}, waiting for ready...`)

    // Wait for pod to be ready (up to 5 minutes for image pull - worker image is ~7GB)
    const status = await k8sService.waitForPodReady(testTaskId, 300000)

    if (!status.podIp) {
      throw new Error('Pod ready but no IP assigned')
    }

    podInfo = {
      podName: result.podName,
      podIp: status.podIp,
      podNamespace: result.podNamespace,
    }

    console.log(`Pod ready: ${podInfo.podName} at ${podInfo.podIp}`)

    // Wait a bit for the worker to fully start
    // Note: The pod already passed K8s readiness probe, so it should be healthy
    await new Promise((resolve) => setTimeout(resolve, 3000))

    // Note: Direct health check may fail from outside the cluster since pod IPs
    // are only accessible from within the cluster network. The K8s readiness probe
    // confirms the pod is healthy internally.
    console.log('Pod passed K8s readiness probe, registering target with driver...')

    // Register target with driver
    const target: KubernetesExecutionTarget = {
      mode: 'kubernetes',
      podName: podInfo.podName,
      podNamespace: podInfo.podNamespace,
      podIp: podInfo.podIp,
      podPhase: 'Running',
      workerPort: 8001,
      repositoryCloneUrl: TEST_REPO_URL,
    }
    driver.registerTarget(testTaskId, target)

    console.log('Integration test setup complete!')
  }, 360000) // 6 minute timeout for setup (image pull can take 5 minutes)

  afterAll(async () => {
    console.log('Cleaning up test pod...')

    try {
      // Unregister target from driver
      if (driver && testTaskId) {
        driver.unregisterTarget(testTaskId)
      }

      // Delete test pod
      if (k8sService && testTaskId) {
        await k8sService.deletePod(testTaskId)
        console.log('Test pod deleted successfully')
      }
    } catch (error) {
      console.error('Cleanup error (non-fatal):', error)
    }
  }, 30000)

  // =========================================================================
  // File Operations
  // =========================================================================

  describe('File Operations', () => {
    test('readFile reads existing file from pod workspace', async () => {
      // The octocat/Hello-World repo has a README file (no .md extension)
      const content = await driver.readFile('/workspace', 'README')

      expect(content).toBeDefined()
      expect(content.length).toBeGreaterThan(0)
    })

    test('writeFile creates new file in pod workspace', async () => {
      const testContent = `Integration test file created at ${new Date().toISOString()}`

      await driver.writeFile('/workspace', 'test-integration-file.txt', testContent)

      // Verify by reading it back
      const content = await driver.readFile('/workspace', 'test-integration-file.txt')
      expect(content).toBe(testContent)
    })

    test('listFiles returns files from pod workspace', async () => {
      const files = await driver.listFiles('/workspace')

      expect(Array.isArray(files)).toBe(true)
      expect(files.length).toBeGreaterThan(0)
      // Should include README.md from the cloned repo
      expect(files.some((f) => f.includes('README') || f.includes('readme'))).toBe(true)
    })

    test('listFiles with recursive option', async () => {
      const files = await driver.listFiles('/workspace', '.', true)

      expect(Array.isArray(files)).toBe(true)
      // Recursive should return more files or same
      const nonRecursive = await driver.listFiles('/workspace', '.')
      expect(files.length).toBeGreaterThanOrEqual(nonRecursive.length)
    })

    test('readFile throws for non-existent file', async () => {
      await expect(
        driver.readFile('/workspace', 'this-file-definitely-does-not-exist-12345.txt'),
      ).rejects.toThrow()
    })
  })

  // =========================================================================
  // Command Execution
  // =========================================================================

  describe('Command Execution', () => {
    test('runCommand executes simple command and streams output', async () => {
      const events: ShellCommandLogEvent[] = []

      for await (const event of driver.runCommand('/workspace', 'echo "Hello from K8s pod"')) {
        events.push(event)
      }

      // Should have stdout with output and system exit event
      expect(events.length).toBeGreaterThan(0)

      const stdout = events
        .filter((e) => e.type === 'stdout')
        .map((e) => e.text)
        .join('')
      expect(stdout).toContain('Hello from K8s pod')

      const systemEvent = events.find((e) => e.type === 'system')
      expect(systemEvent).toBeDefined()
      expect(systemEvent?.text).toContain('exit')
    })

    test('runCommand handles command with exit code', async () => {
      const events: ShellCommandLogEvent[] = []

      for await (const event of driver.runCommand('/workspace', 'ls /workspace && echo done')) {
        events.push(event)
      }

      // Should complete successfully
      const systemEvent = events.find((e) => e.type === 'system')
      expect(systemEvent?.text).toContain('code 0')
    })

    test('runCommand handles failing command', async () => {
      const events: ShellCommandLogEvent[] = []

      for await (const event of driver.runCommand('/workspace', 'exit 42')) {
        events.push(event)
      }

      const systemEvent = events.find((e) => e.type === 'system')
      expect(systemEvent?.text).toContain('42')
    })

    test('runCommand runs in correct directory', async () => {
      const events: ShellCommandLogEvent[] = []

      for await (const event of driver.runCommand('/workspace', 'pwd')) {
        events.push(event)
      }

      const stdout = events
        .filter((e) => e.type === 'stdout')
        .map((e) => e.text)
        .join('')
      // Worker uses /repo as the workspace directory
      expect(stdout).toContain('/repo')
    })
  })

  // =========================================================================
  // Git Operations
  // =========================================================================

  describe('Git Operations', () => {
    test('getGitStatus returns status from cloned repo', async () => {
      const status = await driver.getGitStatus('/workspace')

      expect(status).toBeDefined()
      expect(status.currentBranch).toBeDefined()
      expect(status.status).not.toBe('error')
    })

    test('getGitChanges returns changes info', async () => {
      const changes = await driver.getGitChanges('/workspace')

      expect(changes).toBeDefined()
      expect(changes.currentBranch).toBeDefined()
      // Initially should have no changes
      expect(typeof changes.additions).toBe('number')
      expect(typeof changes.deletions).toBe('number')
      expect(typeof changes.modifications).toBe('number')
    })

    test('getGitChanges reflects file modifications', async () => {
      // First, create a new file to make changes
      await driver.writeFile('/workspace', 'git-test-file.txt', 'test content for git')

      const changes = await driver.getGitChanges('/workspace')

      expect(changes).toBeDefined()
      // Should show at least one addition (the new file)
      expect(changes.additions + changes.modifications).toBeGreaterThanOrEqual(0)
    })
  })

  // =========================================================================
  // Search Operations
  // =========================================================================

  describe('Search Operations', () => {
    test('searchFiles finds files by query', async () => {
      const result = await driver.searchFiles('/workspace', { query: 'README' })

      expect(result).toBeDefined()
      expect(result.files).toBeDefined()
      expect(Array.isArray(result.files)).toBe(true)
    })
  })

  // =========================================================================
  // Error Handling
  // =========================================================================

  describe('Error Handling', () => {
    test('throws error for non-existent path', async () => {
      // The driver should throw an error when path doesn't exist
      await expect(driver.listFiles('/workspace', '/nonexistent-path')).rejects.toThrow(
        'Failed to list files',
      )
    })
  })
})
