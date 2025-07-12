// Task feature exports

// API
export * from './api/tasks'

// Components
export { ChatView } from './components/chat-view'
export { CreateTaskForm } from './components/create-task-form'
export { EventViewer } from './components/event-viewer'
export { GitDiffViewer } from './components/git-diff-viewer'
export { TaskCard } from './components/task-card'
export { TaskPage } from './components/task-page'

// Dual-Context Pattern (simplified from Tri-Context)
export {
  TaskDataProvider,
  TaskProviders,
  TaskUIProvider,
  useEventStream,
  useGitDiff,
  usePanelSizes,
  usePanelView,
  useSelectedFile,
  useSelectedIteration,
  useStreamingStatus,
  // Specialized hooks for efficient re-renders
  useTaskData,
  useTaskEvents,
  useTaskIterations,
  useTaskStatus,
  useTaskUI,
  useWorkspaceStatus,
} from './contexts/task-providers'

// Hooks
export * from './hooks/use-task-data'
export * from './hooks/use-tasks'

// Utils
export * from './utils/format-duration'
export * from './utils/query-helpers'
