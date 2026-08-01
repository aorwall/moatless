import { Link } from '@tanstack/react-router'
import {
  AppWindow,
  Boxes,
  Building2,
  FolderGit2,
  KeyRound,
  Link2,
  Package,
  Plug,
  Repeat,
  Users,
} from 'lucide-react'
import type { ComponentType, SVGProps } from 'react'
import { Github } from '@/lib/components/ui/brand-icons'
import {
  Sidebar,
  SidebarContent,
  SidebarGroup,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from '@/lib/components/ui/sidebar'

type IconComponent = ComponentType<SVGProps<SVGSVGElement>>

interface AdminNavItem {
  label: string
  to: string
  icon: IconComponent
}

interface AdminNavGroup {
  label: string
  items: AdminNavItem[]
}

const ADMIN_NAV: AdminNavGroup[] = [
  {
    label: 'Integrations',
    items: [
      { label: 'Apps', to: '/admin/integrations/apps', icon: AppWindow },
      { label: 'GitHub', to: '/admin/integrations/github', icon: Github },
      { label: 'Connections', to: '/admin/integrations/connections', icon: Plug },
      { label: 'Task Connections', to: '/admin/task-connections', icon: Link2 },
    ],
  },
  {
    label: 'Automation',
    items: [{ label: 'Loops', to: '/admin/loops', icon: Repeat }],
  },
  {
    label: 'System',
    items: [
      { label: 'Repositories', to: '/admin/repositories', icon: FolderGit2 },
      { label: 'Workspaces', to: '/admin/workspaces', icon: Boxes },
      { label: 'Users', to: '/admin/users', icon: Users },
      { label: 'Teams', to: '/admin/teams', icon: Building2 },
      { label: 'Secrets', to: '/admin/secrets', icon: KeyRound },
      { label: 'Plugins', to: '/admin/plugins', icon: Package },
    ],
  },
]

export function AdminSidebar() {
  return (
    <Sidebar collapsible="none" className="border-r">
      <SidebarHeader>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton size="lg" asChild>
              <Link to="/">
                <span className="font-bold">Admin</span>
              </Link>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarHeader>

      <SidebarContent>
        {ADMIN_NAV.map((group) => (
          <SidebarGroup key={group.label}>
            <SidebarGroupLabel>{group.label}</SidebarGroupLabel>
            <SidebarMenu>
              {group.items.map((item) => {
                const Icon = item.icon
                return (
                  <SidebarMenuItem key={item.to}>
                    <SidebarMenuButton asChild>
                      <Link
                        to={item.to}
                        activeProps={{ 'data-active': 'true' }}
                        activeOptions={{ exact: false }}
                      >
                        <Icon className="h-4 w-4" />
                        <span>{item.label}</span>
                      </Link>
                    </SidebarMenuButton>
                  </SidebarMenuItem>
                )
              })}
            </SidebarMenu>
          </SidebarGroup>
        ))}
      </SidebarContent>
    </Sidebar>
  )
}
