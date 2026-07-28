/**
 * Standalone: Provider settings (specs/auth/03-login-session-scenarios.md)
 *
 * A provider credential is saved, reported and disconnected through
 * `/api/v1/settings/{provider}/…`. The GitHub scenario is the one with teeth: a
 * saved token must live encrypted in the secret store and leave nothing but a
 * reference behind on the provider-token row, and disconnecting must take the
 * secret with it.
 *
 * Scenarios:
 *   AUTH-46: Codex credentials are saved, reported and disconnected
 *   AUTH-47: A saved GitHub token is stored encrypted and never in provider_tokens
 */

import { missingCapability } from '../../shared/profile'
import { scopedSlug } from '../../shared/run-scope'
import { expect, loginAs, test } from '../fixtures/auth-fixtures'
import { psql } from '../helpers/psql'

/** The login the stand-in GitHub answers with. */
const MOCK_LOGIN = 'mock-octocat'

interface ProviderConfig {
  connected: boolean
  login?: string | null
  tokenType?: string | null
}

// ─── AUTH-46: Codex credentials are saved, reported and disconnected ────

test.describe('Standalone: Codex provider settings', () => {
  const reason = missingCapability('seedUsers')
  test.skip(reason !== null, reason ?? '')

  test('AUTH-46: Codex credentials are saved, reported and disconnected', async ({ driver }) => {
    const user = await loginAs(driver, scopedSlug('codex-user'))

    const saved = await user.cli.raw('PUT', '/api/v1/settings/codex/config', {
      authJson: '{"OPENAI_API_KEY":"sk-test","tokens":{"access_token":"session-token"}}',
    })
    expect(saved.status).toBe(200)
    expect((saved.body as ProviderConfig).connected).toBe(true)
    expect((saved.body as ProviderConfig).tokenType).toBe('auth_json')

    const fetched = await user.cli.raw('GET', '/api/v1/settings/codex/config')
    expect(fetched.status).toBe(200)
    expect((fetched.body as ProviderConfig).connected).toBe(true)
    expect((fetched.body as ProviderConfig).tokenType).toBe('auth_json')

    const removed = await user.cli.raw('DELETE', '/api/v1/settings/codex/config')
    expect(removed.status).toBe(200)
    expect((removed.body as ProviderConfig).connected).toBe(false)
    expect((removed.body as ProviderConfig).tokenType ?? null).toBeNull()
  })
})

// ─── AUTH-47: A saved GitHub token never lands in provider_tokens ───────

test.describe('Standalone: GitHub token storage', () => {
  const reason =
    missingCapability('seedUsers', 'directDatabase') ??
    (process.env.GITHUB_PROXY_E2E === undefined
      ? 'GITHUB_PROXY_E2E is not set, so the backend is not pointed at a stand-in GitHub'
      : null)
  test.skip(reason !== null, reason ?? '')

  test('AUTH-47: A saved GitHub token is stored encrypted and never in provider_tokens', async ({
    driver,
  }) => {
    const user = await loginAs(driver, scopedSlug('github-pat'))
    const userId = await user.userId()
    // Distinctive, so searching the row for it means something.
    const pat = 'ghp_integration_test_plaintext_marker'

    const saved = await user.cli.raw('PUT', '/api/v1/settings/github/pat', {
      token: pat,
    })
    expect(saved.status).toBe(200)
    expect((saved.body as ProviderConfig).connected).toBe(true)
    expect((saved.body as ProviderConfig).login).toBe(MOCK_LOGIN)
    expect((saved.body as ProviderConfig).tokenType).toBe('pat')

    // The provider-token row is a reference, not a credential.
    expect(
      psql(
        `SELECT access_token || '|' || auth_method || '|' || COALESCE(token_type, '') ` +
          `FROM provider_tokens WHERE user_id = '${userId}' AND provider = 'github'`,
      ),
    ).toBe('<secret-ref>|secret_ref|pat')

    // Belt and braces: the token appears nowhere on the row at all.
    expect(
      psql(
        `SELECT count(*) FROM provider_tokens WHERE user_id = '${userId}' ` +
          `AND to_jsonb(provider_tokens)::text LIKE '%${pat}%'`,
      ),
    ).toBe('0')

    // The value itself lives encrypted in the secret store.
    expect(
      psql(
        `SELECT kind || '|' || COALESCE(provider, '') || '|' || (value_ciphertext IS NOT NULL) ` +
          `FROM secrets WHERE scope_type = 'user' AND scope_id = '${userId}' AND key = 'GITHUB_TOKEN'`,
      ),
    ).toBe('provider_token|github|true')
    expect(
      psql(
        `SELECT count(*) FROM secrets WHERE scope_id = '${userId}' AND key = 'GITHUB_TOKEN' ` +
          `AND encode(value_ciphertext, 'escape') LIKE '%${pat}%'`,
      ),
    ).toBe('0')

    // The connection is still reported without asking GitHub again.
    const config = await user.cli.raw('GET', '/api/v1/settings/github/config')
    expect(config.status).toBe(200)
    expect((config.body as ProviderConfig).connected).toBe(true)
    expect((config.body as ProviderConfig).login).toBe(MOCK_LOGIN)
    expect((config.body as ProviderConfig).tokenType).toBe('pat')

    // A provider credential is not a secret the user manages by hand, so it
    // does not appear on the generic secrets surface.
    const secrets = await user.cli.raw('GET', '/api/v1/secrets?scope=user')
    expect(secrets.status).toBe(200)
    expect(JSON.stringify(secrets.body)).not.toContain('GITHUB_TOKEN')

    // Disconnecting takes the secret with it.
    const disconnect = await user.cli.raw('DELETE', '/api/v1/settings/github/config')
    expect(disconnect.status).toBe(200)
    expect(
      psql(`SELECT count(*) FROM secrets WHERE scope_id = '${userId}' AND key = 'GITHUB_TOKEN'`),
    ).toBe('0')
  })
})
