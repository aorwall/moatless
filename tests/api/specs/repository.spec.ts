/**
 * Chain: Repository CRUD (repository.md)
 *
 * Tests repository creation from a real GitHub URL, retrieval, updates,
 * listing, and deletion.
 *
 * Chain:
 *   REPO-01 → REPO-01.1 → REPO-02 → REPO-03 → REPO-04 →
 *   REPO-05 → REPO-05.1
 *
 * Standalone: REPO-06, REPO-07
 *
 * Requires: a GitHub token in GITHUB_PAT, and the `gh` CLI on PATH. Without
 * one the chain skips: the repositories it creates are real ones on GitHub.
 */

import { randomUUID } from 'node:crypto'
import { missingCapability } from '../../shared/profile'
import { ownedByThisRun, scopedName } from '../../shared/run-scope'
import type { CreateRepositoryRequest, RepositoryResponse } from '../../shared/types'
import { ChainContext } from '../context/chain-context'
import { expect, test } from '../fixtures/api-fixtures'
import { createTestRepo, deleteTestRepo } from '../helpers/gh-repo'
import { createRepository } from '../steps'

const githubReason = missingCapability('githubRepositories')

// =============================================================================
// Chain: Repository CRUD
// =============================================================================

test.describe('Chain: Repository CRUD', () => {
  test.skip(githubReason !== null, githubReason ?? '')
  test.describe.configure({ mode: 'serial' })

  const ctx = new ChainContext()
  let ghRepoFullName: string | undefined

  test.beforeAll(async () => {
    // Create a private GitHub repo for this chain
    const { repoUrl, repoFullName } = await createTestRepo()
    ghRepoFullName = repoFullName
    ctx.set('repoUrl', repoUrl)
  })

  test.afterAll(async ({ api }) => {
    // Delete the backend repository if it was created
    if (ctx.repositoryId) {
      await api.deleteRepository(ctx.repositoryId).catch(() => {})
    }
    // Delete the GitHub repo
    if (ghRepoFullName) {
      await deleteTestRepo(ghRepoFullName).catch(() => {})
    }
  })

  // ─── REPO-01: Create repository from GitHub URL ─────────────────────

  test('REPO-01: Create repository from GitHub URL', async ({ api }) => {
    const repoUrl = ctx.get<string>('repoUrl')!

    const result = await createRepository(api, {
      name: scopedName('E2E Test Repository'),
      remoteUrl: repoUrl,
      kind: 'remote',
      provider: 'github',
      description: 'Created by API E2E test',
    })

    expect(result.repositoryId).toBeTruthy()
    expect(result.repository.name).toBe(scopedName('E2E Test Repository'))
    expect(result.repository.kind).toBe('remote')
    expect(result.repository.remoteUrl).toBe(repoUrl)

    ctx.repositoryId = result.repositoryId
    ctx.repository = result.repository
  })

  // ─── REPO-01.1: Repository appears in listing ───────────────────────

  test('REPO-01.1: Repository appears in listing', async ({ api }) => {
    const repos = ownedByThisRun(await api.listRepositories(), (r) => r.name)

    const found = repos.some((r) => r.id === ctx.repositoryId)
    expect(found).toBe(true)
  })

  // ─── REPO-02: Get repository by ID ──────────────────────────────────

  test('REPO-02: Get repository by ID', async ({ api }) => {
    const repo = await api.getRepository(ctx.repositoryId!)

    expect(repo.id).toBe(ctx.repositoryId)
    expect(repo.name).toBeTruthy()
    expect(repo.kind).toBe('remote')
    expect(repo.remoteUrl).toBeTruthy()
    expect(repo.createdAt).toBeTruthy()
  })

  // ─── REPO-03: Update repository metadata ────────────────────────────

  test('REPO-03: Update repository metadata', async ({ api }) => {
    const updated = await api.updateRepository(ctx.repositoryId!, {
      name: scopedName('Updated E2E Repository'),
      description: 'Updated by API E2E test',
    })

    expect(updated.name).toBe(scopedName('Updated E2E Repository'))
    expect(updated.description).toBe('Updated by API E2E test')

    // Confirm via GET
    const fetched = await api.getRepository(ctx.repositoryId!)
    expect(fetched.name).toBe(scopedName('Updated E2E Repository'))
    expect(fetched.description).toBe('Updated by API E2E test')
  })

  // ─── REPO-04: List repositories ─────────────────────────────────────

  test('REPO-04: List repositories', async ({ api }) => {
    // A count against the whole environment says nothing on a shared cluster;
    // what the listing owes this test is the repository this run made.
    const repos = ownedByThisRun(await api.listRepositories(), (r) => r.name)

    expect(repos.length).toBeGreaterThan(0)

    const found = repos.find((r) => r.id === ctx.repositoryId)
    expect(found).toBeDefined()
    expect(found!.name).toBe(scopedName('Updated E2E Repository'))
  })

  // ─── REPO-05: Delete repository ─────────────────────────────────────

  test('REPO-05: Delete repository', async ({ api }) => {
    const response = await api.deleteRepositoryRaw(ctx.repositoryId!)

    expect(response.status).toBeLessThan(300)
  })

  // ─── REPO-05.1: Repository removed from listing ─────────────────────

  test('REPO-05.1: Repository removed from listing', async ({ api }) => {
    const repos = ownedByThisRun(await api.listRepositories(), (r) => r.name)

    const found = repos.some((r) => r.id === ctx.repositoryId)
    expect(found).toBe(false)

    // Prevent afterAll from trying to delete again
    ctx.repositoryId = undefined
  })
})

