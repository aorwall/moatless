import {
  Bot,
  Figma,
  GitBranch,
  Github,
  Gitlab,
  Key,
  MessageSquare,
  Package,
  Terminal,
  User,
} from 'lucide-react'
import { useMemo, useState } from 'react'
import {
  ApiKeyConfigurationSection,
  ChannelConfigurationSection,
  ClaudeConfigurationSection,
  CommandConfigurationSection,
  FigmaConfigurationSection,
  GitHubConfigurationSection,
  GitLabConfigurationSection,
  GitnessConfigurationSection,
  MarketplaceConfigurationSection,
  ProfileConfigurationSection,
} from '@/features/settings'
import { useChannelProviders } from '@/features/settings/hooks/use-channels'
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
  defaultSection = 'profile',
}: SettingsDialogProps) {
  const [activeSection, setActiveSection] = useState(defaultSection)
  const showClaudeSettings = useFeatureFlag('SHOW_CLAUDE_SETTINGS')
  const isGitLabEnabled = useFeatureFlag('GITLAB_ENABLED')
  const isGitnessEnabled = useFeatureFlag('GITNESS_ENABLED')
  const { data: channelProviders } = useChannelProviders()
  const hasChannelProviders = (channelProviders?.length ?? 0) > 0

  const sections = useMemo(() => {
    const allSections = []

    // Profile - always visible, placed first
    allSections.push({
      id: 'profile',
      label: 'Profile',
      icon: <User className="h-4 w-4" />,
    })

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

    // Gitness - controlled by GITNESS_ENABLED flag
    if (isGitnessEnabled) {
      allSections.push({
        id: 'gitness',
        label: 'Gitness',
        icon: <GitBranch className="h-4 w-4" />,
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

    // Channels - shown when at least one provider is configured
    if (hasChannelProviders) {
      allSections.push({
        id: 'channels',
        label: 'Channels',
        icon: <MessageSquare className="h-4 w-4" />,
      })
    }

    // Marketplaces - admin only
    allSections.push({
      id: 'marketplaces',
      label: 'Marketplaces',
      icon: <Package className="h-4 w-4" />,
    })

    return allSections
  }, [showClaudeSettings, isGitLabEnabled, isGitnessEnabled, hasChannelProviders])

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
        {activeSection === 'profile' && <ProfileConfigurationSection key="profile" />}
        {activeSection === 'claude' && <ClaudeConfigurationSection key="claude" />}
        {activeSection === 'github' && <GitHubConfigurationSection key="github" />}
        {activeSection === 'gitlab' && <GitLabConfigurationSection key="gitlab" />}
        {activeSection === 'gitness' && <GitnessConfigurationSection key="gitness" />}
        {activeSection === 'figma' && <FigmaConfigurationSection key="figma" />}
        {activeSection === 'api-keys' && <ApiKeyConfigurationSection key="api-keys" />}
        {activeSection === 'marketplaces' && <MarketplaceConfigurationSection key="marketplaces" />}
        {activeSection === 'commands' && <CommandConfigurationSection key="commands" />}
        {activeSection === 'channels' && <ChannelConfigurationSection key="channels" />}
      </SidebarDialog.Content>
    </SidebarDialog>
  )
}
