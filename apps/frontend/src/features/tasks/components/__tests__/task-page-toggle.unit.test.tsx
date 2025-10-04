import { render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createMockQuery } from '@/test-utils/mocks'
import { TaskPage } from '../task-page'

vi.mock('@tanstack/react-router', () => ({
  useParams: vi.fn(() => ({ taskId: 'task-123' })),
}))

vi.mock('../../contexts/task-providers', () => ({
  useTaskId: vi.fn(),
  usePanelView: vi.fn(),
}))

vi.mock('../../hooks/use-workspace-status', () => ({
  useWorkspaceStatus: vi.fn(),
}))

vi.mock('../../hooks/use-task-queries', () => ({
  useTaskDetailQuery: vi.fn(),
}))

vi.mock('../chat-view/chat-container.tsx', () => ({
  ChatContainer: ({ className }: { className?: string }) => (
    <div data-testid="chat-view" className={className}>
      Chat
    </div>
  ),
}))

vi.mock('../task-header/task-left-header', () => ({
  TaskLeftHeader: () => <div data-testid="task-header">Header</div>,
}))

vi.mock('@/features/file-viewer', () => ({
  FileExplorer: () => <div data-testid="file-explorer">File Explorer</div>,
}))

vi.mock('@/features/preview', () => ({
  Preview: () => <div data-testid="preview">Preview</div>,
}))

vi.mock('@/lib/components/error-boundary', () => ({
  ErrorBoundary: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}))

vi.mock('@/lib/components/ui/resizable', () => ({
  ResizablePanelGroup: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  ResizablePanel: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  ResizableHandle: () => <div data-testid="resize-handle" />,
}))

import type { WorkspaceStatusResponse } from '@moatless/types'
import { usePanelView, useTaskId } from '../../contexts/task-providers'
import { useTaskDetailQuery } from '../../hooks/use-task-queries'
import { useWorkspaceStatus } from '../../hooks/use-workspace-status'

/**
 * Helper to create a complete WorkspaceStatusResponse mock with defaults
 */
function createWorkspaceStatusMock(
  overrides?: Partial<WorkspaceStatusResponse>,
): WorkspaceStatusResponse {
  return {
    workspaceState: 'exists',
    workspacePath: '/test/workspace',
    environmentType: 'local',
    environmentStatus: 'ready',
    environmentCreatedAt: new Date().toISOString(),
    agentState: 'idle',
    capabilities: {
      fileOperations: true,
      commandExecution: true,
      serverManagement: false,
    },
    isRunning: false,
    workspaceReady: true,
    canExecute: true,
    configOutdated: false,
    ...overrides,
  }
}

