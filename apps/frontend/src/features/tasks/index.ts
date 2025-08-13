// Task feature exports

// API
export * from './api/tasks'

// Components
export { ChatView } from './components/chat-view'
export { CreateTaskForm } from './components/create-task-form'
export { EventViewer } from './components/event-viewer'
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
  useIsAgentRunning,
  useLatestContextTokens,
  usePanelSizes,
  usePanelView,
  useSelectedFile,
  useSelectedIteration,
  useStreamingStatus,
  useTaskData,
  useTaskEvents,
  useTaskIterations,
  useTaskLoadingError,
  useTaskMetadata,
  useTaskStatus,
  useTaskUI,
  useWorkspaceStatus,
} from './contexts/task-providers'

// Hooks
export * from './hooks/use-task-data'
export * from './hooks/use-tasks'
export * from './hooks/use-workspace-status'

// Utils
export * from './utils/format-duration'
export * from './utils/query-helpers'
