import type { UiContentBlock } from '@moatless/api-client'
import type { ReactNode } from 'react'
import { AskUserQuestionToolCall } from './ask-user-question-tool-call.tsx'
import { CodexToolCall } from './codex-tool-call.tsx'
import { ExitPlanModeToolCall } from './exit-plan-mode-tool-call.tsx'
import { GenericToolCall } from './generic-tool-call.tsx'
import { TodoWriteToolCall } from './todo-write-tool-call.tsx'
import { hasQuestionsInput, hasTodosInput } from './tool-call-input-guards.ts'
import type { ToolCallProps } from './tool-call-props.ts'

interface SpecialToolRenderer {
  /** Whether this renderer can handle the block; falls back to the generic renderer otherwise. */
  canRender: (content: UiContentBlock) => boolean
  Component: (props: ToolCallProps) => ReactNode
}

// Codex tool calls use the `codex.*` namespace and are rendered via a
// dedicated dispatch that knows how to pretty-print shell output and
// file diffs. Unknown codex.* names fall through to the generic renderer.
function CodexToolCallRenderer({ content, toolResult, expanded, onToggle }: ToolCallProps) {
  return (
    <CodexToolCall
      content={content}
      toolResult={toolResult}
      expanded={expanded}
      onToggle={onToggle}
    />
  )
}

const alwaysRenderable = () => true

const codexRenderer: SpecialToolRenderer = {
  canRender: alwaysRenderable,
  Component: CodexToolCallRenderer,
}

/**
 * Registry of tool names with a dedicated renderer. Anything not listed here
 * (or whose input fails the renderer's `canRender` guard) is rendered by
 * `GenericToolCall`.
 */
const specialToolRenderers: Record<string, SpecialToolRenderer> = {
  'codex.shell': codexRenderer,
  'codex.edit': codexRenderer,
  'codex.todo_list': codexRenderer,
  AskUserQuestion: { canRender: hasQuestionsInput, Component: AskUserQuestionToolCall },
  ExitPlanMode: { canRender: alwaysRenderable, Component: ExitPlanModeToolCall },
  TodoWrite: { canRender: hasTodosInput, Component: TodoWriteToolCall },
}

export function ToolCallComponent(props: ToolCallProps) {
  const toolName = props.content.toolName
  const renderer = toolName ? specialToolRenderers[toolName] : undefined
  if (renderer?.canRender(props.content)) {
    return <renderer.Component {...props} />
  }
  return <GenericToolCall {...props} />
}
