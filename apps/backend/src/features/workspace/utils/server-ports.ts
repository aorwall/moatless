import type { RepositoryConfig } from '@moatless/types'

export interface ServerMapEntry {
  port: number
  label?: string
}

export interface ServerConfigEntry {
  name: string
  port: number
  label: string | undefined
  builtin: boolean
  startCommands: string[] | undefined
  startupTimeoutSeconds: number | undefined
}

/**
 * Build servers map from repository config.
 * The database schema (autoPopulateServers in repository.schema.ts) is the single source
 * of truth for server configuration, including files and preview servers with correct ports.
 * This function simply converts the servers array to a map format.
 *
 * @returns Record of server name → port and label
 */
export function buildServersMap(repoConfig: RepositoryConfig): Record<string, ServerMapEntry> {
  const servers: Record<string, ServerMapEntry> = {}
  let nextPort = 3000
  const usedPorts = new Set<number>()

  // First pass: collect user-specified ports
  if (repoConfig.servers) {
    for (const server of repoConfig.servers) {
      if (server.port) {
        usedPorts.add(server.port)
      }
    }
  }

  const allocatePort = (): number => {
    while (usedPorts.has(nextPort)) {
      nextPort++
    }
    const port = nextPort++
    usedPorts.add(port)
    return port
  }

  // Process servers from config (DB is single source of truth for server configuration)
  // autoPopulateServers() in repository.schema.ts ensures files/preview servers exist with correct ports
  if (repoConfig.servers) {
    for (const server of repoConfig.servers) {
      servers[server.name] = {
        port: server.port ?? allocatePort(),
        label: server.label,
      }
    }
  }

  return servers
}

/**
 * Build full server config entries with allocated ports for REPO_CONFIG.
 * Includes startCommands and timeout from repository config.
 */
export function buildServersConfig(repoConfig: RepositoryConfig): ServerConfigEntry[] {
  const serversMap = buildServersMap(repoConfig)

  return Object.entries(serversMap).map(([name, { port, label }]) => {
    const userServer = repoConfig.servers?.find((s) => s.name === name)

    // Determine startCommands
    let startCommands: string[] | undefined
    if (name === 'files') {
      startCommands = userServer?.startCommands ?? ['file-explorer']
    } else if (userServer?.startCommands) {
      startCommands = userServer.startCommands
    } else if (name === 'preview' && repoConfig.previewConfig?.serverStartCommands) {
      startCommands = repoConfig.previewConfig.serverStartCommands
    }

    // Determine timeout
    let startupTimeoutSeconds: number | undefined
    if (userServer?.startupTimeoutSeconds) {
      startupTimeoutSeconds = userServer.startupTimeoutSeconds
    } else if (name === 'preview' && repoConfig.previewConfig?.startupTimeoutSeconds) {
      startupTimeoutSeconds = repoConfig.previewConfig.startupTimeoutSeconds
    }

    return {
      name,
      port,
      label,
      builtin: name === 'files',
      startCommands,
      startupTimeoutSeconds,
    }
  })
}
