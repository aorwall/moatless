/**
 * Standalone: Setup requirements
 *
 * `GET /api/v1/auth/me` carries a setup block: what the environment still needs
 * from this user before it will run their work, and what they can do about it.
 * The remediation is tailored — an admin is pointed at the org-wide route first,
 * a non-admin is never shown it at all.
 *
 * These need a user that is not the one running the suite, and most of them need
 * that user to have no GitHub token, which only direct database access can
 * guarantee on an environment other tests share.
 *
 * Scenarios:
 *   AUTH-40: A fresh user's GitHub requirement reports missing
 *   AUTH-41: A non-admin is never offered the org-wide route
 *   AUTH-42: An admin is offered the org-wide route first
 *   AUTH-43: A user's own PAT satisfies the requirement
 *   AUTH-44: A PAT-backed user can create a task for a GitHub repository
 *   AUTH-45: A non-admin cannot manage global GitHub installations
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

  // ─── AUTH-41: A non-admin is never offered the org-wide route ────────

  test('AUTH-41: A non-admin is never offered the org-wide route', async ({ driver }) => {
    clearGithubTokens()
    const user = await loginAs(driver, scopedSlug('setup-nonadmin'))

    const actions = githubRequirement(await user.me()).actions ?? []
    expect(actions.length).toBeGreaterThan(0)
    // Offering an action they cannot take is worse than offering none.
    expect(actions.some((action) => action.kind === 'admin_route')).toBe(false)
    expect(actions.some((action) => action.kind === 'open_settings')).toBe(true)
  })

  // ─── AUTH-42: An admin is offered the org-wide route first ───────────

  test('AUTH-42: An admin is offered the org-wide route first', async ({ driver }) => {
    clearGithubTokens()
    const admin = await loginAs(driver, scopedSlug('setup-admin'), 'admin')

    const actions = githubRequirement(await admin.me()).actions ?? []
    // Most-preferred first: an admin should connect the org, not just themselves.
    expect(actions[0]?.kind).toBe('admin_route')
    expect(actions[0]?.target).toBe('/admin/integrations/github')
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

  // ─── AUTH-45: A non-admin cannot manage global GitHub installations ──

  test('AUTH-45: A non-admin cannot manage global GitHub installations', async ({ driver }) => {
    const user = await loginAs(driver, scopedSlug('setup-org-forbidden'))

    // 403, not 400. These endpoints answered 400 once while documenting 403;
    // pinning the status is what stops that coming back.
    const routes: [string, string, unknown?][] = [
      ['GET', '/api/v1/admin/settings/github/apps'],
      ['GET', '/api/v1/admin/settings/github/installation'],
      [
        'PUT',
        '/api/v1/admin/settings/github/installation',
        {
          githubAppKey: 'default',
          installationId: 42,
        },
      ],
      ['DELETE', '/api/v1/admin/settings/github/installation'],
    ]

    for (const [method, path, body] of routes) {
      const response = await user.cli.raw(method, path, body)
      expect(response.status, `${method} ${path} should be admin-only`).toBe(403)
    }
  })
})

async function savePat(api: MoatApi, token: string): Promise<number> {
  const response = await api.cli.raw('PUT', '/api/v1/settings/github/pat', { token })
  return response.status
}
