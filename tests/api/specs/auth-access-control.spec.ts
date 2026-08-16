/**
 * Chain: Access Control
 *
 * Scope is two values (the access-scope redesign): `global` is readable by every authenticated
 * user and controlled by admins; `user` is read and controlled by its owner.
 * Admins bypass both. A task additionally grants read-and-interact when its
 * visibility is `public`, and never grants control.
 *
 * An invisible resource reads as *not found*, never *forbidden* — the
 * by-id tests assert 404 rather than 403 for exactly that reason.
 *
 * Chain 1: AUTH-20 → AUTH-20.1 → AUTH-22 → AUTH-22.1 → AUTH-22.2 → AUTH-22.3
 * Chain 2: AUTH-23 → AUTH-23.1 → AUTH-23.2 → AUTH-23.3 → AUTH-23.4 → AUTH-23.5
 * Chain 3: AUTH-24 → AUTH-24.1
 * Standalone: AUTH-25, AUTH-26
 */

import { ownedByThisRun, scopedName } from '../../shared/run-scope'
import { ChainContext } from '../context/chain-context'
import { expect, requiresSeededUsers, test } from '../fixtures/auth-fixtures'

// =============================================================================
// Chain 1: Repository Scope Filtering
// =============================================================================

test.describe('Chain 1: Repository Scope Filtering', () => {
  requiresSeededUsers()
  test.describe.configure({ mode: 'serial' })

  const ctx = new ChainContext()
  const suffix = Date.now().toString(36)

  test.afterAll(async ({ api }) => {
    for (const key of ['globalRepoId', 'userRepoId']) {
      const id = ctx.get<string>(key)
      if (id) {
        await api.deleteRepository(id).catch(() => {})
      }
    }
  })

  // ─── AUTH-20: Create global-scoped repository ────────────────────────

  // Global resources are admin-created, so this one goes through the admin
  // fixture rather than alice.
  test('AUTH-20: Create global-scoped repository', async ({ api }) => {
    const repo = await api.createRepository({
      name: scopedName(`Global Repo ${suffix}`),
      kind: 'template',
      scope: 'global',
    })

    expect(repo.id).toBeTruthy()
    expect(repo.scope).toBe('global')

    ctx.set('globalRepoId', repo.id)
  })

  // ─── AUTH-20.1: Other user can see global repository ─────────────────

  test('AUTH-20.1: Other user can see global repository', async ({ bob }) => {
    const repos = ownedByThisRun(await bob.listRepositories(), (r) => r.name)
    const found = repos.some((r) => r.id === ctx.get<string>('globalRepoId'))
    expect(found).toBe(true)
  })

  // ─── AUTH-20.2: Only an admin may create a global repository ─────────

  test('AUTH-20.2: Only an admin may create a global repository', async ({ bob }) => {
    const response = await bob.createRepositoryRaw({
      name: scopedName(`Global Denied ${suffix}`),
      kind: 'template',
      scope: 'global',
    })

    expect(response.status).toBe(403)
  })

  // ─── AUTH-20.3: A global repository is read-only to everyone else ────

  test('AUTH-20.3: A global repository is read-only to everyone else', async ({ bob }) => {
    const response = await bob.updateRepositoryRaw(ctx.get<string>('globalRepoId')!, {
      description: 'hijack',
    })

    // A refusal to control something readable answers 404, not 403: a
    // control denial must not confirm more than the read already did.
    expect(response.status).toBe(404)
  })

  // ─── AUTH-22: Create user-scoped repository ──────────────────────────

  test('AUTH-22: Create user-scoped repository', async ({ alice }) => {
    const repo = await alice.createRepository({
      name: scopedName(`User Repo ${suffix}`),
      kind: 'template',
      scope: 'user',
    })

    expect(repo.id).toBeTruthy()
    expect(repo.scope).toBe('user')

    ctx.set('userRepoId', repo.id)
  })

  // ─── AUTH-22.1: Creator can see user repository ──────────────────────

  test('AUTH-22.1: Creator can see user repository', async ({ alice }) => {
    const repos = ownedByThisRun(await alice.listRepositories(), (r) => r.name)
    const found = repos.some((r) => r.id === ctx.get<string>('userRepoId'))
    expect(found).toBe(true)
  })

  // ─── AUTH-22.2: Other user cannot see user repository ────────────────

  test('AUTH-22.2: Other user cannot see user repository', async ({ bob }) => {
    const repos = ownedByThisRun(await bob.listRepositories(), (r) => r.name)
    const found = repos.some((r) => r.id === ctx.get<string>('userRepoId'))
    expect(found).toBe(false)
  })

  // ─── AUTH-22.3: An invisible repository reads as not found ───────────

  test("AUTH-22.3: Fetching another user's repository answers 404", async ({ bob }) => {
    const response = await bob.getRepositoryRaw(ctx.get<string>('userRepoId')!)
    expect(response.status).toBe(404)
  })
})

