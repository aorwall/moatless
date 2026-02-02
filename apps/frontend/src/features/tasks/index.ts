// Task feature exports

// API
export * from './api/tasks'

// Components
export { CreateTaskForm } from './components/create-task-form'
export { TaskPage } from './components/task-page'

// Tri-Context Pattern for performance optimization
export {
  TaskDataProvider,
  TaskProviders,
  TaskUIProvider,
  // Task context hooks
  useEventStream,
  // UI hooks
  useIsAgentRunning,
  useLatestContextTokens,
  usePanelSizes,
  useStreamingStatus,
  useTaskEvents,
  useTaskId,
  useTaskMetadata,
  useTaskStatus,
  useTaskStreamingState,
  useTaskUI,
} from './contexts/task-providers'

// URL state hooks and types
export {
  type TaskView,
  usePreviewPath,
  usePreviewViewport,
  useSelectedFile,
  useTaskView,
  type ViewportMode,
} from './hooks/use-task-view'

// Hooks
export * from './hooks/use-tasks'
export * from './hooks/use-workspace-status'

// Utils
export * from './utils/format-duration'
export * from './utils/query-helpers'
