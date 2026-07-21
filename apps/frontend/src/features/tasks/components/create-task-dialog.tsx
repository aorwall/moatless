import { useState } from 'react'
import { useTaskPreferences } from '@/features/tasks/hooks/use-task-preferences'
import { Dialog, DialogContent } from '@/lib/components/ui/dialog'
import { type ClaudeModel, DEFAULT_CLAUDE_MODEL } from '@/lib/types/claude-models'
import { CreateCodeTask } from './create-task/create-code-task'

interface CreateTaskDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  defaultWorkspaceId?: string
  defaultMessage?: string
}

export function CreateTaskDialog({
  open,
  onOpenChange,
  defaultWorkspaceId,
  defaultMessage,
}: CreateTaskDialogProps) {
  const preferences = useTaskPreferences()
  const [model, setModel] = useState<ClaudeModel>(DEFAULT_CLAUDE_MODEL)
  const [agentMode, setAgentMode] = useState<'edit' | 'plan'>('edit')
  const [workspaceId, setWorkspaceId] = useState<string>(defaultWorkspaceId || '')

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl md:max-w-3xl lg:max-w-4xl">
        <div className="space-y-4">
          <CreateCodeTask
            className="w-full"
            maxHeight="500px"
            defaultMessage={defaultMessage}
            model={model}
            onModelChange={setModel}
            agentMode={agentMode}
            onAgentModeChange={setAgentMode}
            agentType={preferences.agentType}
            onAgentTypeChange={preferences.setAgentType}
            workspaceId={workspaceId}
            onWorkspaceChange={setWorkspaceId}
            onSuccess={() => onOpenChange(false)}
          />
        </div>
      </DialogContent>
    </Dialog>
  )
}