// =============================================================================
// Chain 2: Task Scope Inheritance
// =============================================================================

test.describe('Chain 2: Task Scope Inheritance', () => {
  requiresSeededUsers()
  test.describe.configure({ mode: 'serial' })

  const ctx = new ChainContext()
  const suffix = Date.now().toString(36)

  test.afterAll(async ({ alice }) => {
    if (ctx.taskId) {
      await alice.deleteTask(ctx.taskId).catch(() => {})
    }
    if (ctx.repositoryId) {
      await alice.deleteRepository(ctx.repositoryId).catch(() => {})
    }
  })

  // Setup: Alice creates a repository only she can see
  test('Setup: Alice creates a user-scoped repo', async ({ alice }) => {
    const repo = await alice.createRepository({
      name: scopedName(`Task Scope Repo ${suffix}`),
      kind: 'template',
      scope: 'user',
    })
    ctx.repositoryId = repo.id
  })

  // ─── AUTH-23: Create task under user-scoped repository ───────────────

  test('AUTH-23: Create task under user-scoped repository', async ({ alice }) => {
    const task = await alice.createVibeTask({
      templateId: ctx.repositoryId!,
      name: scopedName(`Scoped Task ${suffix}`),
    })

    expect(task.taskId).toBeTruthy()
    ctx.taskId = task.taskId
    ctx.task = task
  })

  // ─── AUTH-23.1: Task inherits scope from its repository ──────────────

  test('AUTH-23.1: Task inherits scope from its repository', async ({ alice }) => {
    const task = await alice.getTask(ctx.taskId!)
    expect(task.taskId).toBe(ctx.taskId)

    // The inheritance is the scenario. A task created under a user-scoped
    // repository that came out global would be visible to the whole
    // environment, and reading back only its own id would not notice.
    expect(task.scope, "the task did not inherit its repository's scope").toBe('user')
    expect(task.ownerUserId, 'the task is not owned by its creator').toBe(await alice.userId())
  })

  // ─── AUTH-23.2: Owner can see the task in listings ───────────────────

  test('AUTH-23.2: Owner can see the task in listings', async ({ alice }) => {
    const tasks = ownedByThisRun((await alice.listTasks()).items, (t) => t.name)
    expect(tasks.some((t) => t.taskId === ctx.taskId)).toBe(true)
  })

  // ─── AUTH-23.3: Another user cannot see the task ─────────────────────

  test('AUTH-23.3: Another user cannot see the task', async ({ bob }) => {
    const tasks = ownedByThisRun((await bob.listTasks()).items, (t) => t.name)
    expect(tasks.some((t) => t.taskId === ctx.taskId)).toBe(false)

    const response = await bob.getTaskRaw(ctx.taskId!)
    expect(response.status).toBe(404)
  })

  // ─── AUTH-23.4: Public visibility grants read to anyone ──────────────

  test('AUTH-23.4: Public visibility grants read to another user', async ({ alice, bob }) => {
    await alice.updateVisibility(ctx.taskId!, 'public')

    const response = await bob.getTaskRaw(ctx.taskId!)
    expect(response.status).toBe(200)
  })

  // ─── AUTH-23.5: Public visibility never grants control ───────────────

  test('AUTH-23.5: Public visibility does not grant control', async ({ bob }) => {
    const response = await bob.deleteTaskRaw(ctx.taskId!)
    expect(response.status).toBeGreaterThanOrEqual(400)
  })
})

// =============================================================================
// Chain 3: Admin Bypass
// =============================================================================

test.describe('Chain 3: Admin Bypass', () => {
  requiresSeededUsers()
  test.describe.configure({ mode: 'serial' })

  const ctx = new ChainContext()
  const suffix = Date.now().toString(36)

  test.afterAll(async ({ alice }) => {
    const id = ctx.get<string>('userRepoId')
    if (id) {
      await alice.deleteRepository(id).catch(() => {})
    }
  })

  // Setup: Alice creates a repository only she owns
  test('Setup: Alice creates a user-scoped repo', async ({ alice }) => {
    const repo = await alice.createRepository({
      name: scopedName(`Alice Private ${suffix}`),
      kind: 'template',
      scope: 'user',
    })
    ctx.set('userRepoId', repo.id)
  })

  // ─── AUTH-24: Admin can see resources they do not own ────────────────

  test('AUTH-24: Admin can see resources they do not own', async ({ carol }) => {
    const repos = ownedByThisRun(await carol.listRepositories(), (r) => r.name)
    expect(repos.some((r) => r.id === ctx.get<string>('userRepoId'))).toBe(true)
  })

  // ─── AUTH-24.1: Admin can fetch user-scoped repository by ID ─────────

  test('AUTH-24.1: Admin can fetch user-scoped repository by ID', async ({ carol }) => {
    const repo = await carol.getRepository(ctx.get<string>('userRepoId')!)
    expect(repo.id).toBe(ctx.get<string>('userRepoId'))
    expect(repo.name).toContain('Alice Private')

    ctx.set('ownerUserId', repo.ownerUserId)
  })

  // ─── AUTH-24.2: An admin's edit does not take ownership ──────────────

  test("AUTH-24.2: An admin's edit does not take ownership", async ({ carol }) => {
    // Re-submitting the scope the repository already has is the case that
    // could quietly re-own it: the write is an admin's, the scope is `user`.
    const updated = await carol.updateRepository(ctx.get<string>('userRepoId')!, {
      scope: 'user',
    })

    expect(updated.ownerUserId).toBe(ctx.get<string>('ownerUserId'))
  })
})

