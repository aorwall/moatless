import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { FileTreeWithChangesResponse } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { TestApp } from '../../test/helper.js'
import { build } from '../../test/helper.js'

describe('Workspace File Tree Routes - Integration Tests', () => {
  let app: TestApp
  let tempDataDir: string
  let tempRepoDir: string
  let taskId: string

  beforeEach(async () => {
    // Setup temp directories
    tempDataDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-file-tree-test-'))
    tempRepoDir = await fs.mkdtemp(path.join(os.tmpdir(), 'test-repo-'))
    taskId = `test-task-${Date.now()}`

    process.env.DATA_DIR = tempDataDir

    // Initialize git repo
    await setupTestGitRepo(tempRepoDir)

    // Create test repository config
    await createTestRepository(tempRepoDir)

    // Create test task
    await createTestTask(taskId, tempRepoDir)

    app = await build()
  })

  afterEach(async () => {
    await cleanupTempDir(tempDataDir)
    await cleanupTempDir(tempRepoDir)
  })

  describe('GET /api/v1/tasks/:taskId/file-tree-with-changes', () => {
    it('should return file tree with git changes', async () => {
      const response = await app.request(`/api/v1/tasks/${taskId}/file-tree-with-changes`)

      expect(response.status).toBe(200)

      const data = (await response.json()) as FileTreeWithChangesResponse

      expect(data).toMatchObject({
        tree: expect.arrayContaining([
          expect.objectContaining({
            name: expect.any(String),
            path: expect.any(String),
            type: expect.stringMatching(/^(file|directory)$/),
            isChanged: expect.any(Boolean),
          }),
        ]),
        gitSummary: expect.objectContaining({
          totalFiles: expect.any(Number),
          totalAdditions: expect.any(Number),
          totalDeletions: expect.any(Number),
          totalModified: expect.any(Number),
          totalStaged: expect.any(Number),
          compareBase: expect.any(String),
          hasChanges: expect.any(Boolean),
        }),
      })
    })

    it('should return 404 for non-existent task', async () => {
      const response = await app.request('/api/v1/tasks/non-existent/file-tree-with-changes')
      expect(response.status).toBe(404)
    })

    it('should handle query parameters correctly', async () => {
      const response = await app.request(`/api/v1/tasks/${taskId}/file-tree-with-changes?path=src`)

      expect(response.status).toBe(200)
      const data = (await response.json()) as FileTreeWithChangesResponse
      expect(data.tree).toBeDefined()
    })

    it('should return correct git status for different file states', async () => {
      // Create files with different git states
      await createFileWithGitStatus(tempRepoDir, 'added-file.txt', 'added')
      await createFileWithGitStatus(tempRepoDir, 'modified-file.txt', 'modified')
      await createFileWithGitStatus(tempRepoDir, 'staged-file.txt', 'staged')

      const response = await app.request(`/api/v1/tasks/${taskId}/file-tree-with-changes`)
      expect(response.status).toBe(200)

      const data = (await response.json()) as FileTreeWithChangesResponse

      // Find files with different statuses
      const allFiles = flattenFileTree(data.tree)

      const addedFile = allFiles.find((f) => f.name === 'added-file.txt')
      expect(addedFile?.gitStatus).toBe('added')
      expect(addedFile?.isChanged).toBe(true)

      const modifiedFile = allFiles.find((f) => f.name === 'modified-file.txt')
      expect(modifiedFile?.gitStatus).toBe('modified')
      expect(modifiedFile?.isChanged).toBe(true)

      const stagedFile = allFiles.find((f) => f.name === 'staged-file.txt')
      expect(stagedFile?.isStaged).toBe(true)
      expect(stagedFile?.isChanged).toBe(true)
    })

    it('should return hierarchical tree structure', async () => {
      // Create nested directory structure
      await fs.mkdir(path.join(tempRepoDir, 'src', 'components'), { recursive: true })
      await fs.writeFile(
        path.join(tempRepoDir, 'src', 'components', 'Button.tsx'),
        'export const Button = () => null',
      )
      await fs.writeFile(path.join(tempRepoDir, 'src', 'index.ts'), 'export * from "./components"')

      const response = await app.request(`/api/v1/tasks/${taskId}/file-tree-with-changes`)
      expect(response.status).toBe(200)

      const data = (await response.json()) as FileTreeWithChangesResponse

      // Find src directory
      const srcDir = data.tree.find((node) => node.name === 'src' && node.type === 'directory')
      expect(srcDir).toBeDefined()
      expect(srcDir?.children).toBeDefined()
      expect(srcDir?.children?.length).toBeGreaterThan(0)

      // Find components directory within src
      const componentsDir = srcDir?.children?.find((node: any) => node.name === 'components')
      expect(componentsDir).toBeDefined()
      expect(componentsDir?.type).toBe('directory')
      expect(componentsDir?.children).toBeDefined()

      // Find Button.tsx within components
      const buttonFile = componentsDir?.children?.find((node: any) => node.name === 'Button.tsx')
      expect(buttonFile).toBeDefined()
      expect(buttonFile?.type).toBe('file')
    })

    it('should handle empty repository correctly', async () => {
      // Create empty task workspace
      const emptyTaskId = `empty-task-${Date.now()}`
      await createTestTask(emptyTaskId, tempRepoDir, true) // empty = true

      const response = await app.request(`/api/v1/tasks/${emptyTaskId}/file-tree-with-changes`)
      expect(response.status).toBe(200)

      const data = (await response.json()) as FileTreeWithChangesResponse
      expect(data.tree).toEqual([])
      expect(data.gitSummary.hasChanges).toBe(false)
      expect(data.gitSummary.totalFiles).toBe(0)
    })

    it('should return correct summary statistics', async () => {
      // Create known file changes
      await createFileWithGitStatus(tempRepoDir, 'file1.txt', 'added', {
        additions: 10,
        deletions: 0,
      })
      await createFileWithGitStatus(tempRepoDir, 'file2.txt', 'modified', {
        additions: 5,
        deletions: 3,
      })
      await createFileWithGitStatus(tempRepoDir, 'file3.txt', 'staged', {
        additions: 2,
        deletions: 1,
      })

      const response = await app.request(`/api/v1/tasks/${taskId}/file-tree-with-changes`)
      expect(response.status).toBe(200)

      const data = (await response.json()) as FileTreeWithChangesResponse

      expect(data.gitSummary.totalFiles).toBe(3)
      expect(data.gitSummary.totalAdditions).toBe(17) // 10 + 5 + 2
      expect(data.gitSummary.totalDeletions).toBe(4) // 0 + 3 + 1
      expect(data.gitSummary.totalStaged).toBe(1) // only file3.txt is staged
      expect(data.gitSummary.hasChanges).toBe(true)
    })
  })
})

