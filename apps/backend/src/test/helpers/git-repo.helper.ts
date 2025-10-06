import { exec } from 'node:child_process'
import { promises as fs } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { promisify } from 'node:util'

const execAsync = promisify(exec)

interface TestGitRepo {
  path: string
  cleanup: () => Promise<void>
}

interface GitRepoOptions {
  files?: Record<string, string>
  branches?: string[]
  commits?: string[]
}

/**
 * Creates a test git repository with optional files and branches
 * @param options - Options for creating the test repository
 * @returns Object with repo path and cleanup function
 */
export async function setupTestGitRepo(options: GitRepoOptions = {}): Promise<TestGitRepo> {
  const { files = {}, branches = [], commits = [] } = options

  // Create temporary directory
  const repoPath = await fs.mkdtemp(path.join(tmpdir(), 'test-repo-'))

  try {
    // Initialize git repo
    await execAsync('git init', { cwd: repoPath })
    await execAsync('git config user.email "test@example.com"', {
      cwd: repoPath,
    })
    await execAsync('git config user.name "Test User"', { cwd: repoPath })

    // Create initial files
    const defaultFiles = {
      'README.md': '# Test Repository',
      '.gitignore': 'node_modules/\n*.log',
      ...files,
    }

    for (const [filePath, content] of Object.entries(defaultFiles)) {
      const fullPath = path.join(repoPath, filePath)
      await fs.mkdir(path.dirname(fullPath), { recursive: true })
      await fs.writeFile(fullPath, content)
    }

    // Initial commit
    await execAsync('git add .', { cwd: repoPath })
    await execAsync('git commit -m "Initial commit"', { cwd: repoPath })

    // Create additional branches
    for (const branch of branches) {
      await execAsync(`git checkout -b ${branch}`, { cwd: repoPath })
      // Add a unique file to each branch
      const branchFile = path.join(repoPath, `${branch}.txt`)
      await fs.writeFile(branchFile, `This is the ${branch} branch`)
      await execAsync('git add .', { cwd: repoPath })
      await execAsync(`git commit -m "Add ${branch} branch file"`, {
        cwd: repoPath,
      })
    }

    // Return to main branch
    if (branches.length > 0) {
      await execAsync('git checkout main', { cwd: repoPath })
    }

    // Add any additional commits
    for (const commit of commits) {
      const commitFile = path.join(repoPath, `commit-${Date.now()}.txt`)
      await fs.writeFile(commitFile, commit)
      await execAsync('git add .', { cwd: repoPath })
      await execAsync(`git commit -m "${commit}"`, { cwd: repoPath })
    }

    return {
      path: repoPath,
      cleanup: async () => {
        try {
          await fs.rm(repoPath, { recursive: true, force: true })
        } catch (error) {
          console.warn('Failed to cleanup test repo:', error)
        }
      },
    }
  } catch (error) {
    // Cleanup on error
    await fs.rm(repoPath, { recursive: true, force: true }).catch(() => {})
    throw error
  }
}

/**
 * Creates a git worktree for testing
 * @param mainRepoPath - Path to the main repository
 * @param worktreeName - Name for the worktree
 * @returns Path to the created worktree
 */
export async function createTestWorktree(
  mainRepoPath: string,
  worktreeName: string,
): Promise<string> {
  const worktreePath = path.join(path.dirname(mainRepoPath), worktreeName)
  await execAsync(`git worktree add "${worktreePath}" -b ${worktreeName}`, {
    cwd: mainRepoPath,
  })
  return worktreePath
}
