import { Link } from '@tanstack/react-router'
import { Bell, Package, Plug, Route as RouteIcon, Terminal } from 'lucide-react'
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
      { label: 'Connections', to: '/admin/integrations/connections', icon: Plug },
      { label: 'Triggers', to: '/admin/integrations/triggers', icon: Bell },
      { label: 'Routes', to: '/admin/integrations/routes', icon: RouteIcon },
    ],
  },
  {
    label: 'System',
    items: [
      { label: 'Marketplaces', to: '/admin/marketplaces', icon: Package },
      { label: 'Commands', to: '/admin/commands', icon: Terminal },
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
