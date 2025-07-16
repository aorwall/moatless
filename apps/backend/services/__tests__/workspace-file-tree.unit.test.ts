import type { RepositoryConfig } from '@moatless/types'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { Workspace } from '../../workspaces/workspace.js'
import { WorkspaceService } from '../workspace.service.js'

// Mock the workspace class
vi.mock('../../workspaces/workspace.js')

describe('WorkspaceService - File Tree with Changes (Unit Tests)', () => {
  let workspaceService: WorkspaceService
  let mockRepositoryService: any
  let mockEventService: any
  let mockStorage: any
  let mockWorkspace: any

  const mockRepository: RepositoryConfig = {
    id: 'repo-1',
    name: 'Test Repo',
    path: '/test/repo',
    useGitWorktrees: true,
    codeExecution: { type: 'local' },
    createdAt: new Date(),
    updatedAt: new Date(),
  }

  const mockTask = {
    id: 'task-1',
    repositoryId: 'repo-1',
    workspacePath: '/test/workspace',
  }

  const mockGitDiff = {
    files: [
      {
        filename: 'src/file1.ts',
        status: 'modified' as const,
        additions: 5,
        deletions: 2,
        isStaged: false,
      },
      {
        filename: 'src/file2.ts',
        status: 'added' as const,
        additions: 10,
        deletions: 0,
        isStaged: true,
      },
      {
        filename: 'README.md',
        status: 'deleted' as const,
        additions: 0,
        deletions: 15,
        isStaged: false,
      },
    ],
    totalChanges: 3,
    totalAdditions: 15,
    totalDeletions: 17,
    compareBase: 'main',
    diffString: 'mock diff string',
  }

  const mockFileList = ['src/', 'src/file1.ts', 'src/file2.ts', 'package.json', 'README.md']

  beforeEach(() => {
    vi.clearAllMocks()

    // Create mocked services
    mockRepositoryService = {
      getRepositoryById: vi.fn(),
    } as any

    mockEventService = {} as any
    mockStorage = {
      getTask: vi.fn(),
    } as any

    // Create mocked workspace
    mockWorkspace = {
      getWorkspacePath: vi.fn().mockReturnValue('/test/workspace'),
      getGitDiff: vi.fn().mockResolvedValue(mockGitDiff),
    } as any

    // Mock workspace constructor
    vi.mocked(Workspace).mockImplementation(() => mockWorkspace)

    workspaceService = new WorkspaceService(mockRepositoryService, mockEventService, mockStorage)

    // Setup common mocks
    mockStorage.getTask.mockResolvedValue(mockTask)
    mockRepositoryService.getRepositoryById.mockResolvedValue(mockRepository)

    // Mock canPerformFileOperations to return true by default
    vi.spyOn(workspaceService, 'canPerformFileOperations').mockResolvedValue(true)
  })

  describe('getFileTreeWithChanges', () => {
    it('should return file tree with git change information', async () => {
      // Mock the private getAllFiles method by spying on fs operations
      const mockGetAllFiles = vi.spyOn(workspaceService as any, 'getAllFiles')
      mockGetAllFiles.mockResolvedValue(mockFileList)

      const result = await workspaceService.getFileTreeWithChanges('task-1')

      expect(result).toBeDefined()
      expect(result.gitSummary).toEqual({
        totalFiles: 3,
        totalAdditions: 15,
        totalDeletions: 17,
        totalModified: 1,
        totalStaged: 1,
        compareBase: 'main',
        hasChanges: true,
      })

      expect(result.tree).toBeDefined()
      expect(Array.isArray(result.tree)).toBe(true)
    })

    it('should build hierarchical tree structure correctly', async () => {
      const mockGetAllFiles = vi.spyOn(workspaceService as any, 'getAllFiles')
      mockGetAllFiles.mockResolvedValue(mockFileList)

      const result = await workspaceService.getFileTreeWithChanges('task-1')

      // Find the src directory
      const srcDir = result.tree.find((node) => node.name === 'src' && node.type === 'directory')
      expect(srcDir).toBeDefined()
      expect(srcDir?.isChanged).toBe(true) // Should be marked as changed since it contains changed files
      expect(srcDir?.children).toBeDefined()
      expect(srcDir?.children?.length).toBeGreaterThan(0)

      // Find the modified file
      const modifiedFile = srcDir?.children?.find((node: any) => node.name === 'file1.ts')
      expect(modifiedFile).toBeDefined()
      expect(modifiedFile?.isChanged).toBe(true)
      expect(modifiedFile?.gitStatus).toBe('modified')
      expect(modifiedFile?.additions).toBe(5)
      expect(modifiedFile?.deletions).toBe(2)
      expect(modifiedFile?.isStaged).toBe(false)

      // Find the added file
      const addedFile = srcDir?.children?.find((node: any) => node.name === 'file2.ts')
      expect(addedFile).toBeDefined()
      expect(addedFile?.isChanged).toBe(true)
      expect(addedFile?.gitStatus).toBe('added')
      expect(addedFile?.isStaged).toBe(true)
    })

    it('should handle files without git changes', async () => {
      const mockGetAllFiles = vi.spyOn(workspaceService as any, 'getAllFiles')
      mockGetAllFiles.mockResolvedValue(mockFileList)

      const result = await workspaceService.getFileTreeWithChanges('task-1')

      // Find package.json which is not in the git diff
      const packageFile = result.tree.find((node) => node.name === 'package.json')
      expect(packageFile).toBeDefined()
      expect(packageFile?.isChanged).toBe(false)
      expect(packageFile?.gitStatus).toBeUndefined()
      expect(packageFile?.additions).toBeUndefined()
      expect(packageFile?.deletions).toBeUndefined()
      expect(packageFile?.isStaged).toBeUndefined()
    })

    it('should throw error if workspace path is not available', async () => {
      mockWorkspace.getWorkspacePath.mockReturnValue('')

      await expect(workspaceService.getFileTreeWithChanges('task-1')).rejects.toThrow(
        'Cannot list files: workspace not ready',
      )
    })

    it('should throw error if cannot list files', async () => {
      // Mock canPerformFileOperations to return false
      vi.spyOn(workspaceService, 'canPerformFileOperations').mockResolvedValue(false)

      await expect(workspaceService.getFileTreeWithChanges('task-1')).rejects.toThrow(
        'Cannot list files: workspace not ready',
      )
    })

    it('should sort tree nodes with directories first', async () => {
      const mockGetAllFiles = vi.spyOn(workspaceService as any, 'getAllFiles')
      mockGetAllFiles.mockResolvedValue([
        'package.json', // file
        'src/', // directory
        'README.md', // file
        'docs/', // directory
      ])

      const result = await workspaceService.getFileTreeWithChanges('task-1')

      // Directories should come first
      expect(result.tree[0].type).toBe('directory')
      expect(result.tree[1].type).toBe('directory')
      expect(result.tree[2].type).toBe('file')
      expect(result.tree[3].type).toBe('file')

      // Should be alphabetically sorted within each type
      expect(result.tree[0].name).toBe('docs')
      expect(result.tree[1].name).toBe('src')
      expect(result.tree[2].name).toBe('package.json')
      expect(result.tree[3].name).toBe('README.md')
    })

    it('should mark parent directories as changed when child files are changed', async () => {
      const mockGetAllFiles = vi.spyOn(workspaceService as any, 'getAllFiles')
      mockGetAllFiles.mockResolvedValue([
        'src/',
        'src/utils/',
        'src/utils/helper.ts',
        'package.json',
      ])

      // Mock git diff with nested file change
      mockWorkspace.getGitDiff.mockResolvedValue({
        ...mockGitDiff,
        files: [
          {
            filename: 'src/utils/helper.ts',
            status: 'modified' as const,
            additions: 3,
            deletions: 1,
            isStaged: false,
          },
        ],
      })

      const result = await workspaceService.getFileTreeWithChanges('task-1')

      const srcDir = result.tree.find((node) => node.name === 'src')
      expect(srcDir?.isChanged).toBe(true) // Parent should be marked as changed

      const utilsDir = srcDir?.children?.find((node: any) => node.name === 'utils')
      expect(utilsDir?.isChanged).toBe(true) // Parent should be marked as changed

      const helperFile = utilsDir?.children?.find((node: any) => node.name === 'helper.ts')
      expect(helperFile?.isChanged).toBe(true) // File itself should be marked as changed
    })

    it('should handle empty git diff correctly', async () => {
      const mockGetAllFiles = vi.spyOn(workspaceService as any, 'getAllFiles')
      mockGetAllFiles.mockResolvedValue(mockFileList)

      mockWorkspace.getGitDiff.mockResolvedValue({
        files: [],
        totalChanges: 0,
        totalAdditions: 0,
        totalDeletions: 0,
        compareBase: 'main',
        diffString: '',
      })

      const result = await workspaceService.getFileTreeWithChanges('task-1')

      expect(result.gitSummary.hasChanges).toBe(false)
      expect(result.gitSummary.totalFiles).toBe(0)
      expect(result.gitSummary.totalStaged).toBe(0)

      // All files should be marked as unchanged
      const allFiles = result.tree.flatMap((node) => [node, ...(node.children || [])])
      const changedFiles = allFiles.filter((node) => node.isChanged)
      expect(changedFiles).toHaveLength(0)
    })
  })
})
