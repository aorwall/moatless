import * as child_process from 'node:child_process'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  createBranch,
  getBranchList,
  getCommitHash,
  getCurrentBranch,
  getGitDiffFiles,
  getGitDiffString,
  getGitStatus,
} from '../git-utils.js'

// Mock child_process
vi.mock('child_process', () => ({
  exec: vi.fn(),
}))

describe('git-utils', () => {
  const mockExec = vi.mocked(child_process.exec)

  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('getGitDiffFiles', () => {
    it('should parse git diff output correctly', async () => {
      // Mock git diff --name-status main
      mockExec.mockImplementationOnce(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (typeof callback === 'function') {
          callback(null, {
            stdout: 'M\tsrc/file1.js\nA\tsrc/file2.js\nD\tsrc/file3.js\n',
            stderr: '',
          })
        }
      }) as any)

      // Mock git diff --numstat main
      mockExec.mockImplementationOnce(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (typeof callback === 'function') {
          callback(null, {
            stdout: '10\t5\tsrc/file1.js\n20\t0\tsrc/file2.js\n0\t15\tsrc/file3.js\n',
            stderr: '',
          })
        }
      }) as any)

      const result = await getGitDiffFiles('/test/repo')

      expect(result.files).toHaveLength(3)
      expect(result.files[0]).toEqual({
        status: 'M',
        filename: 'src/file1.js',
        additions: 10,
        deletions: 5,
      })
      expect(result.files[1]).toEqual({
        status: 'A',
        filename: 'src/file2.js',
        additions: 20,
        deletions: 0,
      })
      expect(result.files[2]).toEqual({
        status: 'D',
        filename: 'src/file3.js',
        additions: 0,
        deletions: 15,
      })
      expect(result.totalAdditions).toBe(30)
      expect(result.totalDeletions).toBe(20)
      expect(result.totalChanges).toBe(3)
    })

    it('should handle empty diff', async () => {
      mockExec.mockImplementation(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (typeof callback === 'function') {
          callback(null, { stdout: '', stderr: '' })
        }
      }) as any)

      const result = await getGitDiffFiles('/test/repo')

      expect(result.files).toHaveLength(0)
      expect(result.totalChanges).toBe(0)
      expect(result.totalAdditions).toBe(0)
      expect(result.totalDeletions).toBe(0)
    })
  })

  describe('getCurrentBranch', () => {
    it('should return the current branch name', async () => {
      mockExec.mockImplementation(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (typeof callback === 'function') {
          callback(null, { stdout: 'main\n', stderr: '' })
        }
      }) as any)

      const branch = await getCurrentBranch('/test/repo')
      expect(branch).toBe('main')
    })

    it('should handle errors', async () => {
      mockExec.mockImplementation(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (typeof callback === 'function') {
          callback(new Error('Not a git repository'), null, null)
        }
      }) as any)

      await expect(getCurrentBranch('/test/repo')).rejects.toThrow('Failed to get current branch')
    })
  })

  describe('createBranch', () => {
    it('should create a new branch', async () => {
      mockExec.mockImplementation(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (typeof callback === 'function') {
          callback(null, { stdout: '', stderr: '' })
        }
      }) as any)

      await createBranch('/test/repo', 'feature-branch')

      expect(mockExec).toHaveBeenCalledWith(
        'git checkout -b "feature-branch"',
        { cwd: '/test/repo', timeout: 10000 },
        expect.any(Function),
      )
    })
  })

  describe('getGitStatus', () => {
    it('should parse git status correctly', async () => {
      mockExec.mockImplementation(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (typeof callback === 'function') {
          // Ensure proper git status format: 2 status chars + space + filename
          // First char is index status, second is working tree status
          // Git status format: XY filename
          // X = index status, Y = working tree status
          // ' M' = modified in working tree, not staged
          // 'M ' = modified and staged
          // 'A ' = added to index
          // '??' = untracked
          const output = [
            ' M src/modified.js',
            '?? src/untracked.js',
            'A  src/staged.js',
            'M  src/staged-modified.js',
          ].join('\n')
          callback(null, { stdout: output, stderr: '' })
        }
      }) as any)

      const status = await getGitStatus('/test/repo')

      expect(status.clean).toBe(false)
      expect(status.modified).toContain('src/modified.js')
      expect(status.untracked).toContain('src/untracked.js')
      expect(status.staged).toContain('src/staged.js')
      expect(status.staged).toContain('src/staged-modified.js')
    })

    it('should indicate clean repository', async () => {
      mockExec.mockImplementation(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (typeof callback === 'function') {
          callback(null, { stdout: '', stderr: '' })
        }
      }) as any)

      const status = await getGitStatus('/test/repo')

      expect(status.clean).toBe(true)
      expect(status.modified).toHaveLength(0)
      expect(status.untracked).toHaveLength(0)
      expect(status.staged).toHaveLength(0)
    })
  })

  describe('getBranchList', () => {
    it('should return list of branches', async () => {
      mockExec.mockImplementation(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (typeof callback === 'function') {
          callback(null, { stdout: '  main\n* feature-branch\n  develop\n', stderr: '' })
        }
      }) as any)

      const branches = await getBranchList('/test/repo')

      expect(branches).toEqual(['main', 'feature-branch', 'develop'])
    })
  })

  describe('getCommitHash', () => {
    it('should return short commit hash by default', async () => {
      mockExec.mockImplementation(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (typeof callback === 'function') {
          callback(null, { stdout: 'abc1234\n', stderr: '' })
        }
      }) as any)

      const hash = await getCommitHash('/test/repo')

      expect(hash).toBe('abc1234')
      expect(mockExec).toHaveBeenCalledWith(
        'git rev-parse --short HEAD',
        expect.any(Object),
        expect.any(Function),
      )
    })

    it('should return full commit hash when requested', async () => {
      mockExec.mockImplementation(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (typeof callback === 'function') {
          callback(null, { stdout: 'abc1234567890abcdef1234567890abcdef12345\n', stderr: '' })
        }
      }) as any)

      const hash = await getCommitHash('/test/repo', false)

      expect(hash).toBe('abc1234567890abcdef1234567890abcdef12345')
      expect(mockExec).toHaveBeenCalledWith(
        'git rev-parse HEAD',
        expect.any(Object),
        expect.any(Function),
      )
    })
  })

  describe('getGitDiffString', () => {
    it('should get diff against specified base branch', async () => {
      const mockDiff = `diff --git a/file1.js b/file1.js
index abc123..def456 100644
--- a/file1.js
+++ b/file1.js
@@ -1,3 +1,3 @@
-old line
+new line
`

      mockExec.mockImplementation(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (typeof callback === 'function') {
          callback(null, { stdout: mockDiff, stderr: '' })
        }
      }) as any)

      const [diff, baseBranch] = await getGitDiffString('/test/repo', 'develop')

      expect(diff).toBe(mockDiff)
      expect(baseBranch).toBe('develop')
      expect(mockExec).toHaveBeenCalledWith(
        'git diff develop',
        { cwd: '/test/repo', timeout: 30000, maxBuffer: 10485760 },
        expect.any(Function),
      )
    })

    it('should fallback to main branch when no base specified', async () => {
      const mockDiff = `diff --git a/file1.js b/file1.js`

      mockExec.mockImplementation(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (typeof callback === 'function') {
          callback(null, { stdout: mockDiff, stderr: '' })
        }
      }) as any)

      const [diff, baseBranch] = await getGitDiffString('/test/repo')

      expect(diff).toBe(mockDiff)
      expect(baseBranch).toBe('main')
      expect(mockExec).toHaveBeenCalledWith(
        'git diff main',
        expect.any(Object),
        expect.any(Function),
      )
    })

    it('should fallback to master then uncommitted changes when main fails', async () => {
      const mockDiff = `diff --git a/file1.js b/file1.js`
      let callCount = 0

      mockExec.mockImplementation(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (typeof callback === 'function') {
          callCount++
          if (callCount === 1) {
            // First call (main) fails
            callback(new Error('Not found'), null, null)
          } else if (callCount === 2) {
            // Second call (master) fails
            callback(new Error('Not found'), null, null)
          } else {
            // Third call (uncommitted changes only) succeeds
            callback(null, { stdout: mockDiff, stderr: '' })
          }
        }
      }) as any)

      const [diff, baseBranch] = await getGitDiffString('/test/repo')

      expect(diff).toBe(mockDiff)
      expect(baseBranch).toBe('HEAD')
      expect(mockExec).toHaveBeenCalledTimes(3)
    })

    it('should work correctly with git worktrees', async () => {
      // This simulates a worktree path which would be used by workspace service
      const worktreePath = '/test/repo/../test-repo-workspace-task123'
      const mockDiff = `diff --git a/src/feature.js b/src/feature.js
new file mode 100644
index 0000000..abc123
--- /dev/null
+++ b/src/feature.js
@@ -0,0 +1,5 @@
+function newFeature() {
+  console.log('New feature in worktree');
+}
+
+module.exports = newFeature;
`

      mockExec.mockImplementation(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (typeof callback === 'function') {
          // Verify the command is executed in the worktree path
          expect(opts.cwd).toBe(worktreePath)
          callback(null, { stdout: mockDiff, stderr: '' })
        }
      }) as any)

      const [diff, baseBranch] = await getGitDiffString(worktreePath, 'main')

      expect(diff).toBe(mockDiff)
      expect(baseBranch).toBe('main')
      expect(mockExec).toHaveBeenCalledWith(
        'git diff main',
        { cwd: worktreePath, timeout: 30000, maxBuffer: 10485760 },
        expect.any(Function),
      )
    })

    it('should show all changes in worktree including committed and uncommitted', async () => {
      // This tests that git diff base shows both committed changes and working directory changes
      const worktreePath = '/workspace/task-branch'
      const mockDiff = `diff --git a/src/api.js b/src/api.js
index abc123..def456 100644
--- a/src/api.js
+++ b/src/api.js
@@ -10,6 +10,10 @@ export const api = {
   async updateUser(id, data) {
     return fetch('/users/' + id, { method: 'PUT', body: data })
   },
+  
+  async deleteUser(id) {
+    return fetch('/users/' + id, { method: 'DELETE' })
+  },
 }
diff --git a/src/component.js b/src/component.js
index 123abc..456def 100644
--- a/src/component.js
+++ b/src/component.js
@@ -1,5 +1,5 @@
 export function Button({ label, onClick }) {
-  return <button onClick={onClick}>{label}</button>
+  return <button className="btn" onClick={onClick}>{label}</button>
 }
`

      mockExec.mockImplementation(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (typeof callback === 'function') {
          expect(opts.cwd).toBe(worktreePath)
          // git diff base shows ALL changes: committed + staged + unstaged
          expect(_cmd).toBe('git diff main')
          callback(null, { stdout: mockDiff, stderr: '' })
        }
      }) as any)

      const [diff, baseBranch] = await getGitDiffString(worktreePath, 'main')

      expect(diff).toBe(mockDiff)
      expect(baseBranch).toBe('main')
      // Verify the diff includes both files (representing committed and uncommitted changes)
      expect(diff).toContain('src/api.js')
      expect(diff).toContain('src/component.js')
      expect(diff).toContain('deleteUser')
      expect(diff).toContain('className="btn"')
    })
  })

  describe('git worktree integration', () => {
    it('should get git diff files from a worktree', async () => {
      const worktreePath = '/test/repo/../repo-workspace-123'

      // Mock git diff --name-status main for worktree
      mockExec.mockImplementationOnce(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (typeof callback === 'function') {
          expect(opts.cwd).toBe(worktreePath)
          expect(_cmd).toContain('git diff --name-status main')
          callback(null, {
            stdout: 'A\tsrc/new-feature.js\nM\tsrc/existing.js\n',
            stderr: '',
          })
        }
      }) as any)

      // Mock git diff --numstat main for worktree
      mockExec.mockImplementationOnce(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (typeof callback === 'function') {
          expect(opts.cwd).toBe(worktreePath)
          expect(_cmd).toContain('git diff --numstat main')
          callback(null, {
            stdout: '50\t0\tsrc/new-feature.js\n10\t5\tsrc/existing.js\n',
            stderr: '',
          })
        }
      }) as any)

      const result = await getGitDiffFiles(worktreePath, 'main')

      expect(result.files).toHaveLength(2)
      expect(result.files[0]).toEqual({
        status: 'A',
        filename: 'src/new-feature.js',
        additions: 50,
        deletions: 0,
      })
      expect(result.files[1]).toEqual({
        status: 'M',
        filename: 'src/existing.js',
        additions: 10,
        deletions: 5,
      })
      expect(result.totalAdditions).toBe(60)
      expect(result.totalDeletions).toBe(5)
    })

    it('should handle git status in worktree', async () => {
      const worktreePath = '/test/repo-worktrees/feature-branch'

      mockExec.mockImplementation(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (typeof callback === 'function') {
          expect(opts.cwd).toBe(worktreePath)
          const output = ['A  src/new-file.js', ' M src/modified.js', '?? src/untracked.js'].join(
            '\n',
          )
          callback(null, { stdout: output, stderr: '' })
        }
      }) as any)

      const status = await getGitStatus(worktreePath)

      expect(status.clean).toBe(false)
      expect(status.staged).toContain('src/new-file.js')
      expect(status.modified).toContain('src/modified.js')
      expect(status.untracked).toContain('src/untracked.js')
    })

    it('should get current branch in worktree', async () => {
      const worktreePath = '/workspace/repo-workspace-task-123'

      mockExec.mockImplementation(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (typeof callback === 'function') {
          expect(opts.cwd).toBe(worktreePath)
          callback(null, { stdout: 'workspace-task-123\n', stderr: '' })
        }
      }) as any)

      const branch = await getCurrentBranch(worktreePath)

      expect(branch).toBe('workspace-task-123')
    })
  })
})
