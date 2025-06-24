import { CheckCircle2, Circle, Clock, ListTodo } from 'lucide-react'
import type { ReactNode } from 'react'

interface Todo {
  id: string
  content: string
  status: 'pending' | 'in_progress' | 'completed'
  priority: 'high' | 'medium' | 'low'
}

interface TodoToolCallProps {
  input: {
    todos: Array<Todo>
  }
  name: string
}

export function TodoToolCall({ input, name }: TodoToolCallProps): ReactNode {
  const getStatusIcon = (status: Todo['status']) => {
    switch (status) {
      case 'completed':
        return <CheckCircle2 className="h-3.5 w-3.5 text-green-600 dark:text-green-400" />
      case 'in_progress':
        return <Clock className="h-3.5 w-3.5 text-blue-600 dark:text-blue-400" />
      default:
        return <Circle className="h-3.5 w-3.5 text-muted-foreground" />
    }
  }

  const getPriorityColor = (priority: Todo['priority']) => {
    switch (priority) {
      case 'high':
        return 'text-red-600 dark:text-red-400'
      case 'medium':
        return 'text-yellow-600 dark:text-yellow-400'
      case 'low':
        return 'text-muted-foreground'
      default:
        return 'text-muted-foreground'
    }
  }

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <ListTodo className="h-3.5 w-3.5 text-indigo-600 dark:text-indigo-400" />
        <span className="font-medium text-foreground">{name}</span>
        <span className="text-xs text-muted-foreground">
          - {input.todos.length} item{input.todos.length !== 1 ? 's' : ''}
        </span>
      </div>

      <div className="space-y-1.5 ml-5">
        {input.todos.map((todo) => (
          <div key={todo.id} className="flex items-start gap-2">
            {getStatusIcon(todo.status)}
            <div className="flex-1 min-w-0">
              <span
                className={`text-sm ${todo.status === 'completed' ? 'line-through text-muted-foreground' : ''}`}
              >
                {todo.content}
              </span>
              <span className={`ml-2 text-xs ${getPriorityColor(todo.priority)}`}>
                ({todo.priority})
              </span>
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}
