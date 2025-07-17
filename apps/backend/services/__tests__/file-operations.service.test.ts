import { promises as fs } from 'node:fs'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { FileOperations } from '../file-operations.service.js'

// Mock file system operations
vi.mock('node:fs', () => ({
  promises: {
    access: vi.fn(),
    stat: vi.fn(),
    readdir: vi.fn(),
    readFile: vi.fn(),
    writeFile: vi.fn(),
    mkdir: vi.fn(),
  },
}))

// Mock git utilities
vi.mock('../../utils/git-utils.js', () => ({
  isGitRepository: vi.fn().mockResolvedValue(true),
  hasUncommittedChanges: vi.fn().mockResolvedValue(false),
  getGitDiffDetailed: vi.fn().mockResolvedValue({
    files: [],
    summary: { insertions: 0, deletions: 0, filesChanged: 0 },
  }),
  getOriginSyncStatus: vi.fn().mockResolvedValue({
    currentBranch: 'main',
    baseBranch: 'main',
    hasUncommittedChanges: false,
    status: 'synced',
    commitsAhead: 0,
    commitsBehind: 0,
    originExists: true,
  }),
  checkForConflicts: vi.fn().mockResolvedValue(false),
  getWorkingDirectoryChanges: vi.fn().mockResolvedValue({
    currentBranch: 'main',
    status: 'clean',
    additions: 0,
    deletions: 0,
    modifications: 0,
  }),
}))

// Mock file search utility
vi.mock('../../utils/file-search.js', () => ({
  searchFilesInDirectoryNew: vi.fn().mockResolvedValue({
    results: [],
    totalFound: 0,
    query: '',
  }),
}))

