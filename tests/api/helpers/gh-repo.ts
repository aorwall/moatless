/**
 * GitHub repository lifecycle helpers for API E2E tests.
 *
 * Uses the `gh` CLI to create and delete test repositories on GitHub.
 * Requires GITHUB_PAT env var (used as GH_TOKEN for authentication).
 */

import { execSync } from 'node:child_process'

/**
 * Get the authenticated GitHub username via `gh api user`.
 */
export async function getGhUser(): Promise<string> {
  const output = execSync('gh api user --jq .login', {
    encoding: 'utf-8',
    env: { ...process.env, GH_TOKEN: process.env.GITHUB_PAT },
    timeout: 15_000,
  }).trim()

  if (!output) {
    throw new Error('Failed to get GitHub username. Is GITHUB_PAT valid?')
  }

  return output
}

/**
 * Create a private test repository on GitHub using the gh CLI.
 *
 * Returns the full repo URL and the owner/name for later deletion.
 */
export async function createTestRepo(
  name?: string,
): Promise<{ repoUrl: string; repoFullName: string }> {
  const user = await getGhUser()
  const repoName = name || `moatless-test-${Date.now()}`
  const fullName = `${user}/${repoName}`

  execSync(`gh repo create ${fullName} --private --add-readme --clone=false`, {
    encoding: 'utf-8',
    env: { ...process.env, GH_TOKEN: process.env.GITHUB_PAT },
    timeout: 30_000,
  })

  const repoUrl = `https://github.com/${fullName}.git`
  console.log(`[gh-repo] Created test repo: ${fullName}`)

  return { repoUrl, repoFullName: fullName }
}

/**
 * Delete a GitHub repository via `gh repo delete`.
 */
export async function deleteTestRepo(repoFullName: string): Promise<void> {
  try {
    execSync(`gh repo delete ${repoFullName} --yes`, {
      encoding: 'utf-8',
      env: { ...process.env, GH_TOKEN: process.env.GITHUB_PAT },
      timeout: 15_000,
    })
    console.log(`[gh-repo] Deleted test repo: ${repoFullName}`)
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    console.warn(`[gh-repo] Failed to delete repo ${repoFullName}: ${message}`)
  }
}
