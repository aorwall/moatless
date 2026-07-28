/**
 * Chain: Auth Verify (03-login-session-scenarios.md)
 *
 * Tests the /api/v1/auth/verify endpoint behavior.
 *
 * Preview access control (AUTH-33 to AUTH-35) is exercised here for the first
 * time: `verify_handler` no longer short-circuits, so the task lookup behind a
 * preview hostname actually runs.
 */

import { MoatApi } from '../../shared/moat-api'
import { expect, requiresSeededUsers, test } from '../fixtures/auth-fixtures'

// =============================================================================
// Chain 1: ForwardAuth Verification
// =============================================================================

test.describe('Chain 1: ForwardAuth Verification', () => {
  // AUTH-30: an unauthenticated request is rejected
  test('AUTH-30: Verify endpoint returns 401 without credentials', async ({ driver }) => {
    const response = await new MoatApi(driver.unauthenticated()).verifyAuth()
    expect(response.status).toBe(401)
  })

  // AUTH-30.1: forged X-User-* headers are not an identity — the auth
  // middleware strips them before validating.
  test('AUTH-30.1: Verify endpoint returns 401 for forged user headers', async ({ driver }) => {
    const response = await new MoatApi(driver.unauthenticated()).verifyAuth({
      'X-User-Id': 'admin',
      'X-User-Role': 'admin',
    })
    expect(response.status).toBe(401)
  })

  // AUTH-31: a real credential passes verification
  test('AUTH-31: Verify endpoint returns 200 with a real credential', async ({ api }) => {
    const response = await api.verifyAuth()
    expect(response.status).toBe(200)
  })

  // AUTH-32: a garbage cookie is rejected
  test('AUTH-32: Verify endpoint returns 401 for an invalid cookie', async ({ driver }) => {
    const response = await new MoatApi(driver.unauthenticated()).verifyAuth({
      Cookie: 'moatless_session=not-a-valid-jwt',
    })
    expect(response.status).toBe(401)
  })
})

// =============================================================================
// Chain 2: Preview Access Control
// =============================================================================

test.describe('Chain 2: Preview Access Control', () => {
  requiresSeededUsers()

  // AUTH-35: a preview host for a task that does not exist is denied
  test("AUTH-35: Verify with an unknown task's preview host returns 403", async ({ alice }) => {
    const response = await alice.verifyAuth({
      'X-Forwarded-Host': 'no-such-task--3000.preview.example.com',
    })
    expect(response.status).toBe(403)
  })

  // AUTH-33 / AUTH-34 / AUTH-35.1: the task's own owner reaches its preview,
  // an unrelated user does not, and an admin reaches it regardless.
  test('AUTH-33/34/35.1: Preview access is scoped to the task', async ({ api, alice, bob }) => {
    // The task belongs to alice, not to the admin. Creating it as the admin
    // would leave AUTH-33 asserting that an admin reaches every preview —
    // which is AUTH-35.1, and says nothing about an owner reaching their own.
    const repo = await alice.ensureTemplateRepository()

    const task = await alice.createVibeTask({
      templateId: repo.id,
      message: 'Preview access control task',
    })
    const host = `${task.taskId}--3000.preview.example.com`

    const ownerResponse = await alice.verifyAuth({ 'X-Forwarded-Host': host })
    expect(ownerResponse.status, 'the owner cannot reach its own preview').toBe(200)

    const strangerResponse = await bob.verifyAuth({
      'X-Forwarded-Host': host,
    })
    expect(strangerResponse.status, "an unrelated user reached another task's preview").toBe(403)

    const adminResponse = await api.verifyAuth({ 'X-Forwarded-Host': host })
    expect(adminResponse.status, 'an admin cannot reach the preview').toBe(200)
  })
})
