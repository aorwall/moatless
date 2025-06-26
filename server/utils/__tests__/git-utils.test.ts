import * as child_process from 'child_process'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  createBranch,
  getBranchList,
  getCommitHash,
  getCurrentBranch,
  getGitDiffFiles,
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
      // Mock git diff --name-status
      mockExec.mockImplementationOnce(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (typeof callback === 'function') {
          callback(null, { stdout: 'M\tsrc/file1.js\nA\tsrc/file2.js\nD\tsrc/file3.js\n', stderr: '' })
        }
      }) as any)

      // Mock git diff --numstat
      mockExec.mockImplementationOnce(((_cmd: string, opts: any, cb?: any) => {
        const callback = cb || opts
        if (typeof callback === 'function') {
          callback(null, { stdout: '10\t5\tsrc/file1.js\n20\t0\tsrc/file2.js\n0\t15\tsrc/file3.js\n', stderr: '' })
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
          const output = [' M src/modified.js', '?? src/untracked.js', 'A  src/staged.js', 'M  src/staged-modified.js'].join(
            '\n',
          )
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
})
