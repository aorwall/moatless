import { CheckCircle2, Circle, Clock, Loader2 } from 'lucide-react'
import { cn } from '@/lib/utils.ts'

export interface Todo {
  id: string
  content: string
  status: 'pending' | 'in_progress' | 'completed'
}

interface TodoListProps {
  todos: Todo[]
  showSpinner?: boolean
}

function getStatusIcon(status: Todo['status'], showSpinner?: boolean) {
  switch (status) {
    case 'completed':
      return <CheckCircle2 className="h-3 w-3 text-green-500" />
    case 'in_progress':
      return showSpinner ? (
        <Loader2 className="h-3 w-3 text-blue-500 animate-spin" />
      ) : (
        <Clock className="h-3 w-3 text-blue-500" />
      )
    case 'pending':
      return <Circle className="h-3 w-3 text-muted-foreground" />
  }
}

export function TodoList({ todos, showSpinner = false }: TodoListProps) {
  if (!todos || todos.length === 0) {
    return <div className="text-xs text-muted-foreground font-mono">No todos found</div>
  }

  return (
    <div className="space-y-1">
      {todos.map((todo) => (
        <div key={todo.id} className="flex items-start gap-2">
          <div className="flex-shrink-0 mt-0.5">{getStatusIcon(todo.status, showSpinner)}</div>
          <div
            className={cn(
              'text-xs break-words',
              todo.status === 'completed' && 'line-through text-muted-foreground',
            )}
          >
            {todo.content}
          </div>
        </div>
      ))}
    </div>
  )
}
