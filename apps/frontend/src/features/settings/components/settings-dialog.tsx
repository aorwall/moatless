import { Bot, GitBranch, Key, Lock, Smartphone, Sparkles, User } from 'lucide-react'
import { useState } from 'react'
import { useAuthMode } from '@/features/auth/hooks/use-auth-mode'
import { Github, Gitlab } from '@/lib/components/ui/brand-icons'
import { SidebarDialog } from '@/lib/components/ui/sidebar-dialog'
import { useFeatureFlag } from '@/lib/hooks/use-feature-flag'
import { ApiKeyConfigurationSection } from './api-key-configuration-section'
import { ClaudeConfigurationSection } from './claude-configuration-section'
import { CodexConfigurationSection } from './codex-configuration-section'
import { DevicePairingSection } from './device-pairing-section'
import { ForgejoConfigurationSection } from './forgejo-configuration-section'
import { GitHubConfigurationSection } from './github-configuration-section'
import { GitLabConfigurationSection } from './gitlab-configuration-section'
import { GitnessConfigurationSection } from './gitness-configuration-section'
import { PasswordConfigurationSection } from './password-configuration-section'
import { ProfileConfigurationSection } from './profile-configuration-section'

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
  const isForgejoEnabled = useFeatureFlag('FORGEJO_ENABLED')
  // On exactly when the T3 compatibility routes are: a pairing code is worth
  // nothing to a client that has nothing to connect to afterwards.
  const isDevicePairingEnabled = useFeatureFlag('DEVICE_PAIRING')
  const { isPasswordMode } = useAuthMode()

  const sections = (() => {
    const allSections = []

    allSections.push({
      id: 'profile',
      label: 'Profile',
      icon: <User className="h-4 w-4" />,
    })

    // Only meaningful under local auth; OAuth deployments have no password.
    if (isPasswordMode) {
      allSections.push({
        id: 'password',
        label: 'Password',
        icon: <Lock className="h-4 w-4" />,
      })
    }

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

    if (isForgejoEnabled) {
      allSections.push({
        id: 'forgejo',
        label: 'Forgejo',
        icon: <GitBranch className="h-4 w-4" />,
      })
    }

    allSections.push({
      id: 'api-keys',
      label: 'API Keys',
      icon: <Key className="h-4 w-4" />,
    })

    if (isDevicePairingEnabled) {
      allSections.push({
        id: 'devices',
        label: 'Devices',
        icon: <Smartphone className="h-4 w-4" />,
      })
    }

    return allSections
  })()

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
        {activeSection === 'password' && <PasswordConfigurationSection key="password" />}
        {activeSection === 'claude' && <ClaudeConfigurationSection key="claude" />}
        {activeSection === 'codex' && <CodexConfigurationSection key="codex" />}
        {activeSection === 'github' && <GitHubConfigurationSection key="github" />}
        {activeSection === 'gitlab' && <GitLabConfigurationSection key="gitlab" />}
        {activeSection === 'gitness' && <GitnessConfigurationSection key="gitness" />}
        {activeSection === 'forgejo' && <ForgejoConfigurationSection key="forgejo" />}
        {activeSection === 'api-keys' && <ApiKeyConfigurationSection key="api-keys" />}
        {activeSection === 'devices' && <DevicePairingSection key="devices" />}
      </SidebarDialog.Content>
    </SidebarDialog>
  )
}
