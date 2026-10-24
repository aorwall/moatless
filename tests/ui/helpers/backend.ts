/**
 * The REST reads the UI suite observes the backend through. The browser does
 * every action; these only answer questions the UI does not state reliably —
 * whether a turn has finished, which Scripts a task holds — and clean up.
 */

import { type APIRequestContext, expect, request } from '@playwright/test'
import type { UiEnv } from './env'

export interface TaskScript {
  name: string
  command: string
  port: number | null
  scope: string
}

/** A Script as a workspace declares it. */
export interface WorkspaceScript {
  name: string
  command: string
  label?: string | null
  port?: number | null
}

interface LiveStatus {
  sandboxStatus: string
  agentStatus: string
  sandboxError: string | null
}

interface TaskMessage {
  messageType: string
  content: unknown
}

interface TaskSummary {
  taskId: string
  workspaceName: string
  createdAt: string
}

export class Backend {
  private constructor(private readonly http: APIRequestContext) {}

  static async open(env: UiEnv): Promise<Backend> {
    const http = await request.newContext({
      baseURL: env.baseUrl,
      extraHTTPHeaders: { authorization: `Bearer ${env.apiKey}` },
    })
    return new Backend(http)
  }

  async dispose(): Promise<void> {
    await this.http.dispose()
  }

  /** The id T3 thread URLs are rooted at: `/<environment-id>/<task-id>`. */
  async environmentId(): Promise<string> {
    return (await this.json<{ environmentId: string }>('/.well-known/t3/environment')).environmentId
  }

  async task(taskId: string): Promise<{ status: string; title: string; workspaceName: string }> {
    return this.json(`/api/v1/tasks/${taskId}`)
  }

  /**
   * Finds the task a UI send created, by the marker in its first message. The
   * thread id in the URL cannot be used: a backend that mints its own task id
   * leaves the client's id there, and that id names no task.
   */
  async findTaskBySentText(
    project: string,
    marker: string,
    since: Date,
    timeout: number,
  ): Promise<string> {
    let found: string | undefined
    await expect
      .poll(
        async () => {
          const { items } = await this.json<{ items: TaskSummary[] }>('/api/v1/tasks?limit=20')
          const recent = items.filter(
            (t) => t.workspaceName === project && new Date(t.createdAt) >= since,
          )
          for (const task of recent) {
            const { messages } = await this.json<{ messages: TaskMessage[] }>(
              `/api/v1/tasks/${task.taskId}/messages?summary=true`,
            )
            const first = messages.find((m) => m.messageType === 'user')
            if (first && JSON.stringify(first.content).includes(marker)) found = task.taskId
          }
          return found
        },
        { message: `no ${project} task carries the sent marker "${marker}"`, timeout },
      )
      .toBeDefined()
    return found!
  }

  /**
   * Requires the named project's workspace to declare a Script named
   * `script.name`. The API can no longer write Scripts: the project's
   * repository declares them in `.moatless/workspaces.json`, synced with
   * `moat repos sync-config <repo>`.
   */
  async requireWorkspaceScript(project: string, script: WorkspaceScript): Promise<void> {
    const workspaces = await this.json<{ id: string; name: string }[]>('/api/v1/workspaces')
    const workspace = workspaces.find((w) => w.name === project)
    if (!workspace) throw new Error(`No workspace is named ${project}`)
    const { scripts } = await this.json<{ scripts: WorkspaceScript[] }>(
      `/api/v1/workspaces/${workspace.id}`,
    )
    if (scripts.some((s) => s.name === script.name)) return
    throw new Error(
      `Workspace ${project} declares no "${script.name}" Script. Add ${JSON.stringify(script)} ` +
        "to its entry in the repository's .moatless/workspaces.json, commit it, and run " +
        '`moat repos sync-config <repo>`.',
    )
  }

  async scripts(taskId: string): Promise<TaskScript[]> {
    const body = await this.json<{ scripts: TaskScript[] }>(`/api/orchestration/scripts/${taskId}`)
    return body.scripts
  }

  /**
   * Waits until `turns` agent turns have finished. A turn ends with one
   * `result` message; `agentStatus` alone reads `idle` before the first turn
   * starts, so it cannot tell a finished turn from one not yet begun.
   */
  async waitForTurns(taskId: string, turns: number, timeout: number): Promise<void> {
    await expect
      .poll(
        async () => {
          const live = await this.json<LiveStatus>(`/api/sandbox/v1/tasks/${taskId}/live-status`)
          if (live.sandboxError) return `sandbox error: ${live.sandboxError}`
          const { messages } = await this.json<{ messages: TaskMessage[] }>(
            `/api/v1/tasks/${taskId}/messages?summary=true`,
          )
          const results = messages.filter((m) => m.messageType === 'result').length
          return results >= turns && live.agentStatus === 'idle'
            ? 'done'
            : `${results}/${turns} turns, agent ${live.agentStatus}, sandbox ${live.sandboxStatus}`
        },
        {
          message: `the agent did not finish turn ${turns} of task ${taskId}`,
          timeout,
          intervals: [5000],
        },
      )
      .toBe('done')
  }

  /** Closes the task when a failed chain left it open. A missing or closed task is not an error. */
  async closeIfOpen(taskId: string | undefined): Promise<void> {
    if (!taskId) return
    const found = await this.http.get(`/api/v1/tasks/${taskId}`)
    if (!found.ok()) return
    const { status } = (await found.json()) as { status: string }
    if (status !== 'open') return
    const closed = await this.http.post(`/api/v1/tasks/${taskId}/close`, { data: {} })
    if (!closed.ok())
      console.warn(`[ui teardown] closing ${taskId} failed: HTTP ${closed.status()}`)
  }

  private async json<T>(path: string): Promise<T> {
    const response = await this.http.get(path)
    if (!response.ok()) throw new Error(`GET ${path} answered HTTP ${response.status()}`)
    return (await response.json()) as T
  }
}
