import { createFileRoute } from '@tanstack/react-router'
import { useState } from 'react'

import { ViewToggle } from '@/components/view-toggle'
import { CreateCodeTask } from '@/features/tasks/components/create-task/create-code-task'
import { CreateVibeTask } from '@/features/tasks/components/create-task/create-vibe-task'

type ViewMode = 'code' | 'vibe'

export const Route = createFileRoute('/')({
  component: Dashboard,
})

function Dashboard() {
  const [mode, setMode] = useState<ViewMode>(() => {
    // Load last mode from localStorage
    if (typeof window !== 'undefined') {
      return (localStorage.getItem('lastViewMode') as ViewMode) || 'code'
    }
    return 'code'
  })

  // Save mode to localStorage when it changes
  const handleModeChange = (newMode: ViewMode) => {
    setMode(newMode)
    if (typeof window !== 'undefined') {
      localStorage.setItem('lastViewMode', newMode)
    }
  }

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
          />
        ) : (
          <CreateVibeTask
            showTitle={true}
            title="What should we vibe today?"
            className="w-full"
            maxHeight="300px"
          />
        )}
      </div>
    </div>
  )
}
