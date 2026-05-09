import { Bot, Figma, GitBranch, Github, Gitlab, Key, Sparkles, User } from 'lucide-react'
import { useMemo, useState } from 'react'
import {
  ApiKeyConfigurationSection,
  ClaudeConfigurationSection,
  CodexConfigurationSection,
  FigmaConfigurationSection,
  GitHubConfigurationSection,
  GitLabConfigurationSection,
  GitnessConfigurationSection,
  ProfileConfigurationSection,
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
  defaultSection = 'profile',
}: SettingsDialogProps) {
  const [activeSection, setActiveSection] = useState(defaultSection)
  const showClaudeSettings = useFeatureFlag('SHOW_CLAUDE_SETTINGS')
  const showCodexSettings = useFeatureFlag('SHOW_CODEX_SETTINGS')
  const isGitLabEnabled = useFeatureFlag('GITLAB_ENABLED')
  const isGitnessEnabled = useFeatureFlag('GITNESS_ENABLED')

  const sections = useMemo(() => {
    const allSections = []

    allSections.push({
      id: 'profile',
      label: 'Profile',
      icon: <User className="h-4 w-4" />,
    })

    if (showClaudeSettings) {
      allSections.push({
        id: 'claude',
        label: 'Claude',
        icon: <Bot className="h-4 w-4" />,
      })
    }

    if (showCodexSettings) {
      allSections.push({
        id: 'codex',
        label: 'Codex',
        icon: <Sparkles className="h-4 w-4" />,
      })
    }

    allSections.push({
      id: 'github',
      label: 'GitHub',
      icon: <Github className="h-4 w-4" />,
    })

    if (isGitLabEnabled) {
      allSections.push({
        id: 'gitlab',
        label: 'GitLab',
        icon: <Gitlab className="h-4 w-4" />,
      })
    }

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
    )

    return allSections
  }, [showClaudeSettings, showCodexSettings, isGitLabEnabled, isGitnessEnabled])

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
        {activeSection === 'codex' && <CodexConfigurationSection key="codex" />}
        {activeSection === 'github' && <GitHubConfigurationSection key="github" />}
        {activeSection === 'gitlab' && <GitLabConfigurationSection key="gitlab" />}
        {activeSection === 'gitness' && <GitnessConfigurationSection key="gitness" />}
        {activeSection === 'figma' && <FigmaConfigurationSection key="figma" />}
        {activeSection === 'api-keys' && <ApiKeyConfigurationSection key="api-keys" />}
      </SidebarDialog.Content>
    </SidebarDialog>
  )
}
