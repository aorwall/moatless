/**
 * Standalone: The GitHub proxy
 *
 * The backend posts to GitHub on a task's behalf. Doing it through the backend
 * rather than from the caller is what lets the reply carry the task's identity,
 * bind the task to what it replied to, and be recognised later as the platform's
 * own comment rather than a new inbound event.
 *
 * The refusals need no GitHub at all. The happy path needs a GitHub the test can
 * inspect, so it runs against a stand-in the backend has been pointed at — which
 * means the backend must be able to reach a server this process hosts.
 *
 * Scenarios:
 *   INT-70: Opening a pull request for a task that does not exist is refused
 *   INT-71: Commenting for a task that does not exist is refused
 *   INT-72: Replying for a task that does not exist is refused
 *   INT-73: Opening a pull request with no usable GitHub credentials fails
 *   INT-74: A pull request, a comment and a reply all bind the one task
 */

import {
  MOCK_COMMENT_ID,
  MOCK_DEFAULT_BRANCH,
  MOCK_PR_NUMBER,
  MOCK_REPLY_ID,
  MockGithub,
} from '../../shared/mock-github'
import { missingCapability } from '../../shared/profile'
import { scopedSlug } from '../../shared/run-scope'
import { expect, test } from '../fixtures/api-fixtures'

const UNKNOWN_TASK = 'task-does-not-exist'
const MOCKED = process.env.GITHUB_PROXY_E2E !== undefined

// ─── Refusals — no GitHub involved ──────────────────────────────────────

test.describe('Standalone: GitHub proxy refusals', () => {
  test('INT-70: Opening a pull request for a task that does not exist is refused', async ({
    api,
  }) => {
    const response = await api.githubCreatePrRaw('no-such-owner', 'no-such-repo', {
      title: 't',
      head: 'feat/x',
      base: 'main',
      taskId: UNKNOWN_TASK,
    })

    // Access to the task is settled before anything is sent to GitHub.
    expect(response.status).toBe(404)
  })

  test('INT-71: Commenting for a task that does not exist is refused', async ({ api }) => {
    const response = await api.githubIssueCommentRaw('no-such-owner', 'no-such-repo', 1, {
      body: 'hi',
      taskId: UNKNOWN_TASK,
    })

    expect(response.status).toBe(404)
  })

  test('INT-72: Replying for a task that does not exist is refused', async ({ api }) => {
    const response = await api.githubReplyRaw('no-such-owner', 'no-such-repo', 1, 99, {
      body: 'hi',
      taskId: UNKNOWN_TASK,
    })

    expect(response.status).toBe(404)
  })

  test('INT-73: Opening a pull request with no usable GitHub credentials fails', async ({
    api,
  }) => {
    // Only meaningful against the real GitHub: the stand-in would answer.
    test.skip(MOCKED, 'the backend is pointed at a stand-in GitHub, which answers')

    const response = await api.githubCreatePrRaw('no-such-owner', 'no-such-repo', {
      title: 't',
      head: 'feat/x',
      base: 'main',
    })

    // No token and a token GitHub rejects both surface the same way: the
    // request could not be completed upstream.
    expect(response.status).toBe(502)
  })
})

// ─── INT-74: The happy path, against a GitHub the test can inspect ──────

