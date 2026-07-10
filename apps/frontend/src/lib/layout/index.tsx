import type { ReactNode } from 'react'

import { ComposeDock } from '@/features/tasks/components/compose/compose-dock'
import { ComposeTaskProvider } from '@/features/tasks/contexts/compose-task-provider'
import { ThemeProvider } from '@/lib/components/theme-provider'

type LayoutProps = {
  children: ReactNode
}

export const Layout = ({ children }: LayoutProps) => {
  return (
    <ThemeProvider>
      <ComposeTaskProvider>
        <div className="flex h-full w-full bg-background text-foreground">
          <main className="flex-1 overflow-hidden h-full">{children}</main>
        </div>
        <ComposeDock />
      </ComposeTaskProvider>
    </ThemeProvider>
  )
}
