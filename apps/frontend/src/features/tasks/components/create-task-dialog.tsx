import type { ClaudeModel } from '@moatless/types'
import { DEFAULT_CLAUDE_MODEL } from '@moatless/types'
import { useState } from 'react'
import { Dialog, DialogContent } from '@/lib/components/ui/dialog'
import { ViewToggle } from '@/lib/components/view-toggle.tsx'
import { CreateCodeTask } from './create-task/create-code-task'
import { CreateVibeTask } from './create-task/create-vibe-task'

type ViewMode = 'code' | 'vibe'

interface CreateTaskDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  defaultRepositoryId?: string
  defaultMessage?: string
  defaultMode?: ViewMode
}

export function CreateTaskDialog({
  open,
  onOpenChange,
  defaultRepositoryId,
  defaultMessage,
  defaultMode = 'code',
}: CreateTaskDialogProps) {
  const [mode, setMode] = useState<ViewMode>(defaultMode)

  // State for CreateCodeTask
  const [model, setModel] = useState<ClaudeModel>(DEFAULT_CLAUDE_MODEL)
  const [agentMode, setAgentMode] = useState<'edit' | 'plan'>('edit')
  const [repositoryId, setRepositoryId] = useState<string>(defaultRepositoryId || '')

  // State for CreateVibeTask
  const [templateId, setTemplateId] = useState<string>('')

  const handleModeChange = (newMode: ViewMode) => {
    setMode(newMode)
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl md:max-w-3xl lg:max-w-4xl">
        <div className="space-y-4">
          {/* View Toggle */}
          <div className="flex justify-center">
            <ViewToggle mode={mode} onModeChange={handleModeChange} />
          </div>

          {/* Task Creation Components */}
          {mode === 'code' ? (
            <CreateCodeTask
              className="w-full"
              maxHeight="500px"
              defaultMessage={defaultMessage}
              model={model}
              onModelChange={setModel}
              agentMode={agentMode}
              onAgentModeChange={setAgentMode}
              repositoryId={repositoryId}
              onRepositoryChange={setRepositoryId}
              onSuccess={() => onOpenChange(false)}
            />
          ) : (
            <CreateVibeTask
              className="w-full"
              maxHeight="500px"
              defaultMessage={defaultMessage}
              model={model}
              onModelChange={setModel}
              templateId={templateId}
              onTemplateChange={setTemplateId}
              onSuccess={() => onOpenChange(false)}
            />
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}