test.describe('Standalone: GitHub proxy binds the task', () => {
  const reason =
    missingCapability('inboundCallbacks') ??
    (MOCKED
      ? null
      : 'GITHUB_PROXY_E2E is not set, so the backend is not pointed at a stand-in GitHub')
  test.skip(reason !== null, reason ?? '')

  test('INT-74: A pull request, a comment and a reply all bind the one task', async ({ api }) => {
    const github = new MockGithub()
    await github.start(
      process.env.GITHUB_MOCK_PORT ? Number(process.env.GITHUB_MOCK_PORT) : undefined,
    )

    // The proxy resolves a token from the connection that owns the org, and
    // the trigger is what matches events for `owner/repo` back to it.
    const owner = scopedSlug('mockorg')
    const repo = 'mockrepo'
    const fixture = await api.createTaskTriggerFixture({
      adapterKind: 'github',
      connectionKind: 'github_org',
      sourceId: owner,
      sourceMatcher: `${owner}/${repo}`,
      workspaceId: (await api.ensureTemplateWorkspace()).id,
      routingMode: 'by_subject',
    })

    const template = await api.ensureTemplateRepository()
    const task = await api.createVibeTask({
      templateId: template.id,
      name: 'github proxy',
    })

    try {
      // ── The pull request ──
      const pr = await api.githubCreatePr(owner, repo, {
        title: 'feat: mock pr',
        body: 'PR body from e2e test',
        head: 'feat/mock',
        taskId: task.taskId,
      })

      expect(pr.number).toBe(MOCK_PR_NUMBER)
      expect(pr.htmlUrl).toMatch(new RegExp(`/pull/${MOCK_PR_NUMBER}$`))
      expect(pr.bindingId, 'a pull request opened for a task must bind it').toBeTruthy()

      const created = await github.waitFor(
        (request) => request.method === 'POST' && request.path.endsWith('/pulls'),
      )
      expect(created, 'no pull request reached GitHub').not.toBeNull()
      // The base was not given, so the backend looked it up.
      expect(created?.body.base).toBe(MOCK_DEFAULT_BRANCH)
      expect(created?.body.head).toBe('feat/mock')
      const sentBody = String(created?.body.body ?? '')
      expect(sentBody.startsWith('PR body from e2e test')).toBe(true)
      // The footer is what makes the pull request traceable back to the task.
      expect(sentBody).toContain('Moatless task:')
      expect(sentBody).toContain(task.taskId)

      const afterPr = await api.listTaskBindings(task.taskId)
      expect(afterPr).toHaveLength(1)
      expect(afterPr[0]?.subjectKind).toBe('github_pr')
      expect(afterPr[0]?.destinationConfig?.pr_number).toBe(MOCK_PR_NUMBER)
      expect(afterPr[0]?.destinationConfig?.owner).toBe(owner)

      // ── The comment ──
      const comment = await api.githubIssueComment(owner, repo, MOCK_PR_NUMBER, {
        body: 'comment from e2e test',
        taskId: task.taskId,
      })

      expect(comment.id).toBe(MOCK_COMMENT_ID)
      expect(comment.htmlUrl).toMatch(new RegExp(`#issuecomment-${MOCK_COMMENT_ID}$`))

      const commented = await github.waitFor(
        (request) =>
          request.method === 'POST' &&
          request.path.includes('/issues/') &&
          request.path.endsWith('/comments'),
      )
      expect(commented?.body.body).toBe('comment from e2e test')

      // Same subject, so the binding is updated rather than duplicated.
      expect(await api.listTaskBindings(task.taskId)).toHaveLength(1)

      // ── The threaded reply ──
      const reply = await api.githubReply(owner, repo, MOCK_PR_NUMBER, 99, {
        body: 'reply from e2e test',
        taskId: task.taskId,
      })

      expect(reply.id).toBe(MOCK_REPLY_ID)
      expect(reply.htmlUrl).toMatch(
        new RegExp(`/pull/${MOCK_PR_NUMBER}#discussion_r${MOCK_REPLY_ID}$`),
      )
      expect(reply.bindingId, 'a reply sent for a task must bind it').toBeTruthy()

      const replied = await github.waitFor(
        (request) => request.method === 'POST' && request.path.endsWith('/comments/99/replies'),
      )
      expect(replied?.body.body).toBe('reply from e2e test')

      // A reply belongs to the pull request, not to the comment, so it is
      // still the one binding.
      expect(await api.listTaskBindings(task.taskId)).toHaveLength(1)
    } finally {
      await github.stop()
      await api.stopTask(task.taskId).catch(() => {})
      await api.deleteTask(task.taskId).catch(() => {})
      await api.deleteLoop(fixture.id).catch(() => {})
    }
  })
})