describe('FileOperations Service', () => {
  let fileOps: FileOperations
  const taskId = 'test-task-123'
  const workspacePath = '/test/workspace'
  const originalBranch = 'main'

  beforeEach(() => {
    fileOps = new FileOperations(taskId, workspacePath, originalBranch)
    vi.clearAllMocks()
  })

  afterEach(() => {
    vi.clearAllMocks()
  })

  describe('pathExists', () => {
    it('should return true when path exists', async () => {
      vi.mocked(fs.access).mockResolvedValue(undefined)

      const exists = await fileOps.pathExists('/test/path')

      expect(exists).toBe(true)
      expect(fs.access).toHaveBeenCalledWith('/test/path')
    })

    it('should return false when path does not exist', async () => {
      vi.mocked(fs.access).mockRejectedValue(new Error('ENOENT'))

      const exists = await fileOps.pathExists('/test/nonexistent')

      expect(exists).toBe(false)
    })
  })

  describe('isWorkspacePathValid', () => {
    it('should return true when workspace is a valid git repository', async () => {
      vi.mocked(fs.access).mockResolvedValue(undefined)
      const { isGitRepository } = await import('../../utils/git-utils.js')
      vi.mocked(isGitRepository).mockResolvedValue(true)

      const isValid = await fileOps.isWorkspacePathValid()

      expect(isValid).toBe(true)
      expect(fs.access).toHaveBeenCalledWith(workspacePath)
      expect(isGitRepository).toHaveBeenCalledWith(workspacePath)
    })

    it('should return false when workspace path does not exist', async () => {
      vi.mocked(fs.access).mockRejectedValue(new Error('ENOENT'))

      const isValid = await fileOps.isWorkspacePathValid()

      expect(isValid).toBe(false)
    })

    it('should return false when workspace is not a git repository', async () => {
      vi.mocked(fs.access).mockResolvedValue(undefined)
      const { isGitRepository } = await import('../../utils/git-utils.js')
      vi.mocked(isGitRepository).mockResolvedValue(false)

      const isValid = await fileOps.isWorkspacePathValid()

      expect(isValid).toBe(false)
    })
  })

  describe('listFiles', () => {
    it('should list files in directory', async () => {
      vi.mocked(fs.stat).mockResolvedValue({
        isFile: () => false,
        isDirectory: () => true,
      } as any)

      vi.mocked(fs.readdir).mockResolvedValue([
        { name: 'file1.ts', isDirectory: () => false } as any,
        { name: 'file2.js', isDirectory: () => false } as any,
        { name: 'subdir', isDirectory: () => true } as any,
        { name: '.hidden', isDirectory: () => false } as any,
        { name: 'node_modules', isDirectory: () => true } as any,
      ])

      const files = await fileOps.listFiles()

      expect(files).toEqual(['file1.ts', 'file2.js', 'subdir/'])
      expect(files).not.toContain('.hidden')
      expect(files).not.toContain('node_modules/')
    })

    it('should list files recursively', async () => {
      vi.mocked(fs.stat).mockResolvedValue({
        isFile: () => false,
        isDirectory: () => true,
      } as any)

      vi.mocked(fs.readdir)
        .mockResolvedValueOnce([
          { name: 'file1.ts', isDirectory: () => false } as any,
          { name: 'subdir', isDirectory: () => true } as any,
        ])
        .mockResolvedValueOnce([{ name: 'nested.js', isDirectory: () => false } as any])

      const files = await fileOps.listFiles('.', true)

      expect(files).toEqual(['file1.ts', 'subdir/', 'subdir/nested.js'])
    })

    it('should prevent path traversal attacks', async () => {
      await expect(fileOps.listFiles('../..')).rejects.toThrow(
        'Access denied: Path outside workspace',
      )
    })

    it('should handle single file path', async () => {
      vi.mocked(fs.stat).mockResolvedValue({
        isFile: () => true,
        isDirectory: () => false,
      } as any)

      const files = await fileOps.listFiles('single-file.ts')

      expect(files).toEqual(['single-file.ts'])
    })
  })

  describe('readFile', () => {
    it('should read file content', async () => {
      vi.mocked(fs.stat).mockResolvedValue({
        isFile: () => true,
        size: 100,
      } as any)
      vi.mocked(fs.readFile).mockResolvedValue('file content')

      const content = await fileOps.readFile('test.txt')

      expect(content).toBe('file content')
      expect(fs.readFile).toHaveBeenCalledWith(path.join(workspacePath, 'test.txt'), 'utf-8')
    })

    it('should prevent reading files outside workspace', async () => {
      await expect(fileOps.readFile('../../../etc/passwd')).rejects.toThrow(
        'Access denied: path outside workspace',
      )
    })

    it('should reject files that are too large', async () => {
      vi.mocked(fs.stat).mockResolvedValue({
        isFile: () => true,
        size: 11 * 1024 * 1024, // 11MB
      } as any)

      await expect(fileOps.readFile('large.txt')).rejects.toThrow('File too large to read')
    })

    it('should reject directories', async () => {
      vi.mocked(fs.stat).mockResolvedValue({
        isFile: () => false,
        isDirectory: () => true,
      } as any)

      await expect(fileOps.readFile('directory')).rejects.toThrow('Path is not a file')
    })

    it('should handle file not found', async () => {
      vi.mocked(fs.stat).mockRejectedValue({ code: 'ENOENT' })

      await expect(fileOps.readFile('nonexistent.txt')).rejects.toThrow(
        'File not found: nonexistent.txt',
      )
    })
  })

  describe('writeFile', () => {
    it('should write file content', async () => {
      vi.mocked(fs.mkdir).mockResolvedValue(undefined)
      vi.mocked(fs.writeFile).mockResolvedValue(undefined)

      await fileOps.writeFile('test.txt', 'new content')

      expect(fs.mkdir).toHaveBeenCalledWith(path.dirname(path.join(workspacePath, 'test.txt')), {
        recursive: true,
      })
      expect(fs.writeFile).toHaveBeenCalledWith(
        path.join(workspacePath, 'test.txt'),
        'new content',
        'utf-8',
      )
    })

    it('should prevent writing files outside workspace', async () => {
      await expect(fileOps.writeFile('../../../etc/passwd', 'malicious')).rejects.toThrow(
        'Access denied: path outside workspace',
      )
    })

    it('should create directories as needed', async () => {
      vi.mocked(fs.mkdir).mockResolvedValue(undefined)
      vi.mocked(fs.writeFile).mockResolvedValue(undefined)

      await fileOps.writeFile('deep/nested/file.txt', 'content')

      expect(fs.mkdir).toHaveBeenCalledWith(path.join(workspacePath, 'deep/nested'), {
        recursive: true,
      })
    })
  })

  describe('searchFiles', () => {
    it('should search files in workspace', async () => {
      vi.mocked(fs.access).mockResolvedValue(undefined)
      const { searchFilesInDirectoryNew } = await import('../../utils/file-search.js')
      vi.mocked(searchFilesInDirectoryNew).mockResolvedValue({
        results: [
          { path: 'src/index.ts', absolutePath: '/test/workspace/src/index.ts', score: 1.0 },
          { path: 'src/utils.ts', absolutePath: '/test/workspace/src/utils.ts', score: 0.8 },
        ],
        totalFound: 2,
        query: 'index',
      })

      const result = await fileOps.searchFiles({ query: 'index' })

      expect(result.results).toHaveLength(2)
      expect(result.query).toBe('index')
      expect(searchFilesInDirectoryNew).toHaveBeenCalledWith(workspacePath, { query: 'index' })
    })

    it('should throw error if workspace does not exist', async () => {
      vi.mocked(fs.access).mockRejectedValue(new Error('ENOENT'))

      await expect(fileOps.searchFiles({ query: 'test' })).rejects.toThrow(
        'Workspace path does not exist',
      )
    })
  })

  describe('getGitDiff', () => {
    it('should get git diff', async () => {
      vi.mocked(fs.access).mockResolvedValue(undefined)
      const { getGitDiffDetailed } = await import('../../utils/git-utils.js')
      vi.mocked(getGitDiffDetailed).mockResolvedValue({
        files: [{ path: 'test.ts', status: 'modified', additions: 5, deletions: 2 }],
        summary: { insertions: 5, deletions: 2, filesChanged: 1 },
      } as any)

      const diff = await fileOps.getGitDiff()

      expect(diff.files).toHaveLength(1)
      expect(getGitDiffDetailed).toHaveBeenCalledWith(workspacePath, originalBranch)
    })

    it('should throw error if workspace does not exist', async () => {
      vi.mocked(fs.access).mockRejectedValue(new Error('ENOENT'))

      await expect(fileOps.getGitDiff()).rejects.toThrow('Workspace path does not exist')
    })
  })

  describe('hasUncommittedChanges', () => {
    it('should return true when there are uncommitted changes', async () => {
      vi.mocked(fs.access).mockResolvedValue(undefined)
      const { hasUncommittedChanges } = await import('../../utils/git-utils.js')
      vi.mocked(hasUncommittedChanges).mockResolvedValue(true)

      const hasChanges = await fileOps.hasUncommittedChanges()

      expect(hasChanges).toBe(true)
      expect(hasUncommittedChanges).toHaveBeenCalledWith(workspacePath)
    })

    it('should return false when workspace does not exist', async () => {
      vi.mocked(fs.access).mockRejectedValue(new Error('ENOENT'))

      const hasChanges = await fileOps.hasUncommittedChanges()

      expect(hasChanges).toBe(false)
    })
  })

  describe('getGitStatus', () => {
    it('should return git status with file counts', async () => {
      vi.mocked(fs.access).mockResolvedValue(undefined)
      const { getOriginSyncStatus, checkForConflicts, getGitDiffDetailed } = await import(
        '../../utils/git-utils.js'
      )

      vi.mocked(getOriginSyncStatus).mockResolvedValue({
        currentBranch: 'feature-branch',
        baseBranch: 'main',
        hasUncommittedChanges: true,
        status: 'ahead',
        commitsAhead: 2,
        commitsBehind: 0,
        originExists: true,
      } as any)

      vi.mocked(checkForConflicts).mockResolvedValue(false)

      vi.mocked(getGitDiffDetailed).mockResolvedValue({
        files: [
          { path: 'new.ts', status: 'added' },
          { path: 'deleted.ts', status: 'deleted' },
          { path: 'modified.ts', status: 'modified' },
          { path: 'another.ts', status: 'modified' },
        ],
        summary: { insertions: 10, deletions: 5, filesChanged: 4 },
      } as any)

      const status = await fileOps.getGitStatus()

      expect(status).toEqual({
        currentBranch: 'feature-branch',
        baseBranch: 'main',
        hasUncommittedChanges: true,
        hasConflicts: false,
        status: 'ahead',
        commitsAhead: 2,
        commitsBehind: 0,
        originExists: true,
        additions: 1,
        deletions: 1,
        modifications: 2,
      })
    })

    it('should return default status when workspace does not exist', async () => {
      vi.mocked(fs.access).mockRejectedValue(new Error('ENOENT'))

      const status = await fileOps.getGitStatus()

      expect(status).toEqual({
        currentBranch: 'unknown',
        baseBranch: 'main',
        hasUncommittedChanges: false,
        hasConflicts: false,
        status: 'error',
        commitsAhead: 0,
        commitsBehind: 0,
        originExists: false,
        additions: 0,
        deletions: 0,
        modifications: 0,
      })
    })
  })

  describe('getGitChanges', () => {
    it('should return working directory changes', async () => {
      vi.mocked(fs.access).mockResolvedValue(undefined)
      const { getWorkingDirectoryChanges } = await import('../../utils/git-utils.js')

      vi.mocked(getWorkingDirectoryChanges).mockResolvedValue({
        currentBranch: 'feature-branch',
        status: 'modified',
        additions: 15,
        deletions: 5,
        modifications: 3,
      })

      const changes = await fileOps.getGitChanges()

      expect(changes).toEqual({
        currentBranch: 'feature-branch',
        status: 'modified',
        additions: 15,
        deletions: 5,
        modifications: 3,
      })
    })

    it('should return error status when workspace does not exist', async () => {
      vi.mocked(fs.access).mockRejectedValue(new Error('ENOENT'))

      const changes = await fileOps.getGitChanges()

      expect(changes).toEqual({
        currentBranch: 'unknown',
        status: 'error',
        additions: 0,
        deletions: 0,
        modifications: 0,
      })
    })
  })

  describe('workspace path management', () => {
    it('should get workspace path', () => {
      expect(fileOps.getWorkspacePath()).toBe(workspacePath)
    })

    it('should update workspace path', () => {
      const newPath = '/new/workspace/path'
      fileOps.setWorkspacePath(newPath)
      expect(fileOps.getWorkspacePath()).toBe(newPath)
    })
  })
})
