import type { FileTreeWithChangesResponse } from '@moatless/types'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createWrapper } from '@/test-utils/render'
import { useFileTreeWithChanges } from '../../api/use-file-tree-with-changes'
import { FileTree } from '../file-tree'

// Mock the API hook
vi.mock('../../api/use-file-tree-with-changes')

describe('FileTree with Git Changes - Integration Tests', () => {
  const mockTaskId = 'test-task-123'
  const mockOnFileSelect = vi.fn()

  const mockFileTreeResponse: FileTreeWithChangesResponse = {
    tree: [
      {
        name: 'src',
        path: 'src',
        type: 'directory',
        isChanged: true,
        children: [
          {
            name: 'modified-file.ts',
            path: 'src/modified-file.ts',
            type: 'file',
            isChanged: true,
            gitStatus: 'modified',
            additions: 5,
            deletions: 2,
            isStaged: false,
          },
          {
            name: 'added-file.ts',
            path: 'src/added-file.ts',
            type: 'file',
            isChanged: true,
            gitStatus: 'added',
            additions: 10,
            deletions: 0,
            isStaged: true,
          },
          {
            name: 'deleted-file.ts',
            path: 'src/deleted-file.ts',
            type: 'file',
            isChanged: true,
            gitStatus: 'deleted',
            additions: 0,
            deletions: 15,
            isStaged: false,
          },
        ],
      },
      {
        name: 'package.json',
        path: 'package.json',
        type: 'file',
        isChanged: false,
      },
    ],
    gitSummary: {
      totalFiles: 3,
      totalAdditions: 15,
      totalDeletions: 17,
      totalModified: 1,
      totalStaged: 1,
      compareBase: 'main',
      hasChanges: true,
    },
  }

  beforeEach(() => {
    vi.clearAllMocks()
    mockOnFileSelect.mockClear()
  })

  it('should display file tree with git status indicators', async () => {
    vi.mocked(useFileTreeWithChanges).mockReturnValue({
      data: mockFileTreeResponse,
      isLoading: false,
      error: null,
      isSuccess: true,
    } as any)

    render(<FileTree taskId={mockTaskId} onFileSelect={mockOnFileSelect} />, {
      wrapper: createWrapper(),
    })

    // Check header with git summary
    expect(screen.getByText('Files')).toBeInTheDocument()
    expect(screen.getByText('3 files')).toBeInTheDocument()
    expect(screen.getByText('+15')).toBeInTheDocument()
    expect(screen.getByText('−17')).toBeInTheDocument()
    expect(screen.getByText('(1 staged)')).toBeInTheDocument()

    // Check directory is expanded (because it contains changes)
    expect(screen.getByText('src')).toBeInTheDocument()

    // Check files with different git statuses
    expect(screen.getByText('modified-file.ts')).toBeInTheDocument()
    expect(screen.getByText('added-file.ts')).toBeInTheDocument()
    expect(screen.getByText('deleted-file.ts')).toBeInTheDocument()

    // Check unchanged file
    expect(screen.getByText('package.json')).toBeInTheDocument()
  })

  it('should show correct git status indicators for different file types', async () => {
    vi.mocked(useFileTreeWithChanges).mockReturnValue({
      data: mockFileTreeResponse,
      isLoading: false,
      error: null,
      isSuccess: true,
    } as any)

    render(<FileTree taskId={mockTaskId} onFileSelect={mockOnFileSelect} />, {
      wrapper: createWrapper(),
    })

    // Check that files have different colors/icons based on git status
    // (This would be more comprehensive with actual DOM inspection)
    const modifiedFile = screen.getByText('modified-file.ts')
    const addedFile = screen.getByText('added-file.ts')
    const deletedFile = screen.getByText('deleted-file.ts')

    expect(modifiedFile).toBeInTheDocument()
    expect(addedFile).toBeInTheDocument()
    expect(deletedFile).toBeInTheDocument()
  })

  it('should display staged badge for staged files', async () => {
    vi.mocked(useFileTreeWithChanges).mockReturnValue({
      data: mockFileTreeResponse,
      isLoading: false,
      error: null,
      isSuccess: true,
    } as any)

    render(<FileTree taskId={mockTaskId} onFileSelect={mockOnFileSelect} />, {
      wrapper: createWrapper(),
    })

    // Check for staged badge
    expect(screen.getByText('staged')).toBeInTheDocument()
  })

  it('should display addition/deletion counts', async () => {
    vi.mocked(useFileTreeWithChanges).mockReturnValue({
      data: mockFileTreeResponse,
      isLoading: false,
      error: null,
      isSuccess: true,
    } as any)

    render(<FileTree taskId={mockTaskId} onFileSelect={mockOnFileSelect} />, {
      wrapper: createWrapper(),
    })

    // Check for addition/deletion counts
    expect(screen.getByText('+5')).toBeInTheDocument() // modified file additions
    expect(screen.getByText('−2')).toBeInTheDocument() // modified file deletions
    expect(screen.getByText('+10')).toBeInTheDocument() // added file additions
    expect(screen.getByText('−0')).toBeInTheDocument() // added file deletions
    expect(screen.getByText('+0')).toBeInTheDocument() // deleted file additions
    expect(screen.getByText('−15')).toBeInTheDocument() // deleted file deletions
  })

  it('should handle file selection', async () => {
    vi.mocked(useFileTreeWithChanges).mockReturnValue({
      data: mockFileTreeResponse,
      isLoading: false,
      error: null,
      isSuccess: true,
    } as any)

    const user = userEvent.setup()
    render(<FileTree taskId={mockTaskId} onFileSelect={mockOnFileSelect} />, {
      wrapper: createWrapper(),
    })

    // Click on a file
    const modifiedFile = screen.getByText('modified-file.ts')
    await user.click(modifiedFile)

    expect(mockOnFileSelect).toHaveBeenCalledWith('src/modified-file.ts')
  })

  it('should expand/collapse directories', async () => {
    vi.mocked(useFileTreeWithChanges).mockReturnValue({
      data: mockFileTreeResponse,
      isLoading: false,
      error: null,
      isSuccess: true,
    } as any)

    const user = userEvent.setup()
    render(<FileTree taskId={mockTaskId} onFileSelect={mockOnFileSelect} />, {
      wrapper: createWrapper(),
    })

    // Directory should be expanded initially (because it contains changes)
    expect(screen.getByText('modified-file.ts')).toBeInTheDocument()

    // Find the chevron button (the collapsible trigger)
    const srcDirButton = screen.getByRole('button', { name: /src/i })
    await user.click(srcDirButton)

    // Files should be hidden after collapse
    expect(screen.queryByText('modified-file.ts')).not.toBeInTheDocument()

    // Click again to expand
    await user.click(srcDirButton)

    // Files should be visible again
    expect(screen.getByText('modified-file.ts')).toBeInTheDocument()
  })

  it('should display loading state', async () => {
    vi.mocked(useFileTreeWithChanges).mockReturnValue({
      data: undefined,
      isLoading: true,
      error: null,
      isSuccess: false,
    } as any)

    render(<FileTree taskId={mockTaskId} onFileSelect={mockOnFileSelect} />, {
      wrapper: createWrapper(),
    })

    // Should show loading skeleton
    expect(screen.getByTestId('file-tree-skeleton')).toBeInTheDocument()
  })

  it('should display error state', async () => {
    const mockError = new Error('Failed to fetch file tree')
    vi.mocked(useFileTreeWithChanges).mockReturnValue({
      data: undefined,
      isLoading: false,
      error: mockError,
      isSuccess: false,
    } as any)

    render(<FileTree taskId={mockTaskId} onFileSelect={mockOnFileSelect} />, {
      wrapper: createWrapper(),
    })

    // Should show error message
    expect(screen.getByText(/Failed to fetch file tree/)).toBeInTheDocument()
  })

  it('should display empty state when no files', async () => {
    const emptyResponse: FileTreeWithChangesResponse = {
      tree: [],
      gitSummary: {
        totalFiles: 0,
        totalAdditions: 0,
        totalDeletions: 0,
        totalModified: 0,
        totalStaged: 0,
        compareBase: 'main',
        hasChanges: false,
      },
    }

    vi.mocked(useFileTreeWithChanges).mockReturnValue({
      data: emptyResponse,
      isLoading: false,
      error: null,
      isSuccess: true,
    } as any)

    render(<FileTree taskId={mockTaskId} onFileSelect={mockOnFileSelect} />, {
      wrapper: createWrapper(),
    })

    expect(screen.getByText('No files found')).toBeInTheDocument()
  })

  it('should not show git summary when there are no changes', async () => {
    const noChangesResponse: FileTreeWithChangesResponse = {
      tree: [
        {
          name: 'package.json',
          path: 'package.json',
          type: 'file',
          isChanged: false,
        },
      ],
      gitSummary: {
        totalFiles: 0,
        totalAdditions: 0,
        totalDeletions: 0,
        totalModified: 0,
        totalStaged: 0,
        compareBase: 'main',
        hasChanges: false,
      },
    }

    vi.mocked(useFileTreeWithChanges).mockReturnValue({
      data: noChangesResponse,
      isLoading: false,
      error: null,
      isSuccess: true,
    } as any)

    render(<FileTree taskId={mockTaskId} onFileSelect={mockOnFileSelect} />, {
      wrapper: createWrapper(),
    })

    // Should not show git summary when no changes
    expect(screen.queryByText(/changed/)).not.toBeInTheDocument()
    expect(screen.getByText('Files')).toBeInTheDocument()
    expect(screen.getByText('package.json')).toBeInTheDocument()
  })

  it('should highlight selected file', async () => {
    vi.mocked(useFileTreeWithChanges).mockReturnValue({
      data: mockFileTreeResponse,
      isLoading: false,
      error: null,
      isSuccess: true,
    } as any)

    const user = userEvent.setup()
    render(<FileTree taskId={mockTaskId} onFileSelect={mockOnFileSelect} />, {
      wrapper: createWrapper(),
    })

    // Click on a file
    const modifiedFile = screen.getByText('modified-file.ts')
    await user.click(modifiedFile)

    // File should be highlighted (has bg-accent class)
    expect(modifiedFile.closest('button')).toHaveClass('bg-accent')
  })

  it('should handle nested directory structure', async () => {
    const nestedResponse: FileTreeWithChangesResponse = {
      tree: [
        {
          name: 'src',
          path: 'src',
          type: 'directory',
          isChanged: true,
          children: [
            {
              name: 'components',
              path: 'src/components',
              type: 'directory',
              isChanged: true,
              children: [
                {
                  name: 'Button.tsx',
                  path: 'src/components/Button.tsx',
                  type: 'file',
                  isChanged: true,
                  gitStatus: 'modified',
                  additions: 3,
                  deletions: 1,
                  isStaged: false,
                },
              ],
            },
          ],
        },
      ],
      gitSummary: {
        totalFiles: 1,
        totalAdditions: 3,
        totalDeletions: 1,
        totalModified: 1,
        totalStaged: 0,
        compareBase: 'main',
        hasChanges: true,
      },
    }

    vi.mocked(useFileTreeWithChanges).mockReturnValue({
      data: nestedResponse,
      isLoading: false,
      error: null,
      isSuccess: true,
    } as any)

    render(<FileTree taskId={mockTaskId} onFileSelect={mockOnFileSelect} />, {
      wrapper: createWrapper(),
    })

    // Should show nested structure
    expect(screen.getByText('src')).toBeInTheDocument()
    expect(screen.getByText('components')).toBeInTheDocument()
    expect(screen.getByText('Button.tsx')).toBeInTheDocument()
  })

  it('should filter to show only changed files when toggle is pressed', async () => {
    vi.mocked(useFileTreeWithChanges).mockReturnValue({
      data: mockFileTreeResponse,
      isLoading: false,
      error: null,
      isSuccess: true,
    } as any)

    const user = userEvent.setup()
    render(<FileTree taskId={mockTaskId} onFileSelect={mockOnFileSelect} />, {
      wrapper: createWrapper(),
    })

    // Initially all files should be visible
    expect(screen.getByText('package.json')).toBeInTheDocument()
    expect(screen.getByText('modified-file.ts')).toBeInTheDocument()

    // Click the filter button
    const filterButton = screen.getByRole('button', { name: /toggle changed files only/i })
    await user.click(filterButton)

    // Only changed files should be visible
    expect(screen.queryByText('package.json')).not.toBeInTheDocument()
    expect(screen.getByText('modified-file.ts')).toBeInTheDocument()
    expect(screen.getByText('added-file.ts')).toBeInTheDocument()
    expect(screen.getByText('deleted-file.ts')).toBeInTheDocument()

    // Click again to show all files
    await user.click(filterButton)

    // All files should be visible again
    expect(screen.getByText('package.json')).toBeInTheDocument()
    expect(screen.getByText('modified-file.ts')).toBeInTheDocument()
  })
})
