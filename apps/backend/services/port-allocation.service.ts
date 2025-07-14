import { promises as fs } from 'node:fs'
import * as net from 'node:net'
import path from 'node:path'
import { createTaskLogger } from '../utils/logger.js'

export interface PortAllocation {
  port: number
  taskId: string
  type: 'preview' | 'worker'
  allocatedAt: Date
  pid?: number
}

export interface PortRange {
  min: number
  max: number
  type: 'preview' | 'worker'
}

export class PortAllocationService {
  private static instance: PortAllocationService
  private logger = createTaskLogger('port-allocation')
  private allocations = new Map<number, PortAllocation>()
  private dataDir: string
  private allocationsFile: string

  private readonly portRanges: PortRange[] = [
    { min: 3000, max: 3999, type: 'preview' },
    { min: 4000, max: 4999, type: 'worker' },
  ]

  private constructor(dataDir: string) {
    this.dataDir = dataDir
    this.allocationsFile = path.join(dataDir, 'port-allocations.json')
  }

  static getInstance(dataDir = 'data'): PortAllocationService {
    if (!PortAllocationService.instance) {
      PortAllocationService.instance = new PortAllocationService(dataDir)
    }
    return PortAllocationService.instance
  }

  async initialize(): Promise<void> {
    await this.ensureDataDir()
    await this.loadAllocations()
    await this.cleanupStaleAllocations()
  }

  async allocatePort(taskId: string, type: 'preview' | 'worker', pid?: number): Promise<number> {
    const range = this.portRanges.find((r) => r.type === type)
    if (!range) {
      throw new Error(`No port range configured for type: ${type}`)
    }

    // Check if we already have an allocation for this task and type
    const existingAllocation = Array.from(this.allocations.values()).find(
      (a) => a.taskId === taskId && a.type === type,
    )

    if (existingAllocation) {
      this.logger.debug(`Reusing existing port allocation for ${taskId}:${type}`, {
        port: existingAllocation.port,
      })
      return existingAllocation.port
    }

    // Find a free port in the range
    for (let port = range.min; port <= range.max; port++) {
      if (!this.allocations.has(port) && (await this.isPortAvailable(port))) {
        const allocation: PortAllocation = {
          port,
          taskId,
          type,
          allocatedAt: new Date(),
          pid,
        }

        this.allocations.set(port, allocation)
        await this.saveAllocations()

        this.logger.info(`Allocated port ${port} for ${taskId}:${type}`, { pid })
        return port
      }
    }

    throw new Error(`No available ports in range ${range.min}-${range.max} for type: ${type}`)
  }

  async releasePort(port: number): Promise<void> {
    const allocation = this.allocations.get(port)
    if (!allocation) {
      this.logger.warn(`Attempted to release unallocated port: ${port}`)
      return
    }

    this.allocations.delete(port)
    await this.saveAllocations()

    this.logger.info(`Released port ${port} for ${allocation.taskId}:${allocation.type}`)
  }

  async releasePortsForTask(taskId: string): Promise<void> {
    const taskAllocations = Array.from(this.allocations.values()).filter((a) => a.taskId === taskId)

    for (const allocation of taskAllocations) {
      this.allocations.delete(allocation.port)
    }

    if (taskAllocations.length > 0) {
      await this.saveAllocations()
      this.logger.info(`Released ${taskAllocations.length} ports for task ${taskId}`)
    }
  }

  getPortForTask(taskId: string, type: 'preview' | 'worker'): number | undefined {
    const allocation = Array.from(this.allocations.values()).find(
      (a) => a.taskId === taskId && a.type === type,
    )
    return allocation?.port
  }

  getAllocations(): PortAllocation[] {
    return Array.from(this.allocations.values())
  }

  async findFreePort(preferredRange?: 'preview' | 'worker'): Promise<number> {
    if (preferredRange) {
      const range = this.portRanges.find((r) => r.type === preferredRange)
      if (range) {
        for (let port = range.min; port <= range.max; port++) {
          if (!this.allocations.has(port) && (await this.isPortAvailable(port))) {
            return port
          }
        }
      }
    }

    // Fallback to any available port
    return this.findAnyFreePort()
  }

  private async findAnyFreePort(): Promise<number> {
    return new Promise((resolve, reject) => {
      const server = net.createServer()
      server.listen(0, '127.0.0.1', () => {
        const address = server.address() as net.AddressInfo
        const port = address.port
        server.close(() => resolve(port))
      })
      server.on('error', reject)
    })
  }

  private async isPortAvailable(port: number): Promise<boolean> {
    return new Promise((resolve) => {
      const client = new net.Socket()
      client.setTimeout(100)

      client.on('connect', () => {
        client.destroy()
        resolve(false) // Port is in use
      })

      client.on('timeout', () => {
        client.destroy()
        resolve(true) // Port is available
      })

      client.on('error', () => {
        resolve(true) // Port is available
      })

      client.connect(port, '127.0.0.1')
    })
  }

  private async ensureDataDir(): Promise<void> {
    try {
      await fs.mkdir(this.dataDir, { recursive: true })
    } catch (error) {
      this.logger.error('Failed to create data directory', error as Error)
      throw error
    }
  }

  private async loadAllocations(): Promise<void> {
    try {
      const data = await fs.readFile(this.allocationsFile, 'utf-8')
      const allocations: PortAllocation[] = JSON.parse(data)

      this.allocations.clear()
      for (const allocation of allocations) {
        // Convert date string back to Date object
        allocation.allocatedAt = new Date(allocation.allocatedAt)
        this.allocations.set(allocation.port, allocation)
      }

      this.logger.debug(`Loaded ${allocations.length} port allocations`)
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
        this.logger.debug('No existing port allocations file found')
      } else {
        this.logger.error('Failed to load port allocations', error as Error)
      }
    }
  }

  private async saveAllocations(): Promise<void> {
    try {
      const allocations = Array.from(this.allocations.values())
      await fs.writeFile(this.allocationsFile, JSON.stringify(allocations, null, 2))
    } catch (error) {
      this.logger.error('Failed to save port allocations', error as Error)
    }
  }

  private async cleanupStaleAllocations(): Promise<void> {
    const staleAllocations: number[] = []
    const maxAge = 24 * 60 * 60 * 1000 // 24 hours

    for (const [port, allocation] of this.allocations.entries()) {
      const age = Date.now() - allocation.allocatedAt.getTime()

      // Check if allocation is too old or if the port is actually available
      if (age > maxAge || (await this.isPortAvailable(port))) {
        staleAllocations.push(port)
      }
    }

    for (const port of staleAllocations) {
      this.allocations.delete(port)
    }

    if (staleAllocations.length > 0) {
      await this.saveAllocations()
      this.logger.info(`Cleaned up ${staleAllocations.length} stale port allocations`)
    }
  }
}
