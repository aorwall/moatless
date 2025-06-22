import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  createMockGitDiff,
  createMockTaskIteration,
} from '@/test-utils/factories';
import { renderWithRouter } from '@/test-utils/render';
import type { GitDiffFile } from '@/types/git-diff';

import { useTaskGitDiff } from '../../hooks/use-tasks';
import { useSelectedIteration } from '../../stores/task-ui-store';
import { GitDiffViewer } from '../git-diff-viewer/git-diff-viewer';

// Mock dependencies
vi.mock('../../hooks/use-tasks');
vi.mock('../../stores/task-ui-store');
vi.mock('@tanstack/react-router', () => ({
  useParams: () => ({ taskId: 'task-123' }),
}));

// Mock the git-diff-view library
vi.mock('@git-diff-view/react', () => ({
  DiffModeEnum: { Unified: 'unified', Split: 'split' },
  DiffView: ({ diffFile }: any) => (
    <div data-testid="diff-view" data-mode={diffFile.mode}>
      Diff content for {diffFile.path}
    </div>
  ),
}));

vi.mock('@git-diff-view/file', () => ({
  generateDiffFile: (
    oldPath: string,
    _oldContent: string,
    newPath: string,
    _newContent: string,
  ) => ({
    path: newPath,
    oldPath,
    mode: 'unified',
    init: vi.fn(),
    initTheme: vi.fn(),
    buildSplitDiffLines: vi.fn(),
    buildUnifiedDiffLines: vi.fn(),
  }),
}));

// Helper function for creating mock files (global scope)
const createMockFile = (overrides?: Partial<GitDiffFile>): GitDiffFile => ({
  path: 'src/test.ts',
  old_path: 'src/test.ts',
  status: 'modified',
  additions: 5,
  deletions: 3,
  old_content: 'old content',
  new_content: 'new content',
  ...overrides,
});

