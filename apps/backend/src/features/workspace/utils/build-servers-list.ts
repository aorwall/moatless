import type { ServerCapabilities, ServerEntry } from '@moatless/types'
import { logger } from '../../../utils/logger'
import { loadProxyConfig, type ProxyEnvironmentConfig } from '../../preview/config/proxy-config'

/** Lazy resolve – avoids circular import during container initialisation. */
async function getRepositoryService() {
  const { appContainer } = await import('../../../container/container')
  const { RepositoryService } = await import('../../repositories/services/repository.service')
  return appContainer.get(RepositoryService)
}

async function getPreviewOrchestratorService() {
  const { appContainer } = await import('../../../container/container')
  const { PreviewOrchestratorService } = await import(
    '../../preview/services/preview-orchestrator.service'
  )
  return appContainer.get(PreviewOrchestratorService)
}

/**
 * Build servers list for a task with status, URL, and capabilities.
 *
 * Always derives the server list from repository config.  When
 * `environmentReady` is true each server's live status is checked in
 * parallel (with a 1 s timeout per server).  Otherwise every server
 * gets `status: 'stopped'`.  Proxy URLs are always resolved so the
 * frontend can render tabs immediately.
 */
export async function buildServersList(
  taskId: string,
  repositoryId: string | undefined,
  environmentReady: boolean,
): Promise<ServerEntry[]> {
  if (!repositoryId) {
    return []
  }

  const proxyConfig = loadProxyConfig()

  let repoServers: Array<{
    name: string
    label?: string | undefined
    port?: number | undefined
    default?: boolean | undefined
  }>
  try {
    const repoService = await getRepositoryService()
    const repo = await repoService.getRepositoryById(repositoryId)
    if (!repo?.servers || repo.servers.length === 0) {
      return []
    }
    repoServers = repo.servers
  } catch (err) {
    logger.warn(
      { taskId, repositoryId, err },
      'buildServersList: failed to load repository servers',
    )
    return []
  }

  if (!environmentReady) {
    return repoServers.map((s) => ({
      name: s.name,
      label: s.label ?? s.name,
      port: s.port ?? 0,
      status: 'stopped' as const,
      url: buildProxyUrl(taskId, s.port ?? 0, proxyConfig),
      default: s.default ?? false,
      capabilities: getServerCapabilities(s.name),
    }))
  }

  // Environment is ready — check live status for each server in parallel
  const resolved = await Promise.all(
    repoServers.map((s) => resolveServerStatus(s.name, s.port ?? 0, taskId, proxyConfig)),
  )

  return repoServers.map((s, i) => {
    const r = resolved[i] ?? { status: 'stopped' as const, url: undefined, error: undefined }
    return {
      name: s.name,
      label: s.label ?? s.name,
      port: s.port ?? 0,
      status: r.status,
      url: r.url,
      error: r.error,
      default: s.default ?? false,
      capabilities: getServerCapabilities(s.name),
    }
  })
}

/**
 * Resolve status, url, and error for a single server entry by querying the sandbox.
 */
async function resolveServerStatus(
  name: string,
  port: number,
  taskId: string,
  proxyConfig: ProxyEnvironmentConfig,
): Promise<{ status: ServerEntry['status']; url: string | undefined; error: string | undefined }> {
  // "files" server is a static proxy — always considered started when env is ready
  if (name === 'files') {
    return { status: 'started', url: buildProxyUrl(taskId, port, proxyConfig), error: undefined }
  }

  // For all other servers (preview, storybook, etc.), fetch live status from sandbox
  try {
    const orchestrator = await getPreviewOrchestratorService()
    const serverStatus = await orchestrator.getServerStatus(taskId, name, port)

    const url = serverStatus.url || buildProxyUrl(taskId, port, proxyConfig)

    return {
      status: serverStatus.status as ServerEntry['status'],
      url,
      error: serverStatus.error,
    }
  } catch {
    return {
      status: 'stopped',
      url: buildProxyUrl(taskId, port, proxyConfig),
      error: undefined,
    }
  }
}

function buildProxyUrl(
  taskId: string,
  port: number,
  proxyConfig: ProxyEnvironmentConfig,
): string | undefined {
  if (!(proxyConfig.useHostnameRouting && proxyConfig.proxyDomain)) {
    return undefined
  }
  // Use port number in hostname for deterministic routing (e.g., taskId--3000, taskId--4000)
  const hostPrefix = `${taskId}--${port}`
  return `${proxyConfig.proxyProtocol}://${hostPrefix}.${proxyConfig.proxyDomain}`
}

function getServerCapabilities(serverName: string): ServerCapabilities {
  switch (serverName) {
    case 'files':
      return {
        addressBar: false,
        inspector: false,
        reload: true,
        openExternal: true,
        viewport: false,
        console: false,
      }
    case 'preview':
      return {
        addressBar: true,
        inspector: true,
        reload: true,
        openExternal: true,
        viewport: true,
        console: true,
      }
    default:
      return {
        addressBar: true,
        inspector: false,
        reload: true,
        openExternal: true,
        viewport: false,
        console: false,
      }
  }
}
