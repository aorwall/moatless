/**
 * Chain: Team CRUD and Membership (01-team-scenarios.md)
 *
 * Tests team creation, retrieval, updates, deletion, and membership.
 *
 * Teams are an org/directory grouping, not an access boundary (ADR-0006):
 * team credentials and team-scoped resources are gone, so the coverage here
 * stops at CRUD and membership.
 *
 * Chain 1: AUTH-01 → AUTH-01.1 → AUTH-02 → AUTH-03 → AUTH-04 → AUTH-04.1
 * Chain 2: AUTH-05 → AUTH-05.1 → AUTH-07 → AUTH-07.1
 * Standalone: AUTH-09
 */

import { ownedByThisRun, scopedName, scopedSlug } from '../../shared/run-scope'
import { ChainContext } from '../context/chain-context'
import { expect, requiresSeededUsers, test } from '../fixtures/auth-fixtures'

// =============================================================================
// Chain 1: Team CRUD
// =============================================================================

test.describe('Chain 1: Team CRUD', () => {
  requiresSeededUsers()
  test.describe.configure({ mode: 'serial' })

  const ctx = new ChainContext()
  const suffix = Date.now().toString(36)

  test.afterAll(async ({ alice }) => {
    const teamId = ctx.get<string>('teamId')
    if (teamId) {
      await alice.deleteTeam(teamId).catch(() => {})
    }
  })

  // ─── AUTH-01: Create a team ──────────────────────────────────────────

  test('AUTH-01: Create a team', async ({ alice }) => {
    const team = await alice.createTeam({
      name: 'Engineering',
      slug: `engineering-${suffix}`,
    })

    expect(team.id).toBeTruthy()
    expect(team.name).toBe(scopedName('Engineering'))
    expect(team.slug).toBe(scopedSlug(`engineering-${suffix}`))
    expect(team.createdAt).toBeTruthy()
    expect(team.updatedAt).toBeTruthy()

    ctx.set('teamId', team.id)
    ctx.set('teamSlug', team.slug)
  })

  // ─── AUTH-01.1: Team appears in listing ──────────────────────────────

  test('AUTH-01.1: Team appears in listing', async ({ alice }) => {
    const teams = ownedByThisRun(await alice.listTeams(), (t) => t.name)
    const found = teams.some((t) => t.id === ctx.get<string>('teamId'))
    expect(found).toBe(true)
  })

  // ─── AUTH-02: Get team by ID ─────────────────────────────────────────

  test('AUTH-02: Get team by ID', async ({ alice }) => {
    const team = await alice.getTeam(ctx.get<string>('teamId')!)

    expect(team.name).toBe(scopedName('Engineering'))
    expect(team.slug).toBe(scopedSlug(`engineering-${suffix}`))
  })

  // ─── AUTH-03: Update team ────────────────────────────────────────────

  test('AUTH-03: Update team', async ({ alice }) => {
    const updated = await alice.updateTeam(ctx.get<string>('teamId')!, {
      name: scopedName('Platform Engineering'),
      description: 'Platform team',
    })

    expect(updated.name).toBe(scopedName('Platform Engineering'))
    expect(updated.description).toBe('Platform team')
    expect(updated.slug).toBe(scopedSlug(`engineering-${suffix}`))
  })

  // ─── AUTH-04: Delete team ────────────────────────────────────────────

  test('AUTH-04: Delete team', async ({ alice }) => {
    await alice.deleteTeam(ctx.get<string>('teamId')!)
  })

  // ─── AUTH-04.1: Team removed from listing ────────────────────────────

  test('AUTH-04.1: Team removed from listing', async ({ alice }) => {
    const teams = ownedByThisRun(await alice.listTeams(), (t) => t.name)
    const found = teams.some((t) => t.id === ctx.get<string>('teamId'))
    expect(found).toBe(false)

    // Prevent afterAll from trying to delete again
    ctx.set('teamId', undefined)
  })
})

// =============================================================================
// Chain 2: Team Membership
// =============================================================================

test.describe('Chain 2: Team Membership', () => {
  requiresSeededUsers()
  test.describe.configure({ mode: 'serial' })

  const ctx = new ChainContext()
  const suffix = Date.now().toString(36)

  test.afterAll(async ({ alice }) => {
    const teamId = ctx.get<string>('teamId')
    if (teamId) {
      await alice.deleteTeam(teamId).catch(() => {})
    }
  })

  // Setup: Alice creates a team
  test('Setup: Alice creates team for membership tests', async ({ alice }) => {
    const team = await alice.createTeam({
      name: 'Membership Team',
      slug: `membership-${suffix}`,
    })
    ctx.set('teamId', team.id)
  })

  // ─── AUTH-05: Add member to team ─────────────────────────────────────

  test('AUTH-05: Add member to team', async ({ alice, bob }) => {
    // Server-assigned id — logins are not user ids.
    ctx.set('bobId', await bob.userId())
    await alice.addTeamMember(ctx.get<string>('teamId')!, ctx.get<string>('bobId')!, 'member')
  })

  // ─── AUTH-05.1: Member appears in team member listing ────────────────

  test('AUTH-05.1: Member appears in team member listing', async ({ alice }) => {
    const members = await alice.listTeamMembers(ctx.get<string>('teamId')!)
    const membership = members.find((m) => m.userId === ctx.get<string>('bobId'))

    expect(membership).toBeDefined()
    expect(membership!.role).toBe('member')
  })

  // ─── AUTH-07: Remove member from team ────────────────────────────────

  test('AUTH-07: Remove member from team', async ({ alice }) => {
    await alice.removeTeamMember(ctx.get<string>('teamId')!, ctx.get<string>('bobId')!)
  })

  // ─── AUTH-07.1: Removed member no longer in listing ──────────────────

  test('AUTH-07.1: Removed member no longer in listing', async ({ alice }) => {
    const members = await alice.listTeamMembers(ctx.get<string>('teamId')!)
    const membership = members.find((m) => m.userId === ctx.get<string>('bobId'))
    expect(membership).toBeUndefined()
  })
})

// =============================================================================
// Standalone
// =============================================================================

test.describe('Standalone: Team Error Cases', () => {
  requiresSeededUsers()
  // ─── AUTH-09: Duplicate slug → 409 ───────────────────────────────────

  test('AUTH-09: Create team with duplicate slug fails', async ({ alice }) => {
    const suffix = Date.now().toString(36)
    const slug = scopedSlug(`dup-slug-${suffix}`)

    // Create first team
    const team = await alice.createTeam({ name: 'First', slug })

    try {
      // The raw call is deliberately unscoped — it has to send the very same
      // slug the scoped create produced for the conflict to be the thing under
      // test, so the scoping happens here rather than in the verb.
      const response = await alice.createTeamRaw({
        name: scopedName('Second'),
        slug,
      })
      expect(response.status).toBe(409)
    } finally {
      await alice.deleteTeam(team.id).catch(() => {})
    }
  })
})