describe('GitDiffViewer', () => {
  beforeEach(() => {
    vi.clearAllMocks();

    // Default mocks
    vi.mocked(useSelectedIteration).mockReturnValue(null);
    vi.mocked(useTaskGitDiff).mockReturnValue({
      data: createMockGitDiff({
        files: [],
        total_files: 0,
        total_additions: 0,
        total_deletions: 0,
      }),
      isLoading: false,
      error: null,
    } as any);

    // Mock document.documentElement.classList
    Object.defineProperty(document.documentElement, 'classList', {
      value: {
        contains: vi.fn().mockReturnValue(false),
      },
      writable: true,
    });
  });

  describe('Loading and Error States', () => {
    it('shows loading state', () => {
      vi.mocked(useTaskGitDiff).mockReturnValue({
        data: undefined,
        isLoading: true,
        error: null,
      } as any);

      renderWithRouter(<GitDiffViewer />);

      expect(screen.getByText('Loading diff...')).toBeInTheDocument();
      expect(screen.getByTestId('loading-spinner')).toHaveClass('animate-spin');
    });

    it('shows error state', () => {
      const error = new Error('Failed to fetch diff');
      vi.mocked(useTaskGitDiff).mockReturnValue({
        data: undefined,
        isLoading: false,
        error,
      } as any);

      renderWithRouter(<GitDiffViewer />);

      expect(screen.getByText('Failed to load git diff')).toBeInTheDocument();
      expect(screen.getByText('Failed to fetch diff')).toBeInTheDocument();
    });

    it('shows empty state when no files', () => {
      renderWithRouter(<GitDiffViewer />);

      expect(screen.getByText('No changes to display')).toBeInTheDocument();
    });
  });

  describe('Diff Display', () => {
    it('displays diff summary', () => {
      const mockDiff = createMockGitDiff({
        files: [createMockFile()],
        total_files: 3,
        total_additions: 25,
        total_deletions: 10,
        branch: 'feature-branch',
      });

      vi.mocked(useTaskGitDiff).mockReturnValue({
        data: mockDiff,
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<GitDiffViewer />);

      expect(screen.getByText('3 files changed')).toBeInTheDocument();
      expect(screen.getByText('+25')).toBeInTheDocument();
      expect(screen.getByText('-10')).toBeInTheDocument();
      expect(screen.getByText('on feature-branch')).toBeInTheDocument();
    });

    it('displays single file text correctly', () => {
      const mockDiff = createMockGitDiff({
        files: [createMockFile()],
        total_files: 1,
      });

      vi.mocked(useTaskGitDiff).mockReturnValue({
        data: mockDiff,
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<GitDiffViewer />);

      expect(screen.getByText('1 file changed')).toBeInTheDocument();
    });

    it('renders file diffs', () => {
      const files = [
        createMockFile({ path: 'file1.ts', status: 'added' }),
        createMockFile({ path: 'file2.ts', status: 'modified' }),
        createMockFile({ path: 'file3.ts', status: 'deleted' }),
      ];

      vi.mocked(useTaskGitDiff).mockReturnValue({
        data: createMockGitDiff({ files }),
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<GitDiffViewer />);

      expect(screen.getByText('file1.ts')).toBeInTheDocument();
      expect(screen.getByText('file2.ts')).toBeInTheDocument();
      expect(screen.getByText('file3.ts')).toBeInTheDocument();
    });

    it('displays file status badges', () => {
      const files = [
        createMockFile({ status: 'added' }),
        createMockFile({ status: 'deleted' }),
        createMockFile({ status: 'modified' }),
        createMockFile({ status: 'renamed' }),
      ];

      vi.mocked(useTaskGitDiff).mockReturnValue({
        data: createMockGitDiff({ files }),
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<GitDiffViewer />);

      expect(screen.getByText('added')).toBeInTheDocument();
      expect(screen.getByText('deleted')).toBeInTheDocument();
      expect(screen.getByText('modified')).toBeInTheDocument();
      expect(screen.getByText('renamed')).toBeInTheDocument();
    });

    it('shows rename with old and new paths', () => {
      const file = createMockFile({
        path: 'new-name.ts',
        old_path: 'old-name.ts',
        status: 'renamed',
      });

      vi.mocked(useTaskGitDiff).mockReturnValue({
        data: createMockGitDiff({ files: [file] }),
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<GitDiffViewer />);

      expect(screen.getByText('old-name.ts')).toHaveClass('line-through');
      expect(screen.getByText('new-name.ts')).toBeInTheDocument();
      expect(screen.getByText('→')).toBeInTheDocument();
    });

    it('shows additions and deletions for each file', () => {
      const file = createMockFile({
        additions: 15,
        deletions: 8,
      });

      vi.mocked(useTaskGitDiff).mockReturnValue({
        data: createMockGitDiff({ files: [file] }),
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<GitDiffViewer />);

      expect(screen.getByText('+15')).toBeInTheDocument();
      expect(screen.getByText('-8')).toBeInTheDocument();
    });
  });

  describe('Iteration Diff Priority', () => {
    it('uses iteration git diff when available', () => {
      const iterationDiff = {
        files: [createMockFile({ path: 'iteration-file.ts', status: 'added' })],
        total_files: 1,
        total_additions: 10,
        total_deletions: 0,
      } as any;

      vi.mocked(useSelectedIteration).mockReturnValue(
        createMockTaskIteration({ git_diff: iterationDiff }),
      );

      const contextDiff = createMockGitDiff({
        files: [
          createMockFile({ path: 'context-file.ts', status: 'modified' }),
        ],
        total_files: 1,
        total_additions: 5,
        total_deletions: 3,
      });

      vi.mocked(useTaskGitDiff).mockReturnValue({
        data: contextDiff,
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<GitDiffViewer />);

      // Should show iteration diff, not context diff
      expect(screen.getByText('iteration-file.ts')).toBeInTheDocument();
      expect(screen.queryByText('context-file.ts')).not.toBeInTheDocument();
    });

    it('falls back to context diff when iteration has no diff', () => {
      vi.mocked(useSelectedIteration).mockReturnValue(
        createMockTaskIteration({ git_diff: undefined }),
      );

      const contextDiff = createMockGitDiff({
        files: [
          createMockFile({ path: 'context-file.ts', status: 'modified' }),
        ],
        total_files: 1,
        total_additions: 5,
        total_deletions: 3,
      });

      vi.mocked(useTaskGitDiff).mockReturnValue({
        data: contextDiff,
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<GitDiffViewer />);

      expect(screen.getByText('context-file.ts')).toBeInTheDocument();
    });

    it('handles string git diff from iteration', () => {
      // When git_diff is a string (raw diff), it's currently ignored
      vi.mocked(useSelectedIteration).mockReturnValue(
        createMockTaskIteration({
          git_diff: 'diff --git a/file.ts b/file.ts' as any,
        }),
      );

      const contextDiff = createMockGitDiff({
        files: [
          createMockFile({ path: 'context-file.ts', status: 'modified' }),
        ],
        total_files: 1,
        total_additions: 5,
        total_deletions: 3,
      });

      vi.mocked(useTaskGitDiff).mockReturnValue({
        data: contextDiff,
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<GitDiffViewer />);

      // Should fall back to context diff
      expect(screen.getByText('context-file.ts')).toBeInTheDocument();
    });
  });

  describe('Dark Mode Support', () => {
    it('applies dark theme when in dark mode', () => {
      document.documentElement.classList.contains = vi
        .fn()
        .mockReturnValue(true);

      const file = createMockFile();
      vi.mocked(useTaskGitDiff).mockReturnValue({
        data: createMockGitDiff({ files: [file] }),
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<GitDiffViewer />);

      const wrapper = screen
        .getByTestId('diff-view')
        .closest('.git-diff-view-wrapper');
      expect(wrapper).toHaveClass('diff-view-theme-dark');
    });

    it('applies light theme when not in dark mode', () => {
      document.documentElement.classList.contains = vi
        .fn()
        .mockReturnValue(false);

      const file = createMockFile();
      vi.mocked(useTaskGitDiff).mockReturnValue({
        data: createMockGitDiff({ files: [file] }),
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<GitDiffViewer />);

      const wrapper = screen
        .getByTestId('diff-view')
        .closest('.git-diff-view-wrapper');
      expect(wrapper).toHaveClass('diff-view-theme-light');
    });
  });

  describe('Collapsible Behavior', () => {
    it('auto-expands small diffs', () => {
      const file = createMockFile({
        additions: 10,
        deletions: 5,
      });

      vi.mocked(useTaskGitDiff).mockReturnValue({
        data: createMockGitDiff({ files: [file] }),
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<GitDiffViewer />);

      // Should be expanded by default (total changes < 100)
      expect(screen.getByTestId('diff-view')).toBeInTheDocument();
    });

    it('collapses large diffs by default', async () => {
      const file = createMockFile({
        additions: 80,
        deletions: 30,
      });

      vi.mocked(useTaskGitDiff).mockReturnValue({
        data: createMockGitDiff({ files: [file] }),
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<GitDiffViewer />);

      // Should be collapsed by default (total changes >= 100)
      expect(screen.queryByTestId('diff-view')).not.toBeInTheDocument();
    });

    it('toggles diff visibility on click', async () => {
      const user = userEvent.setup();
      const file = createMockFile();

      vi.mocked(useTaskGitDiff).mockReturnValue({
        data: createMockGitDiff({ files: [file] }),
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<GitDiffViewer />);

      // Find the collapsible trigger
      const trigger = screen.getByRole('button', { name: /test\.ts/i });

      // Initially expanded
      expect(screen.getByTestId('diff-view')).toBeInTheDocument();

      // Click to collapse
      await user.click(trigger);
      expect(screen.queryByTestId('diff-view')).not.toBeInTheDocument();

      // Click to expand
      await user.click(trigger);
      expect(screen.getByTestId('diff-view')).toBeInTheDocument();
    });
  });

  describe('Props and Styling', () => {
    it('applies custom className', () => {
      const file = createMockFile();
      vi.mocked(useTaskGitDiff).mockReturnValue({
        data: createMockGitDiff({ files: [file] }),
        isLoading: false,
        error: null,
      } as any);

      const { container } = renderWithRouter(
        <GitDiffViewer className="custom-class" />,
      );

      expect(container.querySelector('.custom-class')).toBeInTheDocument();
    });

    it('handles files without old_path', () => {
      const file = createMockFile({
        path: 'new-file.ts',
        old_path: undefined,
        status: 'added',
      });

      vi.mocked(useTaskGitDiff).mockReturnValue({
        data: createMockGitDiff({ files: [file] }),
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<GitDiffViewer />);

      expect(screen.getByText('new-file.ts')).toBeInTheDocument();
      expect(screen.queryByText('→')).not.toBeInTheDocument();
    });
  });

  describe('File Type Detection', () => {
    it('extracts file extension for syntax highlighting', () => {
      const files = [
        createMockFile({ path: 'test.ts' }),
        createMockFile({ path: 'styles.css' }),
        createMockFile({ path: 'config.json' }),
        createMockFile({ path: 'README' }), // No extension
      ];

      vi.mocked(useTaskGitDiff).mockReturnValue({
        data: createMockGitDiff({ files }),
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<GitDiffViewer />);

      // Verify all files are rendered
      expect(screen.getByText('test.ts')).toBeInTheDocument();
      expect(screen.getByText('styles.css')).toBeInTheDocument();
      expect(screen.getByText('config.json')).toBeInTheDocument();
      expect(screen.getByText('README')).toBeInTheDocument();
    });
  });
});
