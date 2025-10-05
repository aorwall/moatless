import { vi } from 'vitest'

export const createMockGitUtils = () => ({
  isGitWorktree: vi.fn().mockResolvedValue(true),
  isGitRepository: vi.fn().mockResolvedValue(true),
  createGitWorktree: vi.fn().mockResolvedValue(undefined),
  getBranchList: vi.fn().mockResolvedValue(['main', 'develop']),
  syncWithMain: vi.fn().mockResolvedValue({
    success: true,
    conflicts: false,
    message: 'Sync successful',
    commitsBehind: 0,
    commitsAhead: 0,
  }),
  getGitStatus: vi.fn().mockResolvedValue({
    currentBranch: 'main',
    hasUncommittedChanges: false,
    hasConflicts: false,
    status: 'up_to_date' as const,
  }),
  getGitDiff: vi.fn().mockResolvedValue({
    files: [],
    totalChanges: 0,
    totalAdditions: 0,
    totalDeletions: 0,
    diffString: '',
    compareBase: 'main',
  }),
  getCurrentBranch: vi.fn().mockResolvedValue('main'),
  getCommitsBehind: vi.fn().mockResolvedValue(0),
  getCommitsAhead: vi.fn().mockResolvedValue(0),
  hasUncommittedChanges: vi.fn().mockResolvedValue(false),
  hasConflicts: vi.fn().mockResolvedValue(false),
  checkRemoteExists: vi.fn().mockResolvedValue(true),
})

// Export individual mocks for convenience
export const mockGitUtils = createMockGitUtils()

// Auto-mock function to be used with vi.mock
export const gitUtilsMock = () => mockGitUtils