// =============================================================================
// Standalone
// =============================================================================

test.describe('Standalone: Access Control Filtering', () => {
  requiresSeededUsers()

  // ─── AUTH-25: Listing repositories filters by scope ──────────────────

  test('AUTH-25: Listing repositories filters by scope', async ({ alice, bob, api }) => {
    const suffix = Date.now().toString(36)

    const globalRepo = await api.createRepository({
      name: scopedName(`Global ${suffix}`),
      kind: 'template',
      scope: 'global',
    })
    const userRepo = await alice.createRepository({
      name: scopedName(`User ${suffix}`),
      kind: 'template',
      scope: 'user',
    })

    try {
      // Bob sees the global repository and not alice's
      const bobRepos = ownedByThisRun(await bob.listRepositories(), (r) => r.name)
      expect(bobRepos.some((r) => r.id === globalRepo.id)).toBe(true)
      expect(bobRepos.some((r) => r.id === userRepo.id)).toBe(false)

      // Alice sees both — hers, and everyone's
      const aliceRepos = ownedByThisRun(await alice.listRepositories(), (r) => r.name)
      expect(aliceRepos.some((r) => r.id === globalRepo.id)).toBe(true)
      expect(aliceRepos.some((r) => r.id === userRepo.id)).toBe(true)
    } finally {
      await api.deleteRepository(globalRepo.id).catch(() => {})
      await alice.deleteRepository(userRepo.id).catch(() => {})
    }
  })

  // ─── AUTH-26: Listing tasks filters by scope ─────────────────────────

  test('AUTH-26: Listing tasks filters by scope', async ({ alice, bob }) => {
    const suffix = Date.now().toString(36)

    const repo = await alice.createRepository({
      name: scopedName(`Task Filter Repo ${suffix}`),
      kind: 'template',
      scope: 'user',
    })
    const task = await alice.createVibeTask({
      templateId: repo.id,
      name: scopedName(`Scoped Task ${suffix}`),
    })

    try {
      // Bob does not own the task and it is not public
      const bobTasks = ownedByThisRun((await bob.listTasks()).items, (t) => t.name)
      expect(bobTasks.some((t) => t.taskId === task.taskId)).toBe(false)

      // Its owner does see it
      const aliceTasks = ownedByThisRun((await alice.listTasks()).items, (t) => t.name)
      expect(aliceTasks.some((t) => t.taskId === task.taskId)).toBe(true)
    } finally {
      await alice.deleteTask(task.taskId).catch(() => {})
      await alice.deleteRepository(repo.id).catch(() => {})
    }
  })
})

// =============================================================================
// Standalone: Administrative settings
// =============================================================================

test.describe('Standalone: Administrative settings', () => {
  requiresSeededUsers()

  // ─── AUTH-27: Platform-wide GitHub settings are admin-only ───────────

  /**
   * Which GitHub Apps the environment knows about is an administrator's to
   * read and change, not whoever happens to be logged in.
   */
  test('AUTH-27: Platform-wide GitHub settings are admin-only', async ({ bob, carol }) => {
    const path = '/api/v1/admin/settings/github/apps'

    expect((await bob.cli.raw('GET', path)).status).toBe(403)
    expect((await carol.cli.raw('GET', path)).status).toBe(200)
  })
})

// =============================================================================
// Standalone: Racing identity creation
// =============================================================================

test.describe('Standalone: Racing identity creation', () => {
  requiresSeededUsers()

  /**
   * Two people signing up as the same login at the same instant is a conflict,
   * not a server fault. The unique constraint is what decides it, and the
   * losers have to be turned away as 409 — a 500 here would surface to a real
   * person as "something broke" for an outcome the platform expects.
   */
  test('AUTH-28: Racing local-user creation answers 409, not 500', async ({ api }) => {
    const login = `concurrent-user-${Date.now().toString(36)}`

    const responses = await Promise.all(
      Array.from({ length: 8 }, () =>
        api.cli.raw('POST', '/api/v1/users', {
          isBot: false,
          login,
          password: 'integration-test-password',
          role: 'user',
          mustChangePassword: false,
        }),
      ),
    )

    const created = responses.filter((r) => r.status < 400)
    expect(created, `statuses: ${responses.map((r) => r.status).join(', ')}`).toHaveLength(1)

    for (const response of responses.filter((r) => r.status >= 400)) {
      expect(response.status).toBe(409)
    }
  })
})
