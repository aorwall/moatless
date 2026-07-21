import { createContext, type ReactNode, useContext } from 'react'

interface TaskDataContextValue {
  taskId: string
}

const TaskDataContext = createContext<TaskDataContextValue | undefined>(undefined)

interface TaskDataProviderProps {
  taskId: string
  children: ReactNode
}

export function TaskDataProvider({ taskId, children }: TaskDataProviderProps) {
  const contextValue: TaskDataContextValue = { taskId }

  return <TaskDataContext value={contextValue}>{children}</TaskDataContext>
}

function useTaskDataContext() {
  const context = useContext(TaskDataContext)
  if (!context) {
    throw new Error('Task data hooks must be used within TaskDataProvider')
  }
  return context
}

export function useTaskId() {
  const { taskId } = useTaskDataContext()
  return taskId
}
