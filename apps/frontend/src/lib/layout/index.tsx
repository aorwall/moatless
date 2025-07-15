import { useRouterState } from '@tanstack/react-router'
import type { ReactNode } from 'react'

import { AppSidebar } from '@/features/navigation/components/app-sidebar'
import { ThemeProvider } from '@/lib/components/theme-provider'
import { SidebarProvider } from '@/lib/components/ui/sidebar'

type LayoutProps = {
  children: ReactNode
}

export const Layout = ({ children }: LayoutProps) => {
  const router = useRouterState()
  const pathname = router.location.pathname

  // Hide header for task detail pages
  const hideHeader = pathname.startsWith('/tasks/')

  return (
    <ThemeProvider>
      <SidebarProvider>
        <div className="flex h-screen w-full bg-background text-foreground">
          <AppSidebar />
          <main className="flex-1 overflow-hidden h-full">{children}</main>
        </div>
      </SidebarProvider>
    </ThemeProvider>
  )
}
