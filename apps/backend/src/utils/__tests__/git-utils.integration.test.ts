import { execSync } from 'node:child_process'
import * as fs from 'node:fs'
import * as os from 'node:os'
import * as path from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import {
  checkForConflicts,
  createBranch,
  getBranchList,
  getCommitHash,
  getCurrentBranch,
  getGitDiffString,
  getGitStatus,
  getMergeBase,
  getWorkingDirectoryChanges,
  isGitWorktree,
  pruneWorktrees,
} from '../git-utils'

// Helper class for creating and managing test git repositories
class GitTestRepo {
  private tempDir: string

  constructor() {
    // Create a unique temporary directory for this test
    this.tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'git-test-'))
  }

  get path(): string {
    return this.tempDir
  }

  exec(command: string): string {
    return execSync(command, { cwd: this.tempDir, encoding: 'utf8' })
  }

  init(): void {
    this.exec('git init')
    this.exec('git config user.email "test@example.com"')
    this.exec('git config user.name "Test User"')
  }

  createFile(filename: string, content: string): void {
    const filepath = path.join(this.tempDir, filename)
    const dir = path.dirname(filepath)
    if (!fs.existsSync(dir)) {
      fs.mkdirSync(dir, { recursive: true })
    }
    fs.writeFileSync(filepath, content)
  }

  addFile(filename: string): void {
    this.exec(`git add "${filename}"`)
  }

  commit(message: string): void {
    this.exec(`git commit -m "${message}"`)
  }

  createBranch(branchName: string): void {
    this.exec(`git branch "${branchName}"`)
  }

  checkout(branchName: string): void {
    this.exec(`git checkout "${branchName}"`)
  }

  merge(branchName: string): void {
    this.exec(`git merge "${branchName}"`)
  }

  cleanup(): void {
    if (fs.existsSync(this.tempDir)) {
      fs.rmSync(this.tempDir, { recursive: true, force: true })
    }
  }
}

