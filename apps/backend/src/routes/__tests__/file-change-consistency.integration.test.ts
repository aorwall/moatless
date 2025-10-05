import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { FileContentResponse, FileTreeWithChangesResponse } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { TestApp } from '../../test/helper.js'
import { build } from '../../test/helper.js'

/**
 * Integration tests to verify consistency between file-tree-with-changes
 * and individual file content endpoints. This addresses the issue where
 * files show as changed in the tree but don't return oldContent when
 * requested individually.
 */
describe('File Change Consistency - Integration Tests', () => {
  let app: TestApp
  let tempDataDir: string
  let tempRepoDir: string
  let taskId: string

  beforeEach(async () => {
    // Setup temp directories
    tempDataDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-file-change-test-'))
    tempRepoDir = await fs.mkdtemp(path.join(os.tmpdir(), 'test-repo-'))
    taskId = `test-task-${Date.now()}`

    process.env['DATA_DIR'] = tempDataDir

    // Initialize git repo with proper history
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

  describe('File Tree vs Individual File Content Consistency', () => {
    it('should return oldContent for files shown as modified in file tree', async () => {
      // Setup: Create a file, commit it, then modify it
      const testFileName = 'modified-file.txt'
      const originalContent = 'Original content\nLine 2\nLine 3'
      const modifiedContent = 'Modified content\nNew line 2\nLine 3\nNew line 4'

      await createAndCommitFile(tempRepoDir, testFileName, originalContent)
      await modifyFile(tempRepoDir, testFileName, modifiedContent)

      // Step 1: Get file tree with changes
      const treeResponse = await app.request(`/api/v1/tasks/${taskId}/file-tree-with-changes`)
      expect(treeResponse.status).toBe(200)

      const treeData = (await treeResponse.json()) as FileTreeWithChangesResponse
      const allFiles = flattenFileTree(treeData.tree)
      const modifiedFile = allFiles.find((f) => f.name === testFileName)

      // Verify file shows as changed in tree
      expect(modifiedFile).toBeDefined()
      expect(modifiedFile?.isChanged).toBe(true)
      expect(modifiedFile?.gitStatus).toBe('modified')
      expect(modifiedFile?.additions).toBeGreaterThan(0)

      // Step 2: Get individual file content
      const fileResponse = await app.request(`/api/v1/tasks/${taskId}/files/${testFileName}`)
      expect(fileResponse.status).toBe(200)

      const fileData = (await fileResponse.json()) as FileContentResponse

      // Verify consistency: if tree shows file as changed,
      // individual endpoint MUST return oldContent
      expect(fileData.isModified).toBe(true)
      expect(fileData.oldContent).toBeDefined()
      expect(fileData.oldContent).not.toBeNull()
      expect(fileData.oldContent).toBe(originalContent)
      expect(fileData.content).toBe(modifiedContent)
      expect(fileData.additions).toBeGreaterThan(0)
      expect(fileData.deletions).toBeGreaterThanOrEqual(0)
    })

    it('should return oldContent for new files shown as added in file tree', async () => {
      // Setup: Create a new file (untracked)
      const testFileName = 'new-file.js'
      const newContent = 'export const newFunction = () => {\n  console.log("Hello World");\n};'

      await createFile(tempRepoDir, testFileName, newContent)

      // Step 1: Get file tree with changes
      const treeResponse = await app.request(`/api/v1/tasks/${taskId}/file-tree-with-changes`)
      expect(treeResponse.status).toBe(200)

      const treeData = (await treeResponse.json()) as FileTreeWithChangesResponse
      const allFiles = flattenFileTree(treeData.tree)
      const newFile = allFiles.find((f) => f.name === testFileName)

      // Verify file shows as added in tree
      expect(newFile).toBeDefined()
      expect(newFile?.isChanged).toBe(true)
      expect(newFile?.gitStatus).toBe('added')

      // Step 2: Get individual file content
      const fileResponse = await app.request(`/api/v1/tasks/${taskId}/files/${testFileName}`)
      expect(fileResponse.status).toBe(200)

      const fileData = (await fileResponse.json()) as FileContentResponse

      // Verify consistency: new files should have null oldContent but still be marked as modified
      expect(fileData.isModified).toBe(true)
      expect(fileData.oldContent).toBeNull() // New files have no old content
      expect(fileData.content).toBe(newContent)
      expect(fileData.additions).toBeGreaterThan(0)
      expect(fileData.deletions).toBe(0)
    })

    it('should handle deleted files correctly', async () => {
      // Setup: Create file, commit it, then delete it
      const testFileName = 'deleted-file.txt'
      const originalContent = 'This file will be deleted\nSecond line'

      await createAndCommitFile(tempRepoDir, testFileName, originalContent)
      await deleteFile(tempRepoDir, testFileName)

      // Step 1: Get file tree with changes
      const treeResponse = await app.request(`/api/v1/tasks/${taskId}/file-tree-with-changes`)
      expect(treeResponse.status).toBe(200)

      const treeData = (await treeResponse.json()) as FileTreeWithChangesResponse
      const allFiles = flattenFileTree(treeData.tree)
      const deletedFile = allFiles.find((f) => f.name === testFileName)

      // Verify file shows as deleted in tree
      expect(deletedFile).toBeDefined()
      expect(deletedFile?.isChanged).toBe(true)
      expect(deletedFile?.gitStatus).toBe('deleted')

      // Step 2: Try to get individual file content (should fail since file is deleted)
      const fileResponse = await app.request(`/api/v1/tasks/${taskId}/files/${testFileName}`)
      expect(fileResponse.status).toBe(404) // File doesn't exist on disk
    })

    it('should be consistent across multiple changed files', async () => {
      // Setup: Create multiple files with different change types
      const files = [
        { name: 'modified1.txt', original: 'Original 1', modified: 'Modified 1\nExtra line' },
        {
          name: 'modified2.js',
          original: 'function old() {}',
          modified: 'function updated() {\n  return true;\n}',
        },
        { name: 'new-file.md', original: null, modified: '# New File\nThis is new content' },
      ]

      // Create and modify existing files
      for (const file of files) {
        if (file.original) {
          await createAndCommitFile(tempRepoDir, file.name, file.original)
          await modifyFile(tempRepoDir, file.name, file.modified)
        } else {
          await createFile(tempRepoDir, file.name, file.modified)
        }
      }

      // Step 1: Get file tree with changes
      const treeResponse = await app.request(`/api/v1/tasks/${taskId}/file-tree-with-changes`)
      expect(treeResponse.status).toBe(200)

      const treeData = (await treeResponse.json()) as FileTreeWithChangesResponse
      const allFiles = flattenFileTree(treeData.tree)

      // Step 2: Verify consistency for each file
      for (const file of files) {
        const treeFile = allFiles.find((f) => f.name === file.name)
        expect(treeFile?.isChanged).toBe(true) // Should show as changed in tree

        const fileResponse = await app.request(`/api/v1/tasks/${taskId}/files/${file.name}`)
        expect(fileResponse.status).toBe(200)

        const fileData = (await fileResponse.json()) as FileContentResponse

        // Verify consistency between tree and individual file data
        expect(fileData.isModified).toBe(true)
        expect(fileData.content).toBe(file.modified)

        if (file.original) {
          expect(fileData.oldContent).toBe(file.original)
          expect(treeFile?.gitStatus).toBe('modified')
        } else {
          expect(fileData.oldContent).toBeNull()
          expect(treeFile?.gitStatus).toBe('added')
        }

        expect(fileData.additions).toBeGreaterThan(0)
      }
    })

    it('should handle files with different path formats consistently', async () => {
      // Setup: Create files in nested directories
      const files = [
        { path: 'simple.txt', content: 'Simple file content' },
        { path: 'src/main.js', content: 'console.log("main");' },
        { path: 'src/utils/helper.ts', content: 'export const helper = () => {};' },
        { path: 'docs/README.md', content: '# Documentation\nContent here' },
      ]

      // Create directory structure and files
      for (const file of files) {
        await createAndCommitFile(tempRepoDir, file.path, file.content)
        await modifyFile(tempRepoDir, file.path, `${file.content}\n// Modified`)
      }

      // Step 1: Get file tree with changes
      const treeResponse = await app.request(`/api/v1/tasks/${taskId}/file-tree-with-changes`)
      expect(treeResponse.status).toBe(200)

      const treeData = (await treeResponse.json()) as FileTreeWithChangesResponse
      const allFiles = flattenFileTree(treeData.tree)

      // Step 2: Test different path formats for file requests
      for (const file of files) {
        const fileName = path.basename(file.path)
        const treeFile = allFiles.find((f) => f.name === fileName)
        expect(treeFile?.isChanged).toBe(true)

        // Test with full path
        const fileResponse1 = await app.request(`/api/v1/tasks/${taskId}/files/${file.path}`)
        expect(fileResponse1.status).toBe(200)

        const fileData1 = (await fileResponse1.json()) as FileContentResponse
        expect(fileData1.isModified).toBe(true)
        expect(fileData1.oldContent).toBe(file.content)

        // Test with URL encoded path
        const encodedPath = encodeURIComponent(file.path)
        const fileResponse2 = await app.request(`/api/v1/tasks/${taskId}/files/${encodedPath}`)
        expect(fileResponse2.status).toBe(200)

        const fileData2 = (await fileResponse2.json()) as FileContentResponse
        expect(fileData2.isModified).toBe(true)
        expect(fileData2.oldContent).toBe(file.content)
      }
    })

    it('should handle unchanged files correctly', async () => {
      // Setup: Create a file and commit it without any changes
      const testFileName = 'unchanged-file.txt'
      const content = 'This file remains unchanged'

      await createAndCommitFile(tempRepoDir, testFileName, content)

      // Step 1: Get file tree (should not show this file as changed)
      const treeResponse = await app.request(`/api/v1/tasks/${taskId}/file-tree-with-changes`)
      expect(treeResponse.status).toBe(200)

      const treeData = (await treeResponse.json()) as FileTreeWithChangesResponse
      const allFiles = flattenFileTree(treeData.tree)
      const unchangedFile = allFiles.find((f) => f.name === testFileName)

      // Verify file is not shown as changed in tree (or doesn't appear at all)
      if (unchangedFile) {
        expect(unchangedFile.isChanged).toBe(false)
      }

      // Step 2: Get individual file content
      const fileResponse = await app.request(`/api/v1/tasks/${taskId}/files/${testFileName}`)
      expect(fileResponse.status).toBe(200)

      const fileData = (await fileResponse.json()) as FileContentResponse

      // Verify consistency: unchanged files should not be marked as modified
      expect(fileData.isModified).toBe(false)
      expect(fileData.oldContent).toBeUndefined()
      expect(fileData.content).toBe(content)
      expect(fileData.additions).toBeUndefined()
      expect(fileData.deletions).toBeUndefined()
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

  // Create initial commit to establish main branch
  await fs.writeFile(path.join(repoPath, 'README.md'), '# Test Repository\nInitial commit')
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

  const repoDir = path.join(process.env['DATA_DIR']!, 'repositories')
  await fs.mkdir(repoDir, { recursive: true })
  await fs.writeFile(path.join(repoDir, 'test-repo.json'), JSON.stringify(repoConfig, null, 2))
}

async function createTestTask(taskId: string, workspacePath: string): Promise<void> {
  const taskData = {
    id: taskId,
    repositoryId: 'test-repo',
    workspacePath,
    title: 'Test Task',
    status: 'active',
    created_at: new Date().toISOString(),
  }

  const tasksDir = path.join(process.env['DATA_DIR']!, 'tasks')
  await fs.mkdir(tasksDir, { recursive: true })
  await fs.writeFile(path.join(tasksDir, `${taskId}.json`), JSON.stringify(taskData, null, 2))
}

async function createFile(repoPath: string, filePath: string, content: string): Promise<void> {
  const fullPath = path.join(repoPath, filePath)
  const dir = path.dirname(fullPath)
  await fs.mkdir(dir, { recursive: true })
  await fs.writeFile(fullPath, content)
}

async function createAndCommitFile(
  repoPath: string,
  filePath: string,
  content: string,
): Promise<void> {
  const { exec } = await import('node:child_process')
  const { promisify } = await import('node:util')
  const execAsync = promisify(exec)

  await createFile(repoPath, filePath, content)
  await execAsync(`git add "${filePath}"`, { cwd: repoPath })
  await execAsync(`git commit -m "Add ${filePath}"`, { cwd: repoPath })
}

async function modifyFile(repoPath: string, filePath: string, newContent: string): Promise<void> {
  const fullPath = path.join(repoPath, filePath)
  await fs.writeFile(fullPath, newContent)
}

async function deleteFile(repoPath: string, filePath: string): Promise<void> {
  const fullPath = path.join(repoPath, filePath)
  await fs.unlink(fullPath)
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
