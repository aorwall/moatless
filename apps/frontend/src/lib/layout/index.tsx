import type { ReactNode } from 'react'

import { AppSidebar } from '@/features/navigation/components/app-sidebar'
import { ThemeProvider } from '@/lib/components/theme-provider'
import { SidebarProvider } from '@/lib/components/ui/sidebar'

type LayoutProps = {
  children: ReactNode
}

export const Layout = ({ children }: LayoutProps) => {
  return (
    <ThemeProvider>
      <SidebarProvider>
        <div className="flex h-screen w-full bg-background text-foreground">
          <AppSidebar />
          <main className="flex-1 overflow-auto h-full">{children}</main>
        </div>
      </SidebarProvider>
    </ThemeProvider>
  )
}
