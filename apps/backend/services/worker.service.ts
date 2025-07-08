import { randomUUID } from 'node:crypto'
import type { TaskIteration } from '@moatless/shared/types/task'
import Docker from 'dockerode'
import { createLogger } from '../utils/logger.js'

interface WorkerInfo {
  id: string
  containerId: string
  host: string
  port: number
  apiKey: string
  status: 'starting' | 'ready' | 'busy' | 'stopping'
  tasksExecuted: number
  createdAt: Date
}

export class WorkerService {
  private docker: Docker
  private workers = new Map<string, WorkerInfo>()
  private logger = createLogger()

  constructor() {
    this.docker = new Docker()
  }

  async executeAgentInWorker(
    taskId: string,
    message: string,
    iteration: TaskIteration,
    agentType: string,
    resumeSessionId?: string,
    model?: string,
    repositoryConfig?: any,
  ): Promise<void> {
    const worker = await this.getOrCreateWorker(repositoryConfig)

    this.logger.info('Delegating task to worker', {
      workerId: worker.id,
      taskId,
      iterationNumber: iteration.iterationNumber,
    })

    worker.status = 'busy'

    try {
      const response = await fetch(`http://${worker.host}:${worker.port}/execute-agent`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          Authorization: `Bearer ${worker.apiKey}`,
        },
        body: JSON.stringify({
          taskId,
          message,
          iteration,
          agentType,
          resumeSessionId,
          model,
        }),
      })

      if (!response.ok) {
        const error = await response.text()
        throw new Error(`Worker execution failed: ${error}`)
      }

      worker.tasksExecuted++
      worker.status = 'ready'
    } catch (error) {
      worker.status = 'ready'
      throw error
    }
  }

  private async getOrCreateWorker(repositoryConfig?: any): Promise<WorkerInfo> {
    // Find available worker
    for (const [, worker] of this.workers) {
      if (worker.status === 'ready') {
        return worker
      }
    }

    // Create new worker if none available
    return this.createWorkerContainer(repositoryConfig)
  }

  private async createWorkerContainer(repositoryConfig?: any): Promise<WorkerInfo> {
    const workerId = `worker-${randomUUID().slice(0, 8)}`
    const apiKey = randomUUID()
    const port = 8001 + this.workers.size

    const image = repositoryConfig?.codeExecution?.workerImage || 'moatless/backend:latest'

    this.logger.info('Creating worker container', { workerId, image })

    const container = await this.docker.createContainer({
      Image: image,
      name: workerId,
      Env: [
        'WORKER_MODE=true',
        `WORKER_API_URL=http://host.docker.internal:${process.env.PORT || 8000}`,
        `WORKER_API_KEY=${apiKey}`,
        'WORKSPACE_DIR=/workspace',
        `PORT=${port}`,
      ],
      ExposedPorts: {
        [`${port}/tcp`]: {},
      },
      HostConfig: {
        AutoRemove: true,
        NetworkMode: 'bridge',
        ExtraHosts: ['host.docker.internal:host-gateway'],
        PortBindings: {
          [`${port}/tcp`]: [{ HostPort: `${port}` }],
        },
        Memory: 2 * 1024 * 1024 * 1024, // 2GB
        CpuShares: 1024,
      },
    })

    await container.start()

    const info: WorkerInfo = {
      id: workerId,
      containerId: container.id,
      host: 'localhost',
      port,
      apiKey,
      status: 'starting',
      tasksExecuted: 0,
      createdAt: new Date(),
    }

    this.workers.set(workerId, info)

    // Wait for worker to be ready
    await this.waitForWorkerReady(info)

    return info
  }

  private async waitForWorkerReady(worker: WorkerInfo, maxAttempts = 30): Promise<void> {
    for (let i = 0; i < maxAttempts; i++) {
      try {
        const response = await fetch(`http://${worker.host}:${worker.port}/health`)
        if (response.ok) {
          worker.status = 'ready'
          this.logger.info('Worker is ready', { workerId: worker.id })
          return
        }
      } catch {
        // Worker not ready yet
      }

      await new Promise((resolve) => setTimeout(resolve, 1000))
    }

    throw new Error(`Worker ${worker.id} failed to start`)
  }

  async stopAllWorkers(): Promise<void> {
    this.logger.info('Stopping all workers')

    for (const [id, worker] of this.workers) {
      try {
        const container = this.docker.getContainer(worker.containerId)
        await container.stop({ t: 10 })
      } catch (error) {
        this.logger.error('Failed to stop worker', error as Error, { workerId: id })
      }
    }

    this.workers.clear()
  }
}
