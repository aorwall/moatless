import type { FastifyInstance } from 'fastify'
import type { Environment } from '../../shared/types/index.js'
import type { StorageBackend } from './storage.js'

export class EnvironmentService {
  constructor(private storage: StorageBackend) {}

  async getEnvironment(envId: string): Promise<Environment | null> {
    return this.storage.getEnvironment(envId)
  }

  async listEnvironments(): Promise<Environment[]> {
    const envKeys = await this.storage.list('environments/')
    const environments: Environment[] = []

    for (const key of envKeys) {
      const envId = key.replace('environments/', '')
      const env = await this.storage.getEnvironment(envId)
      if (env) environments.push(env)
    }

    return environments
  }

  async createEnvironment(env: Environment): Promise<void> {
    await this.storage.saveEnvironment(env)
  }

  async updateEnvironment(envId: string, updates: Partial<Environment>): Promise<void> {
    const env = await this.getEnvironment(envId)
    if (!env) {
      throw new Error(`Environment ${envId} not found`)
    }

    const updatedEnv = { ...env, ...updates, updatedAt: new Date() }
    await this.storage.saveEnvironment(updatedEnv)
  }

  async deleteEnvironment(envId: string): Promise<void> {
    await this.storage.delete(`environments/${envId}`)
  }
}

export function createEnvironmentService(
  _app: FastifyInstance,
  storage: StorageBackend,
): EnvironmentService {
  return new EnvironmentService(storage)
}
