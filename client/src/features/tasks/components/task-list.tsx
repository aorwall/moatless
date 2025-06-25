import type { Task } from '@shared/types/task'
import { Archive, FileText } from 'lucide-react'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/lib/components/ui/tabs'

import { TaskCard } from './task-card'

interface TaskListProps {
  activeTasks: Array<Task>
  archivedTasks: Array<Task>
  loading?: boolean
}

export function TaskList({ activeTasks, archivedTasks, loading }: TaskListProps) {
  return (
    <Tabs defaultValue="tasks" className="w-full">
      <TabsList className="grid w-full grid-cols-2">
        <TabsTrigger value="tasks" className="space-x-2">
          <FileText className="h-4 w-4" />
          <span>Active Tasks</span>
        </TabsTrigger>
        <TabsTrigger value="archive" className="space-x-2">
          <Archive className="h-4 w-4" />
          <span>Finished</span>
        </TabsTrigger>
      </TabsList>

      <TabsContent value="tasks" className="space-y-4">
        {loading ? (
          <div className="text-center py-8 text-muted-foreground">Loading tasks...</div>
        ) : activeTasks.length === 0 ? (
          <div className="text-center py-8 text-muted-foreground">No active tasks</div>
        ) : (
          activeTasks.map((task) => <TaskCard key={task.taskId} task={task} />)
        )}
      </TabsContent>

      <TabsContent value="archive" className="space-y-4">
        {archivedTasks.length === 0 ? (
          <div className="text-center py-8 text-muted-foreground">No finished tasks</div>
        ) : (
          archivedTasks.map((task) => <TaskCard key={task.taskId} task={task} />)
        )}
      </TabsContent>
    </Tabs>
  )
}