// =============================================================================
// Standalone: Repository URL identity
// =============================================================================

test('REPO-10: Allow repositories with the same name', async ({ api }) => {
  const suffix = randomUUID()
  const name = `Shared repository name ${suffix}`
  const first = await createRepository(api, {
    name,
    remoteUrl: `https://gitness.invalid/acme/${suffix}`,
    provider: 'gitness',
  })
  const second = await createRepository(api, {
    name,
    remoteUrl: `https://gitness.invalid/acme/${suffix}-other`,
    provider: 'gitness',
  })

  try {
    expect(first.repository.name).toBe(scopedName(name))
    expect(second.repository.name).toBe(scopedName(name))
  } finally {
    await api.deleteRepository(first.repositoryId)
    await api.deleteRepository(second.repositoryId)
  }
})

test('REPO-11: Reject an equivalent repository URL', async ({ api }) => {
  const suffix = randomUUID()
  const first = await createRepository(api, {
    name: scopedName(`Canonical URL ${suffix}`),
    remoteUrl: `https://gitness.invalid/acme/${suffix}`,
    provider: 'gitness',
  })

  try {
    const response = await api.createRepositoryRaw({
      name: scopedName(`Canonical duplicate ${suffix}`),
      kind: 'remote',
      remoteUrl: `  HTTPS://GITNESS.INVALID/ACME/${suffix}.git/  `,
      provider: 'gitness',
    })
    expect(response.status).toBe(409)
    expect(JSON.stringify(response.body)).toContain('Repository URL already exists')
  } finally {
    await api.deleteRepository(first.repositoryId)
  }
})

test('REPO-12: Reject a URL collision during update', async ({ api }) => {
  const suffix = randomUUID()
  const first = await createRepository(api, {
    name: scopedName(`Update target ${suffix}`),
    remoteUrl: `https://gitness.invalid/acme/${suffix}`,
    provider: 'gitness',
  })
  const second = await createRepository(api, {
    name: scopedName(`Update source ${suffix}`),
    remoteUrl: `https://gitness.invalid/acme/${suffix}-other`,
    provider: 'gitness',
  })

  try {
    const response = await api.updateRepositoryRaw(second.repositoryId, {
      remoteUrl: ` HTTPS://GITNESS.INVALID/ACME/${suffix}.git/ `,
    })
    expect(response.status).toBe(409)
    const unchanged = await api.getRepository(second.repositoryId)
    expect(unchanged.remoteUrl).toBe(`https://gitness.invalid/acme/${suffix}-other`)
  } finally {
    await api.deleteRepository(first.repositoryId)
    await api.deleteRepository(second.repositoryId)
  }
})

