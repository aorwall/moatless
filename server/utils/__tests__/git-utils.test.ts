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
  getGitDiffFiles,
  getGitDiffString,
  getGitStatus,
  getMergeBase,
} from '../git-utils.js'

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

  describe('getGitDiffFiles', () => {
    it('should parse git diff output correctly', async () => {
      // Initialize repo
      repo.init()
      
      // Create initial commit on main
      repo.createFile('src/initial.js', 'console.log("initial");')
      repo.addFile('src/initial.js')
      repo.commit('Initial commit')

      // Create feature branch
      repo.createBranch('feature-branch')
      repo.checkout('feature-branch')

      // Make changes on feature branch
      repo.createFile('src/file1.js', 'const a = 1;\nconst b = 2;\nconst c = 3;\n')
      repo.addFile('src/file1.js')
      repo.commit('Add file1')

      repo.createFile('src/file2.js', 'export default {};\n')
      repo.addFile('src/file2.js')
      repo.commit('Add file2')

      // Delete initial file
      repo.exec('git rm src/initial.js')
      repo.commit('Delete initial file')

      // Create untracked file
      repo.createFile('src/untracked.js', 'untracked content')

      const result = await getGitDiffFiles(repo.path, 'main')

      // Find files by filename since order might vary
      const file1 = result.files.find(f => f.filename === 'src/file1.js')
      expect(file1).toBeDefined()
      expect(file1?.status).toBe('A')

      const file2 = result.files.find(f => f.filename === 'src/file2.js')
      expect(file2).toBeDefined()
      expect(file2?.status).toBe('A')

      const deletedFile = result.files.find(f => f.filename === 'src/initial.js')
      expect(deletedFile).toBeDefined()
      expect(deletedFile?.status).toBe('D')

      const untracked = result.files.find(f => f.filename === 'src/untracked.js')
      expect(untracked).toBeDefined()
      expect(untracked?.status).toBe('??')

      expect(result.totalChanges).toBeGreaterThan(0)
    })

    it('should handle empty diff', async () => {
      repo.init()
      repo.createFile('test.txt', 'test')
      repo.addFile('test.txt')
      repo.commit('Initial commit')

      const result = await getGitDiffFiles(repo.path)

      expect(result.files).toHaveLength(0)
      expect(result.totalChanges).toBe(0)
      expect(result.totalAdditions).toBe(0)
      expect(result.totalDeletions).toBe(0)
    })

    it('should include untracked files', async () => {
      repo.init()
      
      // Create initial commit
      repo.createFile('initial.txt', 'initial')
      repo.addFile('initial.txt')
      repo.commit('Initial commit')

      // Create and checkout feature branch
      repo.createBranch('feature')
      repo.checkout('feature')

      // Create a modified file
      repo.createFile('src/modified.js', 'original content')
      repo.addFile('src/modified.js')
      repo.commit('Add modified.js')
      
      // Modify the file
      repo.createFile('src/modified.js', Array(10).fill('new line').join('\n') + '\n' + Array(5).fill('remaining').join('\n'))
      repo.addFile('src/modified.js')
      repo.commit('Modify modified.js')

      // Create untracked files with exact line counts
      repo.createFile('src/untracked1.js', Array(25).fill('line').join('\n') + '\n')
      repo.createFile('src/untracked2.js', Array(30).fill('code').join('\n') + '\n')

      const result = await getGitDiffFiles(repo.path, 'main')

      const modifiedFile = result.files.find(f => f.filename === 'src/modified.js')
      expect(modifiedFile).toBeDefined()
      expect(modifiedFile?.status).toBe('A')

      const untracked1 = result.files.find(f => f.filename === 'src/untracked1.js')
      expect(untracked1).toBeDefined()
      expect(untracked1?.status).toBe('??')
      expect(untracked1?.additions).toBe(25)

      const untracked2 = result.files.find(f => f.filename === 'src/untracked2.js')
      expect(untracked2).toBeDefined()
      expect(untracked2?.status).toBe('??')
      expect(untracked2?.additions).toBe(30)

      expect(result.files.length).toBeGreaterThanOrEqual(3)
    })

    it('should use merge base for comparison', async () => {
      repo.init()
      
      // Create initial commit
      repo.createFile('README.md', '# Project')
      repo.addFile('README.md')
      repo.commit('Initial commit')

      // Get the default branch name first
      const defaultBranch = await getCurrentBranch(repo.path)

      // Create feature branch
      repo.createBranch('feature')
      repo.checkout('feature')

      // Add feature file
      repo.createFile('feature.js', 'feature code')
      repo.addFile('feature.js')
      repo.commit('Add feature')

      // Go back to default branch and add a different file
      repo.checkout(defaultBranch)
      repo.createFile('main-only.js', 'main only code')
      repo.addFile('main-only.js')
      repo.commit('Add main-only file')

      // Go back to feature branch
      repo.checkout('feature')

      // Get diff - should only show feature.js, not main-only.js
      const result = await getGitDiffFiles(repo.path, defaultBranch)

      expect(result.files).toHaveLength(1)
      expect(result.files[0].filename).toBe('feature.js')
      expect(result.files[0].status).toBe('A')
    })

    it('should show no changes after merging to main', async () => {
      repo.init()
      
      // Create initial commit
      repo.createFile('base.js', 'base content')
      repo.addFile('base.js')
      repo.commit('Initial commit')

      // Get default branch name
      const defaultBranch = await getCurrentBranch(repo.path)

      // Create feature branch
      repo.createBranch('feature')
      repo.checkout('feature')

      // Add multiple features
      repo.createFile('feature1.js', 'feature 1 implementation')
      repo.addFile('feature1.js')
      repo.commit('Add feature 1')

      repo.createFile('feature2.js', 'feature 2 implementation')
      repo.addFile('feature2.js')
      repo.commit('Add feature 2')

      // Before merge, we should see the changes
      const beforeMerge = await getGitDiffFiles(repo.path, defaultBranch)
      expect(beforeMerge.files).toHaveLength(2)

      // Merge to main
      repo.checkout(defaultBranch)
      repo.merge('feature')

      // Go back to feature branch
      repo.checkout('feature')

      // After merge, the merge base is now the merge commit, so no diff
      const result = await getGitDiffFiles(repo.path, defaultBranch)
      expect(result.files).toHaveLength(0)
    })
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
      expect(branches.some(b => b === 'main' || b === 'master')).toBe(true)
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

      await expect(getMergeBase(repo.path, 'main', 'unrelated')).rejects.toThrow('Failed to get merge base')
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
        // Make changes in worktree
        const worktreeRepo = new GitTestRepo()
        worktreeRepo['tempDir'] = worktreePath // Set the path directly
        
        worktreeRepo.createFile('feature.js', 'console.log("feature");')
        worktreeRepo.addFile('feature.js')
        worktreeRepo.commit('Add feature')

        // Get diff from worktree
        // Get default branch from main repo
        const defaultBranch = await getCurrentBranch(repo.path)
        const result = await getGitDiffFiles(worktreePath, defaultBranch)
        
        expect(result.files).toHaveLength(1)
        expect(result.files[0].filename).toBe('feature.js')
        expect(result.files[0].status).toBe('A')

        // Verify branch name in worktree
        const branch = await getCurrentBranch(worktreePath)
        expect(branch).toBe('feature-work')
      } finally {
        // Clean up worktree
        repo.exec(`git worktree remove "${worktreePath}" --force`)
      }
    })
  })
})