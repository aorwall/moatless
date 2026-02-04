/**
 * ServerRegistry - Manages multiple ServerProcessMonitor instances
 *
 * Provides a central registry for all server processes in the sandbox,
 * including the preview server and file explorer.
 */

import type { SandboxConfig } from '../../../config/environment'
import type { SandboxEventService } from '../../events/services/sandbox-event.service'
import type { PreviewServerStatus } from '../../preview/types/index'
import { ServerProcessMonitor } from './server-process-monitor'

export interface ServerInfo {
  name: string
  label: string
  port: number
  builtin: boolean
  status: PreviewServerStatus
}

export class ServerRegistry {
  private readonly monitors = new Map<string, ServerProcessMonitor>()
  private readonly labels = new Map<string, string>()
  private readonly builtinFlags = new Map<string, boolean>()

  constructor(
    private readonly config: SandboxConfig,
    private readonly eventService?: SandboxEventService,
  ) {}

  /**
   * Register a server by name and start monitoring it
   */
  register(name: string, options?: { label?: string; builtin?: boolean }): ServerProcessMonitor {
    if (this.monitors.has(name)) {
      return this.monitors.get(name)!
    }

    const monitor = new ServerProcessMonitor(name, this.config, this.eventService)
    this.monitors.set(name, monitor)
    this.labels.set(name, options?.label ?? name)
    this.builtinFlags.set(name, options?.builtin ?? false)

    monitor.startWatching()
    return monitor
  }

  /**
   * Get a monitor by server name
   */
  get(name: string): ServerProcessMonitor | undefined {
    return this.monitors.get(name)
  }

  /**
   * Get all registered monitors
   */
  getAll(): Map<string, ServerProcessMonitor> {
    return this.monitors
  }

  /**
   * Get status for all registered servers
   */
  async getAllStatuses(): Promise<ServerInfo[]> {
    const result: ServerInfo[] = []
    for (const [name, monitor] of this.monitors) {
      result.push({
        name,
        label: this.labels.get(name) ?? name,
        port: await monitor.getConfiguredPort(),
        builtin: this.builtinFlags.get(name) ?? false,
        status: await monitor.getStatus(),
      })
    }
    return result
  }

  /**
   * Stop all monitors
   */
  stopAll(): void {
    for (const monitor of this.monitors.values()) {
      monitor.stopWatching()
    }
  }
}

// Module-scoped registry instance
let registryInstance: ServerRegistry | undefined

interface RepoConfigServer {
  name: string
  label?: string
  builtin?: boolean
}

/**
 * Initialize the server registry with servers from REPO_CONFIG env var.
 * Falls back to default servers (files + preview) if no config is found.
 */
export function initializeServerRegistry(
  config: SandboxConfig,
  eventService?: SandboxEventService,
): ServerRegistry {
  registryInstance = new ServerRegistry(config, eventService)

  // Read servers from REPO_CONFIG env var
  const repoConfigJson = process.env['REPO_CONFIG']
  if (repoConfigJson) {
    try {
      const repoConfig = JSON.parse(repoConfigJson) as { servers?: RepoConfigServer[] }
      const servers = repoConfig.servers ?? []
      if (servers.length > 0) {
        for (const server of servers) {
          registryInstance.register(server.name, {
            label: server.label,
            builtin: server.builtin ?? false,
          })
        }
        return registryInstance
      }
    } catch {
      // Fall through to defaults
    }
  }

  // No config or empty servers: register defaults
  registryInstance.register('files', { label: 'File Explorer', builtin: true })
  registryInstance.register('preview', { label: 'Preview', builtin: false })

  return registryInstance
}

/**
 * Get the registry instance (or undefined if not initialized)
 */
export function getRegistryOrUndefined(): ServerRegistry | undefined {
  return registryInstance
}
