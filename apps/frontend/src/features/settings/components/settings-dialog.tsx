import { Bot, Github, Globe, Server, Settings } from 'lucide-react'
import { useState } from 'react'
import {
  ClaudeConfigurationSection,
  GeneralConfigurationSection,
  GitHubConfigurationSection,
  McpConfigurationSection,
  PublishConfigurationSection,
} from '@/features/settings'
import { ResponsiveConfigurationDialog } from '@/lib/components/configuration'

interface SettingsDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  defaultSection?: string
}

export function SettingsDialog({
  open,
  onOpenChange,
  defaultSection = 'general',
}: SettingsDialogProps) {
  const [activeSection, setActiveSection] = useState(defaultSection)

  const sections = [
    {
      id: 'general',
      label: 'General',
      icon: <Settings className="h-4 w-4" />,
      component: <GeneralConfigurationSection />,
    },
    {
      id: 'claude',
      label: 'Claude',
      icon: <Bot className="h-4 w-4" />,
      component: <ClaudeConfigurationSection />,
    },
    {
      id: 'github',
      label: 'GitHub',
      icon: <Github className="h-4 w-4" />,
      component: <GitHubConfigurationSection />,
    },
    {
      id: 'mcp',
      label: 'MCP Servers',
      icon: <Server className="h-4 w-4" />,
      component: <McpConfigurationSection />,
    },
    {
      id: 'publish',
      label: 'Publish',
      icon: <Globe className="h-4 w-4" />,
      component: <PublishConfigurationSection />,
    },
  ]

  return (
    <ResponsiveConfigurationDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Settings"
      description="Configure your application preferences"
      sections={sections}
      activeSection={activeSection}
      onSectionChange={setActiveSection}
    />
  )
}
