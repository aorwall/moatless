/**
 * Removing what a run left behind.
 *
 * Two callers, one implementation. A finished run sweeps its own prefix so a
 * shared environment does not accumulate its tasks. A scheduled sweep removes
 * anything older than a threshold that any run created — that is the part that
 * matters, because a killed run never reaches its own teardown, and a task left
 * open on the cluster keeps a sandbox pod alive.
 *
 * The rule is the same in both cases: a resource is only ever removed when
 * something about it says a run created it. What this module cannot attribute to
 * a run, it does not touch. For most resources that evidence is the name; for
 * tasks it can also be the description or the parent, because the suite is not
 * the only thing that creates tasks on its behalf — see `claimedTasks`.
 */

import type { TaskResponse } from '@moatless/api-client/sandbox/model'
import type { MoatApi } from './moat-api'
import {
  belongsToAnyRun,
  belongsToThisRun,
  runPrefix,
  runStartedAt,
  sandboxTaskId,
} from './run-scope'

export interface SweepOptions {
  /** Only sweep this process's own run. */
  onlyThisRun?: boolean
  /** Skip resources from runs that started less than this long ago. */
  minimumAgeMs?: number
  /** Report what would go, and remove nothing. */
  dryRun?: boolean
  /** Where progress goes. Defaults to the console. */
  log?: (message: string) => void
}

export interface SweptResource {
  kind: 'task' | 'workspace' | 'repository' | 'team' | 'loop' | 'connection'
  id: string
  name: string
}

export interface SweepReport {
  removed: SweptResource[]
  failed: Array<SweptResource & { reason: string }>
  dryRun: boolean
}

/**
 * Remove the suite's leftovers.
 *
 * Order matters: a task holds a sandbox, and a workspace and repository are
 * both referenced by tasks, so tasks go first and repositories last.
 */
export async function sweep(api: MoatApi, options: SweepOptions = {}): Promise<SweepReport> {
  const {
    onlyThisRun = false,
    minimumAgeMs = 0,
    dryRun = false,
    log = (message: string) => console.log(message),
  } = options

  const owned = (name: string | null | undefined): boolean => {
    if (!(onlyThisRun ? belongsToThisRun(name) : belongsToAnyRun(name))) {
      return false
    }
    if (minimumAgeMs <= 0) {
      return true
    }
    const startedAt = runStartedAt(name)
    return startedAt !== null && Date.now() - startedAt.getTime() >= minimumAgeMs
  }

  const report: SweepReport = { removed: [], failed: [], dryRun }

  const record = async (resource: SweptResource, remove: () => Promise<void>): Promise<void> => {
    log(`${dryRun ? '[dry-run] would remove' : 'removing'} ${resource.kind} ${resource.name}`)
    if (dryRun) {
      report.removed.push(resource)
      return
    }
    try {
      await remove()
      report.removed.push(resource)
    } catch (error) {
      report.failed.push({ ...resource, reason: describe(error) })
    }
  }

  // Tasks — stopped before deletion, so the sandbox goes with them.
  for (const task of claimedTasks(await allTasks(api), owned, minimumAgeMs)) {
    await record({ kind: 'task', id: task.taskId, name: task.name ?? task.taskId }, async () => {
      await api.stopTask(task.taskId)
      await deleteWhenTheSandboxHasGone(api, task.taskId)
    })
  }

  // Loops before the repositories they point at — and before anything else,
  // because a live loop is the one leftover that keeps creating work: it routes
  // inbound events, or fires on its own schedule, into new tasks.
  for (const loop of await api.listLoops()) {
    if (!owned(loop.name)) {
      continue
    }
    await record({ kind: 'loop', id: loop.id, name: loop.name ?? loop.id }, () =>
      api.deleteLoop(loop.id),
    )
  }

  for (const workspace of await api.listWorkspaces()) {
    if (!owned(workspace.name)) {
      continue
    }
    await record({ kind: 'workspace', id: workspace.id, name: workspace.name }, () =>
      api.deleteWorkspace(workspace.id),
    )
  }

  for (const repository of await api.listRepositories()) {
    if (!owned(repository.name)) {
      continue
    }
    await record({ kind: 'repository', id: repository.id, name: repository.name }, () =>
      api.deleteRepository(repository.id),
    )
  }

  for (const team of await api.listTeams()) {
    if (!owned(team.name)) {
      continue
    }
    await record({ kind: 'team', id: team.id, name: team.name }, () => api.deleteTeam(team.id))
  }

  // Adapter connections last: a trigger points at one, so the loops pass above
  // has to have run first. A connection carries the run in its external account
  // id rather than in a name.
  let connectionDeleteUnsupported = false
  for (const connection of await api.listAdapterConnections()) {
    if (connectionDeleteUnsupported || !owned(connection.externalAccountId)) {
      continue
    }
    await record(
      {
        kind: 'connection',
        id: connection.id,
        name: connection.externalAccountId,
      },
      async () => {
        const response = await api.deleteAdapterConnectionRaw(connection.id)
        // A backend older than the delete route answers 404 or 405 for
        // every connection, so saying it once and stopping is better than
        // one failure per connection.
        if (response.status === 404 || response.status === 405) {
          connectionDeleteUnsupported = true
          throw new Error(
            `this environment's backend has no DELETE /api/v1/adapters/connections/{id} (HTTP ${response.status}) — connections will accumulate until it is redeployed`,
          )
        }
        if (response.status >= 400) {
          throw new Error(`HTTP ${response.status}`)
        }
      },
    )
  }

  log(summarize(report, onlyThisRun ? runPrefix() : 'every run'))
  return report
}

