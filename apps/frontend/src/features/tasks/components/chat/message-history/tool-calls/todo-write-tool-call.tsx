import { type Todo, TodoList } from './todo-list.tsx'
import type { ToolCallProps } from './tool-call-props.ts'

/** Todo-list rendering for the TodoWrite tool. */
export function TodoWriteToolCall({ content }: ToolCallProps) {
  const toolInput = content.toolInput as { todos: Todo[] }
  const todos = toolInput.todos

  return (
    <div className="text-xs font-mono w-full max-w-full">
      <div className="flex items-start gap-2">
        <div className="w-1.5 h-1.5 bg-primary rounded-full mt-1 flex-shrink-0" />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1 mb-2">
            <span className="font-medium">{content.toolName}</span>
            <span className="text-muted-foreground">({todos.length} items)</span>
          </div>
          <TodoList todos={todos} />
        </div>
      </div>
    </div>
  )
}
