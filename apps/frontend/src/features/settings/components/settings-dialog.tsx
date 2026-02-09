import { Bot, Figma, Github, Gitlab, Key, Package, Terminal } from 'lucide-react'
import { useMemo, useState } from 'react'
import {
  ApiKeyConfigurationSection,
  ClaudeConfigurationSection,
  CommandConfigurationSection,
  FigmaConfigurationSection,
  GitHubConfigurationSection,
  GitLabConfigurationSection,
  MarketplaceConfigurationSection,
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
  defaultSection = 'github',
}: SettingsDialogProps) {
  const [activeSection, setActiveSection] = useState(defaultSection)
  const showClaudeSettings = useFeatureFlag('SHOW_CLAUDE_SETTINGS')
  const isGitLabEnabled = useFeatureFlag('GITLAB_ENABLED')

  const sections = useMemo(() => {
    const allSections = []

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

    // Marketplaces - admin only
    allSections.push({
      id: 'marketplaces',
      label: 'Marketplaces',
      icon: <Package className="h-4 w-4" />,
    })

    return allSections
  }, [showClaudeSettings, isGitLabEnabled])

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
        {activeSection === 'claude' && <ClaudeConfigurationSection key="claude" />}
        {activeSection === 'github' && <GitHubConfigurationSection key="github" />}
        {activeSection === 'gitlab' && <GitLabConfigurationSection key="gitlab" />}
        {activeSection === 'figma' && <FigmaConfigurationSection key="figma" />}
        {activeSection === 'api-keys' && <ApiKeyConfigurationSection key="api-keys" />}
        {activeSection === 'marketplaces' && <MarketplaceConfigurationSection key="marketplaces" />}
        {activeSection === 'commands' && <CommandConfigurationSection key="commands" />}
      </SidebarDialog.Content>
    </SidebarDialog>
  )
}
