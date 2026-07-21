import type { UiContentBlock } from '@moatless/api-client'
import type { Question } from '@/lib/types/user-input'
import type { Todo } from './todo-list.tsx'

export function hasQuestionsInput(content: UiContentBlock): boolean {
  if (!content.toolInput) {
    return false
  }
  const toolInput = content.toolInput as { questions?: Question[] }
  return !!toolInput.questions && Array.isArray(toolInput.questions)
}

export function hasTodosInput(content: UiContentBlock): boolean {
  if (!content.toolInput) {
    return false
  }
  const toolInput = content.toolInput as { todos?: Todo[] }
  return !!toolInput.todos && Array.isArray(toolInput.todos)
}
