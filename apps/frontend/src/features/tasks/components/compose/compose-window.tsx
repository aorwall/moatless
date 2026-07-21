import { Minus, X } from 'lucide-react'
import { cn } from '@/lib/utils'
import { type ComposeWindowState, useComposeTask } from '../../contexts/compose-task-provider'
import { ComposeTaskForm } from './compose-task-form'

interface ComposeWindowProps {
  window: ComposeWindowState
}

/**
 * A single Gmail-style compose "tab" docked at the bottom of the screen. Sized
 * to match Gmail's compose window (~500 × 500).
 */
export function ComposeWindow({ window }: ComposeWindowProps) {
  const { close, toggleMinimize, update } = useComposeTask()
  const { id, minimized } = window

  return (
    <div
      className={cn(
        'flex flex-col rounded-t-lg border border-b-0 bg-background shadow-2xl',
        minimized ? 'w-auto' : 'w-full sm:w-[500px] sm:max-w-[calc(100vw-2rem)]',
      )}
    >
      <div className="flex items-center justify-between rounded-t-lg bg-muted pl-3 pr-2">
        <button
          type="button"
          onClick={() => toggleMinimize(id)}
          className="flex-1 py-2 text-left text-sm font-medium"
        >
          {window.name.trim() || 'New Task'}
        </button>
        <div className="flex items-center gap-1">
          <button
            type="button"
            title={minimized ? 'Expand' : 'Minimize'}
            className="inline-flex h-6 w-6 items-center justify-center rounded hover:bg-background/60"
            onClick={() => toggleMinimize(id)}
          >
            <Minus className="h-4 w-4" />
          </button>
          <button
            type="button"
            title="Discard"
            className="inline-flex h-6 w-6 items-center justify-center rounded hover:bg-background/60"
            onClick={() => close(id)}
          >
            <X className="h-4 w-4" />
          </button>
        </div>
      </div>

      {!minimized && (
        <div className="h-[calc(100dvh-3rem)] px-3 pb-3 pt-2 sm:h-[460px]">
          <ComposeTaskForm
            navigateOnSuccess={false}
            onSuccess={() => close(id)}
            name={window.name}
            onNameChange={(value) => update(id, { name: value })}
            message={window.message}
            onMessageChange={(value) => update(id, { message: value })}
            branch={window.branch}
            onBranchChange={(value) => update(id, { branch: value })}
            workspaceId={window.workspaceId}
            onWorkspaceChange={(value) => update(id, { workspaceId: value })}
            model={window.model}
            onModelChange={(value) => update(id, { model: value })}
            agentMode={window.agentMode}
            onAgentModeChange={(value) => update(id, { agentMode: value })}
            agentType={window.agentType}
            onAgentTypeChange={(value) => update(id, { agentType: value })}
            templateId={window.templateId}
            onTemplateChange={(value) => update(id, { templateId: value })}
          />
        </div>
      )}
    </div>
  )
}
