import { createFileRoute } from '@tanstack/react-router'
import { Code2, Settings } from 'lucide-react'
import { useState } from 'react'
import {
  ClaudeCodeConfigurationSection,
  GeneralConfigurationSection,
  SystemStatusCard,
} from '@/features/settings'
import { ResponsiveConfigurationDialog } from '@/lib/components/configuration-dialog'
import { Button } from '@/lib/components/ui/button'

export const Route = createFileRoute('/settings')({
  component: SettingsPage,
})

function SettingsPage() {
  const [showDialog, setShowDialog] = useState(false)
  const [activeSection, setActiveSection] = useState('general')

  const sections = [
    {
      id: 'general',
      label: 'General',
      icon: <Settings className="h-4 w-4" />,
      component: <GeneralConfigurationSection />,
    },
    {
      id: 'claude-code',
      label: 'Claude Code',
      icon: <Code2 className="h-4 w-4" />,
      component: <ClaudeCodeConfigurationSection />,
    },
  ]

  return (
    <div className="flex min-h-screen flex-col">
      <div className="flex-1 space-y-6 p-6">
        <div className="mx-auto max-w-6xl">
          <div className="mb-6">
            <SystemStatusCard />
          </div>

          <div className="flex justify-center">
            <Button onClick={() => setShowDialog(true)} size="lg">
              <Settings className="mr-2 h-4 w-4" />
              Open Settings
            </Button>
          </div>

          <ResponsiveConfigurationDialog
            open={showDialog}
            onOpenChange={setShowDialog}
            title="Settings"
            description="Configure your application preferences"
            sections={sections}
            activeSection={activeSection}
            onSectionChange={setActiveSection}
          />
        </div>
      </div>
    </div>
  )
}