// Helper functions
async function setupTestGitRepo(repoPath: string): Promise<void> {
  const { exec } = await import('node:child_process')
  const { promisify } = await import('node:util')
  const execAsync = promisify(exec)

  await execAsync('git init', { cwd: repoPath })
  await execAsync('git config user.name "Test User"', { cwd: repoPath })
  await execAsync('git config user.email "test@example.com"', { cwd: repoPath })

  // Create initial commit
  await fs.writeFile(path.join(repoPath, 'README.md'), '# Test Repository')
  await execAsync('git add README.md', { cwd: repoPath })
  await execAsync('git commit -m "Initial commit"', { cwd: repoPath })
}

async function createTestRepository(repoPath: string): Promise<void> {
  const repoConfig = {
    id: 'test-repo',
    name: 'Test Repository',
    path: repoPath,
    useGitWorktrees: false, // Use direct path for integration tests
    codeExecution: { type: 'local' },
  }

  const repoDir = path.join(process.env.DATA_DIR!, 'repositories')
  await fs.mkdir(repoDir, { recursive: true })
  await fs.writeFile(path.join(repoDir, 'test-repo.json'), JSON.stringify(repoConfig, null, 2))
}

async function createTestTask(taskId: string, workspacePath: string, empty = false): Promise<void> {
  const taskData = {
    id: taskId,
    repositoryId: 'test-repo',
    workspacePath: empty ? undefined : workspacePath,
    title: 'Test Task',
    status: 'active',
    created_at: new Date().toISOString(),
  }

  const tasksDir = path.join(process.env.DATA_DIR!, 'tasks')
  await fs.mkdir(tasksDir, { recursive: true })
  await fs.writeFile(path.join(tasksDir, `${taskId}.json`), JSON.stringify(taskData, null, 2))
}

async function createFileWithGitStatus(
  repoPath: string,
  filename: string,
  status: 'added' | 'modified' | 'staged',
  stats?: { additions: number; deletions: number },
): Promise<void> {
  const { exec } = await import('node:child_process')
  const { promisify } = await import('node:util')
  const execAsync = promisify(exec)

  const filePath = path.join(repoPath, filename)

  switch (status) {
    case 'added':
      await fs.writeFile(filePath, `New file content\n${'x'.repeat(stats?.additions || 5)}`)
      break
    case 'modified':
      // First create and commit the file
      await fs.writeFile(filePath, 'Original content')
      await execAsync(`git add ${filename}`, { cwd: repoPath })
      await execAsync(`git commit -m "Add ${filename}"`, { cwd: repoPath })
      // Then modify it
      await fs.writeFile(filePath, `Modified content\n${'x'.repeat(stats?.additions || 5)}`)
      break
    case 'staged':
      await fs.writeFile(filePath, `Staged content\n${'x'.repeat(stats?.additions || 5)}`)
      await execAsync(`git add ${filename}`, { cwd: repoPath })
      break
  }
}

function flattenFileTree(tree: any[]): any[] {
  const result: any[] = []
  for (const node of tree) {
    result.push(node)
    if (node.children) {
      result.push(...flattenFileTree(node.children))
    }
  }
  return result
}

async function cleanupTempDir(dirPath: string): Promise<void> {
  try {
    await fs.rm(dirPath, { recursive: true, force: true })
  } catch (_error) {
    // Ignore cleanup errors
  }
}
