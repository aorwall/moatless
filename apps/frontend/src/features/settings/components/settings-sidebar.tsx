import { Link } from '@tanstack/react-router'
import { Bell, Plug, Route as RouteIcon } from 'lucide-react'
import type { ComponentType, SVGProps } from 'react'
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

interface SettingsNavItem {
  label: string
  to: string
  icon: IconComponent
}

interface SettingsNavGroup {
  label: string
  items: SettingsNavItem[]
}

const SETTINGS_NAV: SettingsNavGroup[] = [
  {
    label: 'Integrations',
    items: [
      { label: 'Connections', to: '/settings/integrations/connections', icon: Plug },
      { label: 'Triggers', to: '/settings/integrations/triggers', icon: Bell },
      { label: 'Routes', to: '/settings/integrations/routes', icon: RouteIcon },
    ],
  },
]

export function SettingsSidebar() {
  return (
    <Sidebar collapsible="none" className="border-r">
      <SidebarHeader>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton size="lg" asChild>
              <Link to="/">
                <span className="font-bold">Settings</span>
              </Link>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarHeader>

      <SidebarContent>
        {SETTINGS_NAV.map((group) => (
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