/** One line naming what a sweep did, for a report header or a loop's output. */
export function summarize(report: SweepReport, scope: string): string {
  if (report.removed.length === 0 && report.failed.length === 0) {
    return `[sweep] nothing to remove for ${scope}`
  }
  const counts = new Map<string, number>()
  for (const resource of report.removed) {
    counts.set(resource.kind, (counts.get(resource.kind) ?? 0) + 1)
  }
  const breakdown = [...counts.entries()]
    .map(([kind, count]) => `${count} ${kind}${count === 1 ? '' : 's'}`)
    .join(', ')
  const verb = report.dryRun ? 'would remove' : 'removed'
  // The reasons, not just the count: a sweep that quietly failed to remove
  // something is how a shared environment accumulates.
  const reasons = [...new Set(report.failed.map((failure) => failure.reason))]
  const failures =
    report.failed.length > 0 ? `, ${report.failed.length} failed — ${reasons.join('; ')}` : ''
  return `[sweep] ${verb} ${breakdown || 'nothing'} for ${scope}${failures}`
}

/**
 * Delete a task, giving its sandbox time to actually go.
 *
 * Deletion removes the sandbox first and waits for it to be observably absent;
 * until then the call is refused and the row survives with a `Removed` intent
 * for the reconciler to finish. Stopping a task returns before that point, so
 * the first delete after a stop can lose the race — which is not a failure, just
 * an early question. Ask again.
 */
async function deleteWhenTheSandboxHasGone(
  api: MoatApi,
  taskId: string,
  attempts = 3,
): Promise<void> {
  for (let attempt = 1; ; attempt++) {
    try {
      await api.deleteTask(taskId)
      return
    } catch (error) {
      if (attempt >= attempts) {
        throw error
      }
      await new Promise((resolve) => setTimeout(resolve, attempt * 5000))
    }
  }
}

/**
 * The tasks a sweep may remove, children before parents.
 *
 * A name is not the only evidence a run created a task, because the suite is not
 * the only thing that creates tasks on its behalf. Three channels:
 *
 *  - the **name**, for everything a test made through `scopedName`.
 *  - the **description**, which holds an inbound delivery's text verbatim. A
 *    webhook delivery creates a task the backend titles from the message, so the
 *    name is a summary the prefix does not survive into and the description is
 *    the only field that still carries it. This is the channel that works from
 *    another sandbox, which is why a killed run's leftovers are reachable at all.
 *  - the **parent**, because a task created from inside a sandbox is a child of
 *    that sandbox's own task. This claims what a run caused through the product
 *    — routed, spawned, whatever the backend chose to call it — and needs no
 *    cooperation from the naming at all. It only sees the sandbox it runs in.
 *
 * Ancestry is walked to the root rather than one hop, so a grandchild is claimed
 * whatever order the listing arrives in.
 *
 * The sandbox's own task is the one task never claimed: sweeping the task that
 * is doing the sweeping would take the pod out from under the run.
 */
function claimedTasks(
  tasks: TaskResponse[],
  owned: (name: string | null | undefined) => boolean,
  minimumAgeMs: number,
): TaskResponse[] {
  const ambient = sandboxTaskId()
  const byId = new Map(tasks.map((task) => [task.taskId, task]))
  const claimed = new Set<string>()

  for (const task of tasks) {
    if (owned(task.name) || owned(task.description)) {
      claimed.add(task.taskId)
    }
  }

  for (const task of tasks) {
    if (claimed.has(task.taskId) || !oldEnough(task, minimumAgeMs)) {
      continue
    }
    for (const ancestor of ancestry(task, byId)) {
      if (ancestor === ambient || claimed.has(ancestor)) {
        claimed.add(task.taskId)
        break
      }
    }
  }

  return tasks
    .filter((task) => claimed.has(task.taskId) && task.taskId !== ambient)
    .sort((left, right) => ancestry(right, byId).length - ancestry(left, byId).length)
}

/** A task's parents, nearest first, stopping at a cycle. */
function ancestry(task: TaskResponse, byId: Map<string, TaskResponse>): string[] {
  const chain: string[] = []
  const seen = new Set([task.taskId])
  let parentId = task.parentTaskId
  while (parentId && !seen.has(parentId)) {
    chain.push(parentId)
    seen.add(parentId)
    parentId = byId.get(parentId)?.parentTaskId ?? null
  }
  return chain
}

/**
 * Whether a task is old enough for a sweep that has an age threshold.
 *
 * A task claimed through its parent has no prefix to read a run's start time
 * from, so its own creation time is the only clock available. It is also the
 * more accurate one, and it keeps a run in flight in this same sandbox out of a
 * scheduled sweep's way.
 */
function oldEnough(task: TaskResponse, minimumAgeMs: number): boolean {
  if (minimumAgeMs <= 0) {
    return true
  }
  const createdAt = task.createdAt ? Date.parse(task.createdAt) : Number.NaN
  return Number.isFinite(createdAt) && Date.now() - createdAt >= minimumAgeMs
}

/**
 * Every task, not just the first page.
 *
 * The listing is capped server-side at 200 rows. A shared cluster holds far more
 * than that, and a leftover from a killed run is exactly the kind of task that
 * sorts old — so a sweep that read one page would keep missing what it exists
 * to remove.
 */
async function allTasks(api: MoatApi): Promise<TaskResponse[]> {
  const pageSize = 200
  const collected: TaskResponse[] = []
  for (let offset = 0; ; offset += pageSize) {
    const page = await api.listTasks({ limit: pageSize, offset })
    collected.push(...page.items)
    if (!page.pagination.hasMore || page.items.length === 0) {
      return collected
    }
  }
}

function describe(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
