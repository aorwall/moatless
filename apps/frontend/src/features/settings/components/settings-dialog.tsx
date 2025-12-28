import { Bot, Figma, Github, Gitlab, Globe, Key, Server, Settings, Terminal } from 'lucide-react'
import { useMemo, useState } from 'react'
import {
  ApiKeyConfigurationSection,
  ClaudeConfigurationSection,
  CommandConfigurationSection,
  FigmaConfigurationSection,
  GeneralConfigurationSection,
  GitHubConfigurationSection,
  GitLabConfigurationSection,
  McpConfigurationSection,
  PublishConfigurationSection,
} from '@/features/settings'
import { SidebarDialog } from '@/lib/components/ui/sidebar-dialog'
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
  const showClaudeSettings = useFeatureFlag('SHOW_CLAUDE_SETTINGS')
  const isGitLabEnabled = useFeatureFlag('GITLAB_ENABLED')

  const sections = useMemo(() => {
    const allSections = []

    // System settings sections - controlled by SHOW_SYSTEM_SETTINGS flag
    if (showSystemSettings) {
      allSections.push({
        id: 'general',
        label: 'General',
        icon: <Settings className="h-4 w-4" />,
      })
    }

    // Claude settings - controlled by SHOW_CLAUDE_SETTINGS flag
    if (showClaudeSettings) {
      allSections.push({
        id: 'claude',
        label: 'Claude',
        icon: <Bot className="h-4 w-4" />,
      })
    }

    // User-level settings - always visible
    allSections.push({
      id: 'github',
      label: 'GitHub',
      icon: <Github className="h-4 w-4" />,
    })

    // GitLab - controlled by GITLAB_ENABLED flag
    if (isGitLabEnabled) {
      allSections.push({
        id: 'gitlab',
        label: 'GitLab',
        icon: <Gitlab className="h-4 w-4" />,
      })
    }

    allSections.push(
      {
        id: 'figma',
        label: 'Figma',
        icon: <Figma className="h-4 w-4" />,
      },
      {
        id: 'api-keys',
        label: 'API Keys',
        icon: <Key className="h-4 w-4" />,
      },
      {
        id: 'commands',
        label: 'Commands',
        icon: <Terminal className="h-4 w-4" />,
      },
    )

    // MCP Servers - controlled by SHOW_SYSTEM_SETTINGS flag
    if (showSystemSettings) {
      allSections.push({
        id: 'mcp',
        label: 'MCP Servers',
        icon: <Server className="h-4 w-4" />,
      })
    }

    // Only include publish section if feature flag is enabled
    if (isNetlifyPublishEnabled) {
      allSections.push({
        id: 'publish',
        label: 'Publish',
        icon: <Globe className="h-4 w-4" />,
      })
    }

    return allSections
  }, [isNetlifyPublishEnabled, showSystemSettings, showClaudeSettings, isGitLabEnabled])

  return (
    <SidebarDialog open={open} onOpenChange={onOpenChange}>
      <SidebarDialog.Sidebar title="Settings" description="Configure your application preferences">
        <SidebarDialog.Nav>
          {sections.map(({ id, label, icon }) => (
            <SidebarDialog.NavItem
              key={id}
              active={activeSection === id}
              onClick={() => setActiveSection(id)}
              icon={icon}
            >
              {label}
            </SidebarDialog.NavItem>
          ))}
        </SidebarDialog.Nav>
      </SidebarDialog.Sidebar>

      <SidebarDialog.Content title={sections.find((s) => s.id === activeSection)?.label}>
        {activeSection === 'general' && <GeneralConfigurationSection key="general" />}
        {activeSection === 'claude' && <ClaudeConfigurationSection key="claude" />}
        {activeSection === 'github' && <GitHubConfigurationSection key="github" />}
        {activeSection === 'gitlab' && <GitLabConfigurationSection key="gitlab" />}
        {activeSection === 'figma' && <FigmaConfigurationSection key="figma" />}
        {activeSection === 'api-keys' && <ApiKeyConfigurationSection key="api-keys" />}
        {activeSection === 'mcp' && <McpConfigurationSection key="mcp" />}
        {activeSection === 'commands' && <CommandConfigurationSection key="commands" />}
        {activeSection === 'publish' && <PublishConfigurationSection key="publish" />}
      </SidebarDialog.Content>
    </SidebarDialog>
  )
}
