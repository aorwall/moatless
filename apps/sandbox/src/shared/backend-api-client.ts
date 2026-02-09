import type { SandboxConfig } from '../config/environment'
import { logger } from './logger'

const clientLogger = logger.child({ service: 'backend-api-client' })

export interface BackendApiClient {
  patch(path: string, body: unknown): Promise<Response>
  post(path: string, body: unknown): Promise<Response>
  get(path: string): Promise<Response>
}

/**
 * Create a lightweight backend API client for sandbox-to-backend communication.
 * Returns undefined if backendApiUrl or userAuthToken are not configured (graceful degradation).
 */
export function createBackendApiClient(config: SandboxConfig): BackendApiClient | undefined {
  if (!(config.backendApiUrl && config.userAuthToken)) {
    clientLogger.debug(
      {
        hasBackendApiUrl: !!config.backendApiUrl,
        hasUserAuthToken: !!config.userAuthToken,
      },
      'Backend API client not configured, sandbox-to-backend calls disabled',
    )
    return undefined
  }

  const baseUrl = config.backendApiUrl.replace(/\/+$/, '')
  const token = config.userAuthToken
  const taskId = config.taskId

  clientLogger.info({ baseUrl }, 'Backend API client initialized')

  async function request(method: string, path: string, body?: unknown): Promise<Response> {
    const url = `${baseUrl}${path}`
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
      'X-User-Token': token,
      'X-Task-Id': taskId,
    }

    const response = await fetch(url, {
      method,
      headers,
      body: body !== undefined ? JSON.stringify(body) : undefined,
    })

    if (!response.ok) {
      clientLogger.warn(
        { method, path, status: response.status, statusText: response.statusText },
        'Backend API request failed',
      )
    }

    return response
  }

  return {
    patch: (path, body) => request('PATCH', path, body),
    post: (path, body) => request('POST', path, body),
    get: (path) => request('GET', path),
  }
}
