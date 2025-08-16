import type { ClaudeModel } from '@moatless/types'
import { DEFAULT_CLAUDE_MODEL } from '@moatless/types'
import { createFileRoute } from '@tanstack/react-router'
import { useCallback, useState } from 'react'

import { ViewToggle } from '@/components/view-toggle'
import { CreateCodeTask } from '@/features/tasks/components/create-task/create-code-task'
import { CreateVibeTask } from '@/features/tasks/components/create-task/create-vibe-task'

type ViewMode = 'code' | 'vibe'

export const Route = createFileRoute('/')({
  component: Dashboard,
})

function Dashboard() {
  // View mode state with localStorage persistence
  const [mode, setMode] = useState<ViewMode>(() => {
    if (typeof window !== 'undefined') {
      return (localStorage.getItem('lastViewMode') as ViewMode) || 'code'
    }
    return 'code'
  })

  // Shared model state with localStorage persistence
  const [model, setModel] = useState<ClaudeModel>(() => {
    if (typeof window !== 'undefined') {
      const savedModel = localStorage.getItem('lastSelectedModel') as ClaudeModel
      return savedModel || DEFAULT_CLAUDE_MODEL
    }
    return DEFAULT_CLAUDE_MODEL
  })

  // Agent mode state (for code tasks) with localStorage persistence
  const [agentMode, setAgentMode] = useState<'edit' | 'plan'>(() => {
    if (typeof window !== 'undefined') {
      const savedMode = localStorage.getItem('lastSelectedAgentMode') as 'edit' | 'plan'
      return savedMode || 'edit'
    }
    return 'edit'
  })

  // Repository state (for code tasks) with localStorage persistence
  const [repositoryId, setRepositoryId] = useState<string>(() => {
    if (typeof window !== 'undefined') {
      return localStorage.getItem('lastSelectedRepositoryId') || ''
    }
    return ''
  })

  // Template state (for vibe tasks) with localStorage persistence
  const [templateId, setTemplateId] = useState<string>(() => {
    if (typeof window !== 'undefined') {
      return localStorage.getItem('lastSelectedTemplateId') || ''
    }
    return ''
  })

  // Handle view mode change with localStorage persistence
  const handleModeChange = useCallback((newMode: ViewMode) => {
    setMode(newMode)
    if (typeof window !== 'undefined') {
      localStorage.setItem('lastViewMode', newMode)
    }
  }, [])

  // Handle model change with localStorage persistence
  const handleModelChange = useCallback((newModel: ClaudeModel) => {
    setModel(newModel)
    if (typeof window !== 'undefined') {
      localStorage.setItem('lastSelectedModel', newModel)
    }
  }, [])

  // Handle agent mode change with localStorage persistence
  const handleAgentModeChange = useCallback((newMode: 'edit' | 'plan') => {
    setAgentMode(newMode)
    if (typeof window !== 'undefined') {
      localStorage.setItem('lastSelectedAgentMode', newMode)
    }
  }, [])

  // Handle repository change with localStorage persistence
  const handleRepositoryChange = useCallback((newRepoId: string) => {
    setRepositoryId(newRepoId)
    if (typeof window !== 'undefined') {
      localStorage.setItem('lastSelectedRepositoryId', newRepoId)
    }
  }, [])

  // Handle template change with localStorage persistence
  const handleTemplateChange = useCallback((newTemplateId: string) => {
    setTemplateId(newTemplateId)
    if (typeof window !== 'undefined') {
      localStorage.setItem('lastSelectedTemplateId', newTemplateId)
    }
  }, [])

  return (
    <div className="flex h-full items-center justify-center">
      <div className="w-full max-w-3xl px-6">
        {/* View Toggle */}
        <div className="flex justify-center mb-8">
          <ViewToggle mode={mode} onModeChange={handleModeChange} />
        </div>

        {/* Task Creation Components */}
        {mode === 'code' ? (
          <CreateCodeTask
            showTitle={true}
            title="What should we build today?"
            className="w-full"
            maxHeight="300px"
            model={model}
            onModelChange={handleModelChange}
            agentMode={agentMode}
            onAgentModeChange={handleAgentModeChange}
            repositoryId={repositoryId}
            onRepositoryChange={handleRepositoryChange}
          />
        ) : (
          <CreateVibeTask
            showTitle={true}
            title="What should we vibe today?"
            className="w-full"
            maxHeight="300px"
            model={model}
            onModelChange={handleModelChange}
            templateId={templateId}
            onTemplateChange={handleTemplateChange}
          />
        )}
      </div>
    </div>
  )
}