describe('TaskPage - Toggle Functionality', () => {
  const mockSetRightPanelView = vi.fn()
  const taskMock = {
    taskId: 'task-123',
    status: 'open',
    agentType: 'claude-code',
    repositoryId: 'repo-123',
    iterations: [],
    createdAt: new Date().toISOString(),
    name: 'Test Task',
    description: 'Test description',
    vibe: false,
    templateId: undefined,
  }

  beforeEach(() => {
    vi.clearAllMocks()

    vi.mocked(useTaskId).mockReturnValue('task-123')
    vi.mocked(useTaskDetailQuery).mockReturnValue(createMockQuery(taskMock))
    vi.mocked(useWorkspaceStatus).mockReturnValue(createMockQuery(createWorkspaceStatusMock()))
  })

  describe('Initial Default View', () => {
    it('defaults to preview when serverManagement is true', async () => {
      // Start with files view (default for code tasks)
      vi.mocked(usePanelView).mockReturnValue({
        rightPanelView: 'files',
        setRightPanelView: mockSetRightPanelView,
      })

      // Workspace has serverManagement capability
      vi.mocked(useWorkspaceStatus).mockReturnValue(
        createMockQuery(
          createWorkspaceStatusMock({
            capabilities: {
              fileOperations: true,
              commandExecution: true,
              serverManagement: true,
            },
          }),
        ),
      )

      render(<TaskPage />)

      // Should call setRightPanelView to switch to preview
      await waitFor(() => {
        expect(mockSetRightPanelView).toHaveBeenCalledWith('preview')
      })
    })

    it('stays on files view when serverManagement is false', async () => {
      vi.mocked(usePanelView).mockReturnValue({
        rightPanelView: 'files',
        setRightPanelView: mockSetRightPanelView,
      })

      // Workspace does NOT have serverManagement capability
      vi.mocked(useWorkspaceStatus).mockReturnValue(createMockQuery(createWorkspaceStatusMock()))

      render(<TaskPage />)

      // Should NOT call setRightPanelView
      await waitFor(() => {
        expect(mockSetRightPanelView).not.toHaveBeenCalled()
      })
    })

    it('respects preview view when serverManagement is true', async () => {
      // User already on preview view
      vi.mocked(usePanelView).mockReturnValue({
        rightPanelView: 'preview',
        setRightPanelView: mockSetRightPanelView,
      })

      vi.mocked(useWorkspaceStatus).mockReturnValue(
        createMockQuery(
          createWorkspaceStatusMock({
            capabilities: {
              fileOperations: true,
              commandExecution: true,
              serverManagement: true,
            },
          }),
        ),
      )

      render(<TaskPage />)

      // Should NOT call setRightPanelView (already on preview)
      await waitFor(() => {
        expect(mockSetRightPanelView).not.toHaveBeenCalled()
      })
    })
  })

  describe('User Toggle Behavior', () => {
    it('renders file explorer when rightPanelView is files', () => {
      vi.mocked(usePanelView).mockReturnValue({
        rightPanelView: 'files',
        setRightPanelView: mockSetRightPanelView,
      })

      vi.mocked(useWorkspaceStatus).mockReturnValue(createMockQuery(createWorkspaceStatusMock()))

      render(<TaskPage />)

      expect(screen.getByTestId('file-explorer')).toBeInTheDocument()
      expect(screen.queryByTestId('preview')).not.toBeInTheDocument()
    })

    it('renders preview when rightPanelView is preview', () => {
      vi.mocked(usePanelView).mockReturnValue({
        rightPanelView: 'preview',
        setRightPanelView: mockSetRightPanelView,
      })

      vi.mocked(useWorkspaceStatus).mockReturnValue(
        createMockQuery(
          createWorkspaceStatusMock({
            capabilities: {
              fileOperations: true,
              commandExecution: true,
              serverManagement: true,
            },
          }),
        ),
      )

      render(<TaskPage />)

      expect(screen.getByTestId('preview')).toBeInTheDocument()
      expect(screen.queryByTestId('file-explorer')).not.toBeInTheDocument()
    })

    it('allows toggling to files even with serverManagement', async () => {
      // User toggles to files view
      vi.mocked(usePanelView).mockReturnValue({
        rightPanelView: 'files',
        setRightPanelView: mockSetRightPanelView,
      })

      vi.mocked(useWorkspaceStatus).mockReturnValue(
        createMockQuery(
          createWorkspaceStatusMock({
            capabilities: {
              fileOperations: true,
              commandExecution: true,
              serverManagement: true,
            },
          }),
        ),
      )

      // First render would trigger the effect, but the ref prevents re-running
      const { rerender } = render(<TaskPage />)

      // Simulate user clicking Files toggle - change rightPanelView to 'files'
      vi.mocked(usePanelView).mockReturnValue({
        rightPanelView: 'files',
        setRightPanelView: mockSetRightPanelView,
      })

      rerender(<TaskPage />)

      // File explorer should be visible
      expect(screen.getByTestId('file-explorer')).toBeInTheDocument()
    })
  })

  describe('Loading States', () => {
    it('shows skeleton when task is loading', () => {
      vi.mocked(useTaskDetailQuery).mockReturnValue(
        createMockQuery(undefined, {
          data: undefined,
          isLoading: true,
          isSuccess: false,
          status: 'pending',
        }),
      )

      vi.mocked(usePanelView).mockReturnValue({
        rightPanelView: 'files',
        setRightPanelView: mockSetRightPanelView,
      })

      vi.mocked(useWorkspaceStatus).mockReturnValue(createMockQuery(createWorkspaceStatusMock()))

      render(<TaskPage />)

      // Should not show file explorer or preview during loading
      expect(screen.queryByTestId('file-explorer')).not.toBeInTheDocument()
      expect(screen.queryByTestId('preview')).not.toBeInTheDocument()
    })

    it('shows skeleton when workspace status is loading', () => {
      vi.mocked(useTaskDetailQuery).mockReturnValue(createMockQuery(taskMock))

      vi.mocked(usePanelView).mockReturnValue({
        rightPanelView: 'files',
        setRightPanelView: mockSetRightPanelView,
      })

      // Workspace status is loading
      vi.mocked(useWorkspaceStatus).mockReturnValue(
        createMockQuery(undefined, {
          isLoading: true,
          data: undefined,
          isSuccess: false,
          status: 'pending',
        }),
      )

      render(<TaskPage />)

      // Should show loading state, not panels
      expect(screen.queryByTestId('file-explorer')).not.toBeInTheDocument()
      expect(screen.queryByTestId('preview')).not.toBeInTheDocument()
    })
  })
})
