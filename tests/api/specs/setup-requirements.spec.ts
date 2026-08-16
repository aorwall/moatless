/**
 * Standalone: Setup requirements
 *
 * `GET /api/v1/auth/me` carries a setup block: what the environment still needs
 * from this user before it will run their work, and what they can do about it.
 * There is one remedy and it is the same for everyone — connect your own
 * account.
 *
 * These need a user that is not the one running the suite, and most of them need
 * that user to have no GitHub token, which only direct database access can
 * guarantee on an environment other tests share.
 *
 * Scenarios:
 *   AUTH-40: A fresh user's GitHub requirement reports missing
 *   AUTH-41: Nobody is offered an org-wide route
 *   AUTH-43: A user's own PAT satisfies the requirement
 *   AUTH-44: A PAT-backed user can create a task for a GitHub repository
 *   AUTH-46: A user with no GitHub credential is refused a task
 */

import type { SessionResponse, SetupRequirement } from '@moatless/api-client/sandbox/model'
import type { MoatApi } from '../../shared/moat-api'
import { missingCapability } from '../../shared/profile'
import { scopedSlug } from '../../shared/run-scope'
import { expect, loginAs, test } from '../fixtures/auth-fixtures'
import { psql } from '../helpers/psql'

/**
 * The GitHub requirement, which is always reported — satisfied or not.
 */
function githubRequirement(session: SessionResponse): SetupRequirement {
  const requirement = session.setup?.requirements?.find((entry) => entry.id === 'github_token')
  if (!requirement) {
    throw new Error('github_token requirement is always reported')
  }
  return requirement
}

/**
 * Remove every GitHub token a user could reach.
 *
 * Without this the requirement's status depends on what an earlier test left
 * behind, which is the difference between a test and a coin toss.
 */
function clearGithubTokens(): void {
  psql("DELETE FROM provider_tokens WHERE provider = 'github'")
  psql("DELETE FROM secrets WHERE key = 'GITHUB_TOKEN'")
}

test.describe('Standalone: Setup requirements', () => {
  const reason = missingCapability('seedUsers', 'directDatabase')
  test.skip(reason !== null, reason ?? '')

  // ─── AUTH-40: A fresh user's GitHub requirement reports missing ──────

  test("AUTH-40: A fresh user's GitHub requirement reports missing", async ({ driver }) => {
    clearGithubTokens()
    const user = await loginAs(driver, scopedSlug('setup-fresh'))

    const session = await user.me()
    expect(session.setup?.complete).toBe(false)

    const requirement = githubRequirement(session)
    expect(requirement.status).toBe('missing')
    expect(requirement.severity).toBe('required')
  })

  // ─── AUTH-41: Nobody is offered an org-wide route ────────────────────

  test('AUTH-41: Nobody is offered an org-wide route', async ({ driver }) => {
    // An org credential no longer reaches GitHub for anybody, so pointing an
    // administrator at the org page would not fix their own missing one.
    for (const role of ['user', 'admin'] as const) {
      clearGithubTokens()
      const user = await loginAs(driver, scopedSlug(`setup-${role}-actions`), role)

      const actions = githubRequirement(await user.me()).actions ?? []
      expect(actions.length, role).toBe(1)
      expect(actions[0]?.kind, role).toBe('open_settings')
      expect(actions[0]?.target, role).toBe('github')
    }
  })

  // ─── AUTH-43: A user's own PAT satisfies the requirement ─────────────

  test.describe('With a GitHub API the environment can stand in for', () => {
    const mockReason =
      process.env.GITHUB_PROXY_E2E === undefined
        ? 'GITHUB_PROXY_E2E is not set, so the backend is not pointed at a stand-in GitHub'
        : null
    test.skip(mockReason !== null, mockReason ?? '')

    test("AUTH-43: A user's own PAT satisfies the requirement", async ({ driver }) => {
      clearGithubTokens()
      const user = await loginAs(driver, scopedSlug('setup-own-pat'))

      const saved = await savePat(user, 'ghp_setup_own')
      expect(saved).toBe(200)

      const session = await user.me()
      expect(session.setup?.complete).toBe(true)
      expect(githubRequirement(session).status).toBe('satisfied')
    })

    // ─── AUTH-44: A PAT-backed user can create a task ────────────────

    test('AUTH-44: A PAT-backed user can create a task for a GitHub repository', async ({
      driver,
    }) => {
      clearGithubTokens()
      const user = await loginAs(driver, scopedSlug('preflight-pat'))

      expect(await savePat(user, 'ghp_preflight')).toBe(200)

      // A stored PAT lives encrypted in `secrets`; its `provider_tokens` row
      // carries only a sentinel. Preflight has to follow that reference —
      // without it, every PAT-backed user reads as disconnected and their
      // task creation is refused with "GitHub account not connected".
      expect(
        psql(
          "SELECT auth_method FROM provider_tokens WHERE provider = 'github' AND scope = 'user'",
        ),
      ).toBe('secret_ref')

      // The remote is deliberately not github.com: preflight resolves and
      // validates the token but skips the repository probe, which is pinned to
      // the real api.github.com and cannot be stood in for.
      const repository = await user.createRepository({
        name: 'preflight-pat-repo',
        kind: 'remote',
        remoteUrl: 'https://git.example.com/owner/repo',
        defaultBranch: 'main',
        provider: 'github',
      } as Parameters<MoatApi['createRepository']>[0])

      try {
        const response = await user.createVibeTaskRaw({
          repositoryId: repository.id,
          name: 'preflight-pat-task',
        })
        expect(response.status).not.toBe(403)
      } finally {
        await user.deleteRepository(repository.id).catch(() => {})
      }
    })
  })

  // ─── AUTH-46: A user with no GitHub credential is refused a task ─────

  test('AUTH-46: A user with no GitHub credential is refused a task', async ({ driver }) => {
    // The inverse of AUTH-44, and the point of the whole change: with no
    // credential of their own there is nothing to fall through to, so the
    // refusal happens at task creation rather than inside the Sandbox.
    clearGithubTokens()
    const user = await loginAs(driver, scopedSlug('preflight-no-credential'))

    const repository = await user.createRepository({
      name: 'preflight-no-credential-repo',
      kind: 'remote',
      remoteUrl: 'https://git.example.com/owner/repo',
      defaultBranch: 'main',
      provider: 'github',
    } as Parameters<MoatApi['createRepository']>[0])

    try {
      const response = await user.createVibeTaskRaw({
        repositoryId: repository.id,
        name: 'preflight-no-credential-task',
      })
      expect(response.status, 'a user holding no GitHub credential was allowed a task').toBe(403)
    } finally {
      await user.deleteRepository(repository.id).catch(() => {})
    }
  })
})

async function savePat(api: MoatApi, token: string): Promise<number> {
  const response = await api.cli.raw('PUT', '/api/v1/settings/github/pat', { token })
  return response.status
}
