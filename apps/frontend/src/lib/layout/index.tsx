import type { ReactNode } from 'react'

import { ThemeProvider } from '@/lib/components/theme-provider'

type LayoutProps = {
  children: ReactNode
}

export const Layout = ({ children }: LayoutProps) => {
  return (
    <ThemeProvider>
      <div className="flex h-full w-full bg-background text-foreground">
        <main className="flex-1 overflow-hidden h-full">{children}</main>
      </div>
    </ThemeProvider>
  )
}
