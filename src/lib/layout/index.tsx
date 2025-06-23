import { useRouterState } from '@tanstack/react-router';
import type { ReactNode } from 'react';

import { AppSidebar } from '@/features/navigation/components/app-sidebar';
import { ThemeProvider } from '@/lib/components/theme-provider';
import { SidebarProvider, SidebarTrigger } from '@/lib/components/ui/sidebar';

import { Header } from './components/header';

type LayoutProps = {
  children: ReactNode;
};

export const Layout = ({ children }: LayoutProps) => {
  const router = useRouterState();
  const pathname = router.location.pathname;

  // Hide header for task detail pages
  const hideHeader = pathname.startsWith('/tasks/');

  return (
    <ThemeProvider>
      <SidebarProvider>
        <div className="flex min-h-screen w-full bg-background text-foreground">
          <AppSidebar />
          <div className="flex flex-1 flex-col">
            {!hideHeader && (
              <header className="sticky top-0 z-10 flex h-14 items-center gap-4 border-b bg-background px-6">
                <SidebarTrigger />
                <Header />
              </header>
            )}
            <main className="flex-1 relative">{children}</main>
          </div>
        </div>
      </SidebarProvider>
    </ThemeProvider>
  );
};