describe('git-utils with real git commands', () => {
  let repo: GitTestRepo

  beforeEach(() => {
    repo = new GitTestRepo()
  })

  afterEach(() => {
    repo.cleanup()
  })

  describe('getCurrentBranch', () => {
    it('should return the current branch name', async () => {
      repo.init()
      repo.createFile('test.txt', 'test')
      repo.addFile('test.txt')
      repo.commit('Initial commit')

      // Default branch after init
      let branch = await getCurrentBranch(repo.path)
      expect(['main', 'master']).toContain(branch)

      // Create and checkout new branch
      repo.createBranch('feature-xyz')
      repo.checkout('feature-xyz')

      branch = await getCurrentBranch(repo.path)
      expect(branch).toBe('feature-xyz')
    })

    it('should handle detached HEAD state', async () => {
      repo.init()
      repo.createFile('test.txt', 'test')
      repo.addFile('test.txt')
      repo.commit('Initial commit')

      // Get commit hash and checkout to create detached HEAD
      const hash = repo.exec('git rev-parse HEAD').trim()
      repo.exec(`git checkout ${hash}`)

      const branch = await getCurrentBranch(repo.path)
      // In detached HEAD state, git branch --show-current returns empty string
      expect(branch).toBe('')
    })
  })

  describe('createBranch', () => {
    it('should create a new branch from current HEAD', async () => {
      repo.init()
      repo.createFile('test.txt', 'test')
      repo.addFile('test.txt')
      repo.commit('Initial commit')

      await createBranch(repo.path, 'new-feature')

      const branches = await getBranchList(repo.path)
      expect(branches).toContain('new-feature')

      // Verify we're on the new branch
      const currentBranch = await getCurrentBranch(repo.path)
      expect(currentBranch).toBe('new-feature')
    })

    it('should create a branch from current HEAD', async () => {
      repo.init()
      repo.createFile('test.txt', 'test')
      repo.addFile('test.txt')
      repo.commit('Initial commit')

      // Create another branch with different content
      repo.createBranch('other')
      repo.checkout('other')
      repo.createFile('other.txt', 'other')
      repo.addFile('other.txt')
      repo.commit('Other commit')

      // Create branch from current position (other branch)
      await createBranch(repo.path, 'from-other')

      // Verify the new branch has content from 'other'
      expect(fs.existsSync(path.join(repo.path, 'other.txt'))).toBe(true)

      // Verify we're on the new branch
      const currentBranch = await getCurrentBranch(repo.path)
      expect(currentBranch).toBe('from-other')
    })
  })

  describe('getGitStatus', () => {
    it('should get comprehensive git status', async () => {
      repo.init()
      repo.createFile('initial.txt', 'initial')
      repo.addFile('initial.txt')
      repo.commit('Initial commit')

      // Create various file states
      repo.createFile('modified.txt', 'original')
      repo.addFile('modified.txt')
      repo.commit('Add file to modify')

      // Modify committed file
      repo.createFile('modified.txt', 'modified content')

      // Stage a new file
      repo.createFile('staged.txt', 'staged content')
      repo.addFile('staged.txt')

      // Create untracked file
      repo.createFile('untracked.txt', 'untracked')

      const status = await getGitStatus(repo.path)

      expect(status.staged).toContain('staged.txt')
      expect(status.modified).toContain('modified.txt')
      expect(status.untracked).toContain('untracked.txt')
      expect(status.clean).toBe(false)
    })

    it('should indicate clean repository', async () => {
      repo.init()
      repo.createFile('test.txt', 'test')
      repo.addFile('test.txt')
      repo.commit('Initial commit')

      const status = await getGitStatus(repo.path)

      expect(status.clean).toBe(true)
      expect(status.modified).toHaveLength(0)
      expect(status.untracked).toHaveLength(0)
      expect(status.staged).toHaveLength(0)
    })
  })

  describe('getBranchList', () => {
    it('should list all branches', async () => {
      repo.init()
      repo.createFile('test.txt', 'test')
      repo.addFile('test.txt')
      repo.commit('Initial commit')

      // Create multiple branches
      repo.createBranch('feature-1')
      repo.createBranch('feature-2')
      repo.createBranch('bugfix/issue-123')

      const branches = await getBranchList(repo.path)

      // Check that default branch exists (main or master)
      expect(branches.some((b) => b === 'main' || b === 'master')).toBe(true)
      expect(branches).toContain('feature-1')
      expect(branches).toContain('feature-2')
      expect(branches).toContain('bugfix/issue-123')
      expect(branches.length).toBe(4)
    })
  })

  describe('getCommitHash', () => {
    it('should get current commit hash', async () => {
      repo.init()
      repo.createFile('test.txt', 'test')
      repo.addFile('test.txt')
      repo.commit('Test commit')

      const hash = await getCommitHash(repo.path)

      expect(hash).toMatch(/^[a-f0-9]{7}$/) // Short SHA format

      // Get full hash
      const fullHash = await getCommitHash(repo.path, false)
      expect(fullHash).toMatch(/^[a-f0-9]{40}$/) // Full SHA-1 format

      // Verify it matches actual git command
      const actualHash = repo.exec('git rev-parse HEAD').trim()
      expect(fullHash).toBe(actualHash)
    })
  })

  describe('getGitDiffString', () => {
    it('should get diff against merge base of specified branch', async () => {
      repo.init()

      // Create initial file
      repo.createFile('file1.js', 'old line\n')
      repo.addFile('file1.js')
      repo.commit('Initial commit')

      // Create feature branch
      repo.createBranch('develop')
      repo.checkout('develop')

      // Modify file
      repo.createFile('file1.js', 'new line\n')
      repo.addFile('file1.js')
      repo.commit('Update file1')

      const [diff, baseBranch] = await getGitDiffString(repo.path, 'main')

      expect(diff).toContain('diff --git a/file1.js b/file1.js')
      expect(diff).toContain('-old line')
      expect(diff).toContain('+new line')

      // baseBranch should be the merge base commit hash
      const mergeBase = await getMergeBase(repo.path, 'main', 'develop')
      expect(baseBranch).toBe(mergeBase)
    })

    it('should include untracked files in diff', async () => {
      repo.init()

      repo.createFile('tracked.js', 'tracked content')
      repo.addFile('tracked.js')
      repo.commit('Add tracked file')

      // Create untracked file
      repo.createFile('untracked.js', 'export default {};')

      const [diff] = await getGitDiffString(repo.path)

      expect(diff).toContain('diff --git a/untracked.js b/untracked.js')
      expect(diff).toContain('new file mode 100644')
      expect(diff).toContain('+export default {};')
    })

    it('should handle binary files', async () => {
      repo.init()

      repo.createFile('text.txt', 'text')
      repo.addFile('text.txt')
      repo.commit('Initial commit')

      // Create a larger binary file to ensure git treats it as binary
      const binaryPath = path.join(repo.path, 'binary.dat')
      const binaryData = Buffer.alloc(1024)
      for (let i = 0; i < binaryData.length; i++) {
        binaryData[i] = Math.floor(Math.random() * 256)
      }
      fs.writeFileSync(binaryPath, binaryData)

      const [diff] = await getGitDiffString(repo.path)

      expect(diff).toContain('diff --git a/binary.dat b/binary.dat')
      // The diff should contain the new file indicator
      expect(diff).toContain('new file mode 100644')
    })
  })

  describe('getMergeBase', () => {
    it('should find merge base between branches', async () => {
      repo.init()

      // Create initial commits
      repo.createFile('base.txt', 'base')
      repo.addFile('base.txt')
      repo.commit('Base commit')

      const baseCommit = repo.exec('git rev-parse HEAD').trim()

      // Create branch1
      repo.createBranch('branch1')
      repo.checkout('branch1')
      repo.createFile('branch1.txt', 'branch1')
      repo.addFile('branch1.txt')
      repo.commit('Branch1 commit')

      // Create branch2 from main
      repo.checkout('main')
      repo.createBranch('branch2')
      repo.checkout('branch2')
      repo.createFile('branch2.txt', 'branch2')
      repo.addFile('branch2.txt')
      repo.commit('Branch2 commit')

      const mergeBase = await getMergeBase(repo.path, 'branch1', 'branch2')
      expect(mergeBase).toBe(baseCommit)
    })

    it('should handle unrelated branches', async () => {
      repo.init()
      repo.createFile('main.txt', 'main')
      repo.addFile('main.txt')
      repo.commit('Main commit')

      // Create orphan branch (unrelated history)
      repo.exec('git checkout --orphan unrelated')
      repo.createFile('unrelated.txt', 'unrelated')
      repo.addFile('unrelated.txt')
      repo.commit('Unrelated commit')

      await expect(getMergeBase(repo.path, 'main', 'unrelated')).rejects.toThrow(
        'Failed to get merge base',
      )
    })
  })

  describe('checkForConflicts', () => {
    it('should detect merge conflicts', async () => {
      repo.init()

      // Create base file
      repo.createFile('conflict.txt', 'line 1\nline 2\nline 3\n')
      repo.addFile('conflict.txt')
      repo.commit('Base commit')

      // Create branch1 and modify file
      repo.createBranch('branch1')
      repo.checkout('branch1')
      repo.createFile('conflict.txt', 'line 1\nmodified by branch1\nline 3\n')
      repo.addFile('conflict.txt')
      repo.commit('Branch1 changes')

      // Create branch2 from main and modify same file differently
      repo.checkout('main')
      repo.createBranch('branch2')
      repo.checkout('branch2')
      repo.createFile('conflict.txt', 'line 1\nmodified by branch2\nline 3\n')
      repo.addFile('conflict.txt')
      repo.commit('Branch2 changes')

      // Checkout branch2 to test merging branch1 into it
      repo.checkout('branch2')
      const hasConflicts = await checkForConflicts(repo.path, 'branch1')
      expect(hasConflicts).toBe(true)
    })

    it('should detect no conflicts for clean merge', async () => {
      repo.init()

      repo.createFile('base.txt', 'base content')
      repo.addFile('base.txt')
      repo.commit('Base commit')

      // Branch1 adds file1
      repo.createBranch('branch1')
      repo.checkout('branch1')
      repo.createFile('file1.txt', 'file1 content')
      repo.addFile('file1.txt')
      repo.commit('Add file1')

      // Branch2 adds file2 (no conflict)
      repo.checkout('main')
      repo.createBranch('branch2')
      repo.checkout('branch2')
      repo.createFile('file2.txt', 'file2 content')
      repo.addFile('file2.txt')
      repo.commit('Add file2')

      // Checkout branch2 to test merging branch1 into it
      repo.checkout('branch2')
      const hasConflicts = await checkForConflicts(repo.path, 'branch1')
      expect(hasConflicts).toBe(false)
    })
  })

  describe('git worktree integration', () => {
    it('should handle worktree paths correctly', async () => {
      repo.init()

      // Create initial structure
      repo.createFile('README.md', '# Test Project')
      repo.addFile('README.md')
      repo.commit('Initial commit')

      // Create a feature branch
      repo.createBranch('feature-work')

      // Create worktree
      const worktreePath = path.join(path.dirname(repo.path), 'test-worktree')
      repo.exec(`git worktree add "${worktreePath}" feature-work`)

      try {
        // Make changes in worktree directly using git commands
        execSync('git add .', { cwd: worktreePath })
        fs.writeFileSync(path.join(worktreePath, 'feature.js'), 'console.log("feature");')
        execSync('git add feature.js', { cwd: worktreePath })
        execSync('git commit -m "Add feature"', { cwd: worktreePath })

        // Get diff from worktree
        // Get default branch from main repo
        const defaultBranch = await getCurrentBranch(repo.path)
        const [diffString] = await getGitDiffString(worktreePath, defaultBranch)
        expect(diffString).toContain('feature.js')

        // Verify branch name in worktree
        const branch = await getCurrentBranch(worktreePath)
        expect(branch).toBe('feature-work')
      } finally {
        // Clean up worktree
        repo.exec(`git worktree remove "${worktreePath}" --force`)
      }
    })
  })

  describe('isGitWorktree', () => {
    it('should correctly identify a git worktree', async () => {
      repo.init()
      repo.createFile('test.txt', 'test')
      repo.addFile('test.txt')
      repo.commit('Initial commit')

      // Main repo should not be a worktree
      const isMainWorktree = await isGitWorktree(repo.path)
      expect(isMainWorktree).toBe(false)

      // Create a worktree
      const worktreePath = path.join(path.dirname(repo.path), 'test-worktree')
      repo.exec(`git worktree add "${worktreePath}" -b test-branch`)

      try {
        // Worktree should be identified as a worktree
        const isWorktree = await isGitWorktree(worktreePath)
        expect(isWorktree).toBe(true)

        // Verify .git is a file in worktree
        const gitPath = path.join(worktreePath, '.git')
        const stat = await fs.promises.stat(gitPath)
        expect(stat.isFile()).toBe(true)

        // Verify .git file contains gitdir
        const content = await fs.promises.readFile(gitPath, 'utf-8')
        expect(content).toMatch(/^gitdir:/)
      } finally {
        repo.exec(`git worktree remove "${worktreePath}" --force`)
      }
    })

    it('should return false for non-git directories', async () => {
      const nonGitDir = fs.mkdtempSync(path.join(os.tmpdir(), 'non-git-'))

      try {
        const isWorktree = await isGitWorktree(nonGitDir)
        expect(isWorktree).toBe(false)
      } finally {
        fs.rmSync(nonGitDir, { recursive: true, force: true })
      }
    })

    it('should return false for regular git repository', async () => {
      repo.init()
      repo.createFile('test.txt', 'test')
      repo.addFile('test.txt')
      repo.commit('Initial commit')

      const isWorktree = await isGitWorktree(repo.path)
      expect(isWorktree).toBe(false)

      // Verify .git is a directory in main repo
      const gitPath = path.join(repo.path, '.git')
      const stat = await fs.promises.stat(gitPath)
      expect(stat.isDirectory()).toBe(true)
    })
  })

  describe('pruneWorktrees', () => {
    it('should prune stale worktree entries', async () => {
      repo.init()
      repo.createFile('test.txt', 'test')
      repo.addFile('test.txt')
      repo.commit('Initial commit')

      // Create a worktree
      const worktreePath = path.join(path.dirname(repo.path), 'test-worktree')
      repo.exec(`git worktree add "${worktreePath}" -b test-branch`)

      // Manually remove the worktree directory (simulating orphaned entry)
      fs.rmSync(worktreePath, { recursive: true, force: true })

      // List worktrees should still show the orphaned entry
      const beforePrune = repo.exec('git worktree list')
      expect(beforePrune).toContain(worktreePath)

      // Prune worktrees
      await pruneWorktrees(repo.path)

      // List worktrees should no longer show the orphaned entry
      const afterPrune = repo.exec('git worktree list')
      expect(afterPrune).not.toContain(worktreePath)
    })

    it('should handle repositories without worktrees', async () => {
      repo.init()
      repo.createFile('test.txt', 'test')
      repo.addFile('test.txt')
      repo.commit('Initial commit')

      // Should not throw when no worktrees exist
      await expect(pruneWorktrees(repo.path)).resolves.not.toThrow()
    })
  })

  describe('worktree cleanup integration', () => {
    it('should properly clean up worktree and branch', async () => {
      repo.init()
      repo.createFile('test.txt', 'test')
      repo.addFile('test.txt')
      repo.commit('Initial commit')

      const taskId = `test-${Date.now()}`
      const worktreePath = path.join(path.dirname(repo.path), `test-workspace-${taskId}`)
      const branchName = `workspace-${taskId}`

      // Create worktree with branch
      repo.exec(`git worktree add -b "${branchName}" "${worktreePath}"`)

      // Verify worktree exists
      expect(await isGitWorktree(worktreePath)).toBe(true)
      const branches = await getBranchList(repo.path)
      // getBranchList might include branch prefix indicators, so check if any branch contains the name
      expect(branches.some((b) => b.includes(branchName))).toBe(true)

      // Simulate cleanup process
      repo.exec(`git worktree remove --force "${worktreePath}"`)

      // Try to delete branch
      try {
        repo.exec(`git branch -D "${branchName}"`)
      } catch {
        // Branch might be checked out, that's ok
      }

      // Prune any stale entries
      await pruneWorktrees(repo.path)

      // Verify cleanup
      const worktreeList = repo.exec('git worktree list')
      expect(worktreeList).not.toContain(worktreePath)

      // Verify directory is gone
      expect(fs.existsSync(worktreePath)).toBe(false)
    })

    it('should handle worktree removal when directory already deleted', async () => {
      repo.init()
      repo.createFile('test.txt', 'test')
      repo.addFile('test.txt')
      repo.commit('Initial commit')

      const worktreePath = path.join(path.dirname(repo.path), 'orphaned-worktree')
      repo.exec(`git worktree add "${worktreePath}" -b orphaned-branch`)

      // Delete directory manually (simulating unexpected deletion)
      fs.rmSync(worktreePath, { recursive: true, force: true })

      // Should still be able to remove worktree entry
      await expect(async () => {
        repo.exec(`git worktree remove --force "${worktreePath}"`)
      }).not.toThrow()

      // Verify worktree entry is gone
      const worktreeList = repo.exec('git worktree list')
      expect(worktreeList).not.toContain(worktreePath)
    })
  })

  describe('getWorkingDirectoryChanges', () => {
    it('should return clean status when no changes exist', async () => {
      repo.init()
      repo.createFile('test.txt', 'Hello World')
      repo.addFile('test.txt')
      repo.commit('Initial commit')

      const changes = await getWorkingDirectoryChanges(repo.path)

      expect(changes.status).toBe('clean')
      expect(changes.additions).toBe(0)
      expect(changes.deletions).toBe(0)
      expect(changes.modifications).toBe(0)
      expect(changes.currentBranch).toMatch(/main|master/)
    })

    it('should count additions from new untracked files', async () => {
      repo.init()
      repo.createFile('test.txt', 'Hello World')
      repo.addFile('test.txt')
      repo.commit('Initial commit')

      // Create untracked file with 3 lines
      repo.createFile('new.txt', 'Line 1\nLine 2\nLine 3')

      const changes = await getWorkingDirectoryChanges(repo.path)

      expect(changes.status).toBe('modified')
      expect(changes.additions).toBe(3)
      expect(changes.deletions).toBe(0)
      expect(changes.modifications).toBe(1) // One file modified (new file)
    })

    it('should count changes from modified tracked files', async () => {
      repo.init()
      repo.createFile('test.txt', 'Hello World')
      repo.addFile('test.txt')
      repo.commit('Initial commit')

      // Modify existing file (add 2 lines, content becomes 3 lines total)
      repo.createFile('test.txt', 'Hello World\nNew Line 1\nNew Line 2')

      const changes = await getWorkingDirectoryChanges(repo.path)

      expect(changes.status).toBe('modified')
      expect(changes.additions).toBeGreaterThan(0) // Should have additions
      expect(changes.modifications).toBe(1) // One file modified
    })

    it('should count staged and unstaged changes together', async () => {
      repo.init()
      repo.createFile('test.txt', 'Hello World')
      repo.addFile('test.txt')
      repo.commit('Initial commit')

      // Create and stage a new file
      repo.createFile('staged.txt', 'Staged content\nLine 2')
      repo.addFile('staged.txt')

      // Create an unstaged file
      repo.createFile('unstaged.txt', 'Unstaged content')

      // Modify existing file without staging
      repo.createFile('test.txt', 'Hello World\nModified line')

      const changes = await getWorkingDirectoryChanges(repo.path)

      expect(changes.status).toBe('modified')
      expect(changes.additions).toBeGreaterThan(0)
      expect(changes.modifications).toBeGreaterThanOrEqual(2) // At least staged.txt and unstaged.txt
    })

    it('should handle empty repository', async () => {
      repo.init()

      const changes = await getWorkingDirectoryChanges(repo.path)

      expect(changes.status).toBe('clean')
      expect(changes.additions).toBe(0)
      expect(changes.deletions).toBe(0)
      expect(changes.modifications).toBe(0)
    })

    it('should return current branch name correctly', async () => {
      repo.init()
      repo.createFile('test.txt', 'Hello World')
      repo.addFile('test.txt')
      repo.commit('Initial commit')

      // Create and checkout new branch
      repo.createBranch('feature-branch')
      repo.checkout('feature-branch')

      // Make a change
      repo.createFile('feature.txt', 'Feature content')

      const changes = await getWorkingDirectoryChanges(repo.path)

      expect(changes.currentBranch).toBe('feature-branch')
      expect(changes.status).toBe('modified')
    })

    it('should handle repository with deleted files', async () => {
      repo.init()
      repo.createFile('test.txt', 'Hello World')
      repo.createFile('delete-me.txt', 'This will be deleted')
      repo.addFile('test.txt')
      repo.addFile('delete-me.txt')
      repo.commit('Initial commit')

      // Delete file from filesystem (but not from git)
      fs.unlinkSync(path.join(repo.path, 'delete-me.txt'))

      const changes = await getWorkingDirectoryChanges(repo.path)

      expect(changes.status).toBe('modified')
      expect(changes.deletions).toBeGreaterThan(0)
      expect(changes.modifications).toBeGreaterThanOrEqual(1)
    })

    it('should return error status on git failure', async () => {
      // Test with non-git directory
      const nonGitPath = fs.mkdtempSync(path.join(os.tmpdir(), 'non-git-'))

      try {
        const changes = await getWorkingDirectoryChanges(nonGitPath)

        expect(changes.status).toBe('error')
        expect(changes.currentBranch).toBe('unknown')
        expect(changes.additions).toBe(0)
        expect(changes.deletions).toBe(0)
        expect(changes.modifications).toBe(0)
      } finally {
        fs.rmSync(nonGitPath, { recursive: true, force: true })
      }
    })
  })
})