test('REPO-13: Reuse a deleted repository URL', async ({ api }) => {
  const suffix = randomUUID()
  const first = await createRepository(api, {
    name: scopedName(`Deleted URL ${suffix}`),
    remoteUrl: `https://gitness.invalid/acme/${suffix}`,
    provider: 'gitness',
  })
  await api.deleteRepository(first.repositoryId)

  const replacement = await createRepository(api, {
    name: scopedName(`Replacement URL ${suffix}`),
    remoteUrl: ` HTTPS://GITNESS.INVALID/ACME/${suffix}.git/ `,
    provider: 'gitness',
  })
  try {
    expect(replacement.repositoryId).toBeTruthy()
  } finally {
    await api.deleteRepository(replacement.repositoryId)
  }
})

test('REPO-14: Allow the same URL for a different access owner', async ({ api }) => {
  const suffix = randomUUID()
  const remoteUrl = `https://gitness.invalid/acme/${suffix}`
  const userRepository = await api.createRepository({
    name: scopedName(`User URL ${suffix}`),
    kind: 'remote',
    remoteUrl,
    provider: 'gitness',
    scope: 'user',
  })
  const globalRepository = await api.createRepository({
    name: scopedName(`Global URL ${suffix}`),
    kind: 'remote',
    remoteUrl: ` HTTPS://GITNESS.INVALID/ACME/${suffix}.git/ `,
    provider: 'gitness',
    scope: 'global',
  })

  try {
    expect(userRepository.scope).toBe('user')
    expect(globalRepository.scope).toBe('global')
  } finally {
    await api.deleteRepository(userRepository.id)
    await api.deleteRepository(globalRepository.id)
  }
})

// =============================================================================
// Standalone: Error Cases
// =============================================================================

// ─── REPO-06: Create repository with missing required fields (422) ──────

test('REPO-06: Create repository with missing required fields', async ({ api }) => {
  const response = await api.createRepositoryRaw({} as CreateRepositoryRequest)

  expect(response.status).toBeGreaterThanOrEqual(400)
  expect(response.status).toBeLessThan(500)
})

// ─── REPO-07: Get non-existent repository (404) ─────────────────────────

test('REPO-07: Get non-existent repository', async ({ api }) => {
  const fakeId = randomUUID()
  const response = await api.getRepositoryRaw(fakeId)

  expect(response.status).toBe(404)
})

// ─── REPO-08: Reject an unverifiable repository target ──────────────────

test('REPO-08: Reject an unverifiable repository target', async ({ api }) => {
  const suffix = randomUUID()
  const unverifiable: CreateRepositoryRequest[] = [
    // Nothing to check against.
    { name: `Verify Missing URL ${suffix}`, kind: 'template' },
    {
      name: `Verify Invalid URL ${suffix}`,
      kind: 'template',
      remoteUrl: 'not-a-github-url',
      provider: 'github',
    },
    {
      name: `Verify Gitness ${suffix}`,
      kind: 'template',
      remoteUrl: 'https://gitness.example/acme/repo',
      provider: 'gitness',
    },
  ]

  for (const config of unverifiable) {
    // `repos create` requires a URL, so the no-URL case is posted directly.
    const repository = config.remoteUrl
      ? await api.createRepository(config)
      : ((await api.createRepositoryRaw({ ...config, name: scopedName(config.name) }))
          .body as RepositoryResponse)
    try {
      const response = await api.verifyRepositoryAccessRaw(repository.id)

      // Refused outright — the provider is never contacted.
      expect(response.status, `${config.name} should be unverifiable`).toBe(400)
    } finally {
      await api.deleteRepository(repository.id)
    }
  }
})
