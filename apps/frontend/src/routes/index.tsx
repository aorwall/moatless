import { createFileRoute } from '@tanstack/react-router'
import { useState } from 'react'
import { TopBar } from '@/features/navigation/components/top-bar'
import { RepositoriesConfigurationDialog } from '@/features/repositories'
import { SettingsDialog } from '@/features/settings'
import { TaskDashboard } from '@/features/tasks/components/dashboard'
import { useTaskPreferences } from '@/features/tasks/hooks/use-task-preferences'

type ViewMode = 'code' | 'vibe'

export const Route = createFileRoute('/')({
  component: Dashboard,
})

function Dashboard() {
  const [repositoriesOpen, setRepositoriesOpen] = useState(false)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const preferences = useTaskPreferences()
  const [viewMode, setViewMode] = useState<ViewMode>(preferences.viewMode)

  const handleViewModeChange = (mode: ViewMode) => {
    setViewMode(mode)
    preferences.setViewMode(mode) // Still persist to localStorage
  }

  return (
    <div className="flex flex-col h-full">
      <TopBar
        onRepositoriesClick={() => setRepositoriesOpen(true)}
        onSettingsClick={() => setSettingsOpen(true)}
        viewMode={viewMode}
        onViewModeChange={handleViewModeChange}
      />
      <div className="flex-1 overflow-auto">
        <TaskDashboard viewMode={viewMode} />
      </div>
      <RepositoriesConfigurationDialog open={repositoriesOpen} onOpenChange={setRepositoriesOpen} />
      <SettingsDialog open={settingsOpen} onOpenChange={setSettingsOpen} />
    </div>
  )
}
