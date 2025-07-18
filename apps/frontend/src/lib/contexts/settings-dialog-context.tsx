import { Code2, Github, Settings } from 'lucide-react'
import { createContext, type ReactNode, useContext, useState } from 'react'
import {
  ClaudeCodeConfigurationSection,
  GeneralConfigurationSection,
  GitHubConfigurationSection,
} from '@/features/settings'
import { ConfigurationDialog } from '@/lib/components/configuration-dialog-portal'

interface SettingsDialogContextType {
  showDialog: () => void
}

const SettingsDialogContext = createContext<SettingsDialogContextType | undefined>(undefined)

interface SettingsDialogProviderProps {
  children: ReactNode
}

export function SettingsDialogProvider({ children }: SettingsDialogProviderProps) {
  const [open, setOpen] = useState(false)
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
    {
      id: 'github',
      label: 'GitHub',
      icon: <Github className="h-4 w-4" />,
      component: <GitHubConfigurationSection />,
    },
  ]

  const showDialog = () => setOpen(true)

  return (
    <SettingsDialogContext.Provider value={{ showDialog }}>
      {children}
      <ConfigurationDialog
        open={open}
        onOpenChange={setOpen}
        title="Settings"
        description="Configure your application preferences"
        sections={sections}
        activeSection={activeSection}
        onSectionChange={setActiveSection}
      />
    </SettingsDialogContext.Provider>
  )
}

export function useSettingsDialog() {
  const context = useContext(SettingsDialogContext)
  if (context === undefined) {
    throw new Error('useSettingsDialog must be used within a SettingsDialogProvider')
  }
  return context
}
