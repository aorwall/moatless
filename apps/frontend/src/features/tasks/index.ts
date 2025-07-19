// Task feature exports

// API
export * from './api/tasks'

// Components
export { ChatView } from './components/chat-view'
export { CreateTaskForm } from './components/create-task-form'
export { EventViewer } from './components/event-viewer'
export { GitDiffViewer } from './components/git-diff-viewer'
export { TaskPage } from './components/task-page'

// Tri-Context Pattern for performance optimization
export {
  TaskDataProvider,
  TaskProviders,
  TaskUIProvider,
  // Data hooks
  useEventStream,
  useGitDiff,
  // UI hooks
  useGitDiffState,
  useGitDiffWithLoading,
  useLatestContextTokens,
  usePanelSizes,
  usePanelView,
  useSelectedFile,
  useSelectedIteration,
  useStreamingStatus,
  useTaskData,
  useTaskDuration,
  useTaskEvents,
  useTaskIterations,
  useTaskLoadingError,
  useTaskMetadata,
  useTaskStatus,
  useTaskUI,
  useWorkspaceStatus,
  useWorkspaceStatusWithLoading,
} from './contexts/task-providers'

// Hooks
export * from './hooks/use-task-data'
export * from './hooks/use-tasks'

// Utils
export * from './utils/format-duration'
export * from './utils/query-helpers'
