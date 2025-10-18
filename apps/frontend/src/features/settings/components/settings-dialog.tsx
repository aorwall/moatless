import { Bot, Figma, Github, Globe, Server, Settings } from 'lucide-react'
import { useMemo, useState } from 'react'
import {
  ClaudeConfigurationSection,
  FigmaConfigurationSection,
  GeneralConfigurationSection,
  GitHubConfigurationSection,
  McpConfigurationSection,
  PublishConfigurationSection,
} from '@/features/settings'
import { ResponsiveConfigurationDialog } from '@/lib/components/configuration'
import { useFeatureFlag } from '@/lib/hooks/use-feature-flag'

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
  const isNetlifyPublishEnabled = useFeatureFlag('NETLIFY_PUBLISH')
  const showSystemSettings = useFeatureFlag('SHOW_SYSTEM_SETTINGS')

  const sections = useMemo(() => {
    const allSections = []

    // System settings sections - controlled by SHOW_SYSTEM_SETTINGS flag
    if (showSystemSettings) {
      allSections.push(
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
      )
    }

    // User-level settings - always visible
    allSections.push(
      {
        id: 'github',
        label: 'GitHub',
        icon: <Github className="h-4 w-4" />,
        component: <GitHubConfigurationSection />,
      },
      {
        id: 'figma',
        label: 'Figma',
        icon: <Figma className="h-4 w-4" />,
        component: <FigmaConfigurationSection />,
      },
    )

    // MCP Servers - controlled by SHOW_SYSTEM_SETTINGS flag
    if (showSystemSettings) {
      allSections.push({
        id: 'mcp',
        label: 'MCP Servers',
        icon: <Server className="h-4 w-4" />,
        component: <McpConfigurationSection />,
      })
    }

    // Only include publish section if feature flag is enabled
    if (isNetlifyPublishEnabled) {
      allSections.push({
        id: 'publish',
        label: 'Publish',
        icon: <Globe className="h-4 w-4" />,
        component: <PublishConfigurationSection />,
      })
    }

    return allSections
  }, [isNetlifyPublishEnabled, showSystemSettings])

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
