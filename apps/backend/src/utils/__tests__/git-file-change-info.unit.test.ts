import { execSync } from 'node:child_process'
import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { getFileChangeInfo } from '../git-utils'

/**
 * Unit tests for the getFileChangeInfo function that was updated to fix
 * the file change consistency issue between file tree and individual file content.
 */
describe('getFileChangeInfo function - Unit Tests', () => {
  let tempRepoDir: string

  beforeEach(async () => {
    // Create temporary directory for each test
    tempRepoDir = await fs.mkdtemp(path.join(os.tmpdir(), 'git-file-change-test-'))

    // Initialize git repo with proper configuration to avoid hooks
    execSync('git init', { cwd: tempRepoDir })
    execSync('git config user.name "Test User"', { cwd: tempRepoDir })
    execSync('git config user.email "test@example.com"', { cwd: tempRepoDir })

    // Disable all git hooks to prevent husky interference
    execSync('git config core.hooksPath /dev/null', { cwd: tempRepoDir })

    // Create initial commit to establish main branch
    await fs.writeFile(path.join(tempRepoDir, 'README.md'), '# Test Repository')
    execSync('git add README.md', { cwd: tempRepoDir })
    execSync('git commit -m "Initial commit" --no-verify', {
      cwd: tempRepoDir,
    })
  })

  afterEach(async () => {
    await fs.rm(tempRepoDir, { recursive: true, force: true })
  })

  describe('Modified Files', () => {
    it('should return correct change info for modified text files', async () => {
      const fileName = 'test.txt'
      const originalContent = 'Line 1\nLine 2\nLine 3'
      const modifiedContent = 'Modified Line 1\nLine 2\nLine 3\nNew Line 4'

      // Create and commit original file
      await fs.writeFile(path.join(tempRepoDir, fileName), originalContent)
      execSync(`git add "${fileName}"`, { cwd: tempRepoDir })
      execSync('git commit -m "Add test file" --no-verify', {
        cwd: tempRepoDir,
      })

      // Modify the file
      await fs.writeFile(path.join(tempRepoDir, fileName), modifiedContent)

      // Test getFileChangeInfo
      const changeInfo = await getFileChangeInfo(tempRepoDir, fileName)

      expect(changeInfo.isModified).toBe(true)
      expect(changeInfo.status).toBe('modified')
      expect(changeInfo.oldContent).toBe(originalContent)
      expect(changeInfo.additions).toBeGreaterThan(0)
      expect(changeInfo.deletions).toBeGreaterThanOrEqual(0)
    })

    it('should return correct change info for modified JS files', async () => {
      const fileName = 'script.js'
      const originalContent = 'function hello() {\n  console.log("Hello");\n}'
      const modifiedContent =
        'function hello() {\n  console.log("Hello World!");\n  return true;\n}'

      // Create and commit original file
      await fs.writeFile(path.join(tempRepoDir, fileName), originalContent)
      execSync(`git add "${fileName}"`, { cwd: tempRepoDir })
      execSync('git commit -m "Add JS file" --no-verify', { cwd: tempRepoDir })

      // Modify the file
      await fs.writeFile(path.join(tempRepoDir, fileName), modifiedContent)

      // Test getFileChangeInfo
      const changeInfo = await getFileChangeInfo(tempRepoDir, fileName)

      expect(changeInfo.isModified).toBe(true)
      expect(changeInfo.status).toBe('modified')
      expect(changeInfo.oldContent).toBe(originalContent)
      expect(changeInfo.additions).toBeGreaterThan(0)
    })
  })

  describe('New Files', () => {
    it('should return correct change info for new files', async () => {
      const fileName = 'new-file.txt'
      const newContent = 'New file content\nSecond line\nThird line'

      // Create new file (untracked)
      await fs.writeFile(path.join(tempRepoDir, fileName), newContent)

      // Test getFileChangeInfo
      const changeInfo = await getFileChangeInfo(tempRepoDir, fileName)

      expect(changeInfo.isModified).toBe(true)
      expect(changeInfo.status).toBe('added')
      expect(changeInfo.oldContent).toBeNull()
      expect(changeInfo.additions).toBeGreaterThan(0)
      expect(changeInfo.deletions).toBe(0)
    })

    it('should handle new files in subdirectories', async () => {
      const fileName = 'src/utils/helper.js'
      const newContent = 'export const helper = () => {\n  return "helper";\n};'

      // Create directory structure and file
      await fs.mkdir(path.dirname(path.join(tempRepoDir, fileName)), {
        recursive: true,
      })
      await fs.writeFile(path.join(tempRepoDir, fileName), newContent)

      // Test getFileChangeInfo
      const changeInfo = await getFileChangeInfo(tempRepoDir, fileName)

      expect(changeInfo.isModified).toBe(true)
      expect(changeInfo.status).toBe('added')
      expect(changeInfo.oldContent).toBeNull()
      expect(changeInfo.additions).toBeGreaterThan(0)
    })
  })

  describe('Deleted Files', () => {
    it('should return correct change info for deleted files', async () => {
      const fileName = 'delete-me.txt'
      const originalContent = 'This file will be deleted\nSecond line'

      // Create and commit file
      await fs.writeFile(path.join(tempRepoDir, fileName), originalContent)
      execSync(`git add "${fileName}"`, { cwd: tempRepoDir })
      execSync('git commit -m "Add file to delete" --no-verify', {
        cwd: tempRepoDir,
      })

      // Delete the file
      await fs.unlink(path.join(tempRepoDir, fileName))

      // Test getFileChangeInfo
      const changeInfo = await getFileChangeInfo(tempRepoDir, fileName)

      expect(changeInfo.isModified).toBe(true)
      expect(changeInfo.status).toBe('deleted')
      expect(changeInfo.oldContent).toBe(originalContent)
      expect(changeInfo.additions).toBe(0)
      expect(changeInfo.deletions).toBeGreaterThan(0)
    })
  })

  describe('Unchanged Files', () => {
    it('should return unchanged status for unmodified files', async () => {
      const fileName = 'unchanged.txt'
      const content = 'No changes here'

      // Create and commit file
      await fs.writeFile(path.join(tempRepoDir, fileName), content)
      execSync(`git add "${fileName}"`, { cwd: tempRepoDir })
      execSync('git commit -m "Add unchanged file" --no-verify', {
        cwd: tempRepoDir,
      })

      // Test getFileChangeInfo (no modifications)
      const changeInfo = await getFileChangeInfo(tempRepoDir, fileName)

      expect(changeInfo.isModified).toBe(false)
      expect(changeInfo.status).toBe('unchanged')
      expect(changeInfo.oldContent).toBeNull()
      expect(changeInfo.additions).toBe(0)
      expect(changeInfo.deletions).toBe(0)
    })

    it('should handle non-existent files', async () => {
      const fileName = 'does-not-exist.txt'

      // Test getFileChangeInfo for non-existent file
      const changeInfo = await getFileChangeInfo(tempRepoDir, fileName)

      expect(changeInfo.isModified).toBe(false)
      expect(changeInfo.status).toBe('unchanged')
      expect(changeInfo.oldContent).toBeNull()
      expect(changeInfo.additions).toBe(0)
      expect(changeInfo.deletions).toBe(0)
    })
  })

  describe('Edge Cases', () => {
    it('should handle files with special characters in names', async () => {
      const fileName = 'special (file) [name].txt'
      const originalContent = 'Original content'
      const modifiedContent = 'Modified content'

      // Create and commit original file
      await fs.writeFile(path.join(tempRepoDir, fileName), originalContent)
      execSync(`git add "${fileName}"`, { cwd: tempRepoDir })
      execSync('git commit -m "Add special file" --no-verify', {
        cwd: tempRepoDir,
      })

      // Modify the file
      await fs.writeFile(path.join(tempRepoDir, fileName), modifiedContent)

      // Test getFileChangeInfo
      const changeInfo = await getFileChangeInfo(tempRepoDir, fileName)

      expect(changeInfo.isModified).toBe(true)
      expect(changeInfo.status).toBe('modified')
      expect(changeInfo.oldContent).toBe(originalContent)
    })

    it('should handle empty files', async () => {
      const fileName = 'empty.txt'

      // Create empty file
      await fs.writeFile(path.join(tempRepoDir, fileName), '')

      // Test getFileChangeInfo
      const changeInfo = await getFileChangeInfo(tempRepoDir, fileName)

      expect(changeInfo.isModified).toBe(true)
      expect(changeInfo.status).toBe('added')
      expect(changeInfo.oldContent).toBeNull()
      expect(changeInfo.additions).toBe(0) // Empty file has 0 lines
      expect(changeInfo.deletions).toBe(0)
    })

    it('should work with files that have only whitespace changes', async () => {
      const fileName = 'whitespace.txt'
      const originalContent = 'Line 1\nLine 2'
      const modifiedContent = 'Line 1\nLine 2\n' // Added trailing newline

      // Create and commit original file
      await fs.writeFile(path.join(tempRepoDir, fileName), originalContent)
      execSync(`git add "${fileName}"`, { cwd: tempRepoDir })
      execSync('git commit -m "Add whitespace file" --no-verify', {
        cwd: tempRepoDir,
      })

      // Modify the file (add trailing newline)
      await fs.writeFile(path.join(tempRepoDir, fileName), modifiedContent)

      // Test getFileChangeInfo
      const changeInfo = await getFileChangeInfo(tempRepoDir, fileName)

      expect(changeInfo.isModified).toBe(true)
      expect(changeInfo.status).toBe('modified')
      expect(changeInfo.oldContent).toBe(originalContent)
    })
  })

  describe('Different Base Branches', () => {
    it('should work with different base branches', async () => {
      const fileName = 'branch-test.txt'
      const mainContent = 'Main branch content'
      const featureContent = 'Feature branch content\nExtra line'

      // Create file on main branch
      await fs.writeFile(path.join(tempRepoDir, fileName), mainContent)
      execSync(`git add "${fileName}"`, { cwd: tempRepoDir })
      execSync('git commit -m "Main branch commit" --no-verify', {
        cwd: tempRepoDir,
      })

      // Create and switch to feature branch
      execSync('git checkout -b feature', { cwd: tempRepoDir })

      // Modify file on feature branch
      await fs.writeFile(path.join(tempRepoDir, fileName), featureContent)
      execSync(`git add "${fileName}"`, { cwd: tempRepoDir })
      execSync('git commit -m "Feature branch commit" --no-verify', {
        cwd: tempRepoDir,
      })

      // Test getFileChangeInfo with main as base branch
      const changeInfo = await getFileChangeInfo(tempRepoDir, fileName, 'main')

      expect(changeInfo.isModified).toBe(true)
      expect(changeInfo.status).toBe('modified')
      expect(changeInfo.oldContent).toBe(mainContent)
      expect(changeInfo.additions).toBeGreaterThan(0)
    })
  })

  describe('Direct git command verification', () => {
    it('should produce same results as direct git commands', async () => {
      const fileName = 'verify.txt'
      const originalContent = 'Original\nContent\nHere'
      const modifiedContent = 'Modified\nContent\nHere\nWith\nExtra\nLines'

      // Create and commit original file
      await fs.writeFile(path.join(tempRepoDir, fileName), originalContent)
      execSync(`git add "${fileName}"`, { cwd: tempRepoDir })
      execSync('git commit -m "Add verify file" --no-verify', {
        cwd: tempRepoDir,
      })

      // Modify the file
      await fs.writeFile(path.join(tempRepoDir, fileName), modifiedContent)

      // Get change info using our function
      const changeInfo = await getFileChangeInfo(tempRepoDir, fileName)

      // Verify using direct git commands
      const numstatOutput = execSync(`git diff --numstat HEAD -- "${fileName}"`, {
        cwd: tempRepoDir,
        encoding: 'utf8',
      })
      const [additionsStr, deletionsStr] = numstatOutput.trim().split('\t')
      const expectedAdditions = Number.parseInt(additionsStr || '0', 10)
      const expectedDeletions = Number.parseInt(deletionsStr || '0', 10)

      const statusOutput = execSync(`git diff --name-status HEAD -- "${fileName}"`, {
        cwd: tempRepoDir,
        encoding: 'utf8',
      })
      const expectedStatus = statusOutput.trim().startsWith('M') ? 'modified' : 'added'

      // Compare results
      expect(changeInfo.additions).toBe(expectedAdditions)
      expect(changeInfo.deletions).toBe(expectedDeletions)
      expect(changeInfo.status).toBe(expectedStatus)
      expect(changeInfo.isModified).toBe(true)
      expect(changeInfo.oldContent).toBe(originalContent)
    })
  })
})
