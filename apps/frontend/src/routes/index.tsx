import { createFileRoute } from '@tanstack/react-router'
import { useState } from 'react'
import { TopBar } from '@/features/navigation/components/top-bar'
import { RepositoriesConfigurationDialog } from '@/features/repositories'
import { SettingsDialog } from '@/features/settings'
import { TaskDashboard } from '@/features/tasks/components/dashboard'

export const Route = createFileRoute('/')({
  component: Dashboard,
})

function Dashboard() {
  const [repositoriesOpen, setRepositoriesOpen] = useState(false)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const preferences = useTaskPreferences()

  return (
    <div className="flex flex-col h-full">
      <TopBar
        onRepositoriesClick={() => setRepositoriesOpen(true)}
        onSettingsClick={() => setSettingsOpen(true)}
        viewMode={preferences.viewMode}
        onViewModeChange={preferences.setViewMode}
      />
      <div className="flex-1 overflow-auto">
        <TaskDashboard />
      </div>
      <RepositoriesConfigurationDialog open={repositoriesOpen} onOpenChange={setRepositoriesOpen} />
      <SettingsDialog open={settingsOpen} onOpenChange={setSettingsOpen} />
    </div>
  )
}
