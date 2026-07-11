import { Menu, Search, XIcon } from 'lucide-react'
import type { ReactNode } from 'react'
import { createContext, useContext, useEffect, useMemo, useState } from 'react'
import { cn } from '@/lib/utils'
import { Button } from './button'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from './dialog'
import { Input } from './input'
import { ScrollArea } from './scroll-area'
import { Sheet, SheetContent } from './sheet'

// Context for managing responsive state
interface SidebarDialogContextValue {
  isMobile: boolean
  mobileSidebarOpen: boolean
  setMobileSidebarOpen: (open: boolean) => void
}

const SidebarDialogContext = createContext<SidebarDialogContextValue>({
  isMobile: false,
  mobileSidebarOpen: false,
  setMobileSidebarOpen: () => {
    // Default no-op implementation
  },
})

// Hook to detect mobile viewport
function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(false)

  useEffect(() => {
    const media = window.matchMedia(query)
    setMatches(media.matches)

    const listener = (event: MediaQueryListEvent) => {
      setMatches(event.matches)
    }

    media.addEventListener('change', listener)
    return () => media.removeEventListener('change', listener)
  }, [query])

  return matches
}

// Root Component
interface SidebarDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  children: ReactNode
  className?: string
}

function SidebarDialogRoot({ open, onOpenChange, children, className }: SidebarDialogProps) {
  const isMobile = !useMediaQuery('(min-width: 640px)')
  const [mobileSidebarOpen, setMobileSidebarOpen] = useState(false)

  const contextValue = useMemo(
    () => ({ isMobile, mobileSidebarOpen, setMobileSidebarOpen }),
    [isMobile, mobileSidebarOpen],
  )

  return (
    <SidebarDialogContext value={contextValue}>
      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent
          className={cn(
            'sm:max-w-[1200px] sm:h-[85vh] sm:max-h-[900px]',
            'max-sm:h-screen max-sm:w-screen max-sm:max-w-none max-sm:rounded-none',
            'p-0 gap-0',
            className,
          )}
          showCloseButton={false}
        >
          <div className="flex h-full overflow-hidden relative">
            {/* Close button */}
            <button
              onClick={() => onOpenChange(false)}
              className="absolute top-4 right-4 flex items-center justify-center w-8 h-8 rounded-md hover:bg-muted transition-colors z-10"
              type="button"
            >
              <XIcon className="h-4 w-4" />
              <span className="sr-only">Close</span>
            </button>

            {children}
          </div>
        </DialogContent>
      </Dialog>
    </SidebarDialogContext>
  )
}

// Sidebar Component
interface SidebarProps {
  title: string
  description?: string
  children: ReactNode
}

function Sidebar({ title, description, children }: SidebarProps) {
  const { isMobile, mobileSidebarOpen, setMobileSidebarOpen } = useContext(SidebarDialogContext)

  // Sidebar content (shared between mobile Sheet and desktop sidebar)
  const sidebarContent = (
    <>
      <div className="p-6 border-b flex-shrink-0">
        <DialogTitle className="text-lg font-semibold">{title}</DialogTitle>
        {description && (
          <DialogDescription className="text-sm text-muted-foreground mt-1">
            {description}
          </DialogDescription>
        )}
      </div>
      {children}
    </>
  )

  if (isMobile) {
    // Mobile: render as Sheet overlay
    return (
      <Sheet open={mobileSidebarOpen} onOpenChange={setMobileSidebarOpen}>
        <SheetContent side="left" className="w-64 p-0 flex flex-col">
          {sidebarContent}
        </SheetContent>
      </Sheet>
    )
  }

  // Desktop: render as sidebar
  return <div className="w-64 bg-muted/30 border-r flex flex-col h-full">{sidebarContent}</div>
}

// Search Component
interface SearchProps {
  value: string
  onChange: (value: string) => void
  placeholder?: string
}

function SearchInput({ value, onChange, placeholder = 'Search...' }: SearchProps) {
  return (
    <div className="p-4 border-b flex-shrink-0">
      <div className="relative">
        <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
        <Input
          placeholder={placeholder}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          className="pl-9"
        />
      </div>
    </div>
  )
}

// Nav Component
interface NavProps {
  children: ReactNode
  emptyMessage?: string
}

function Nav({ children, emptyMessage = 'No items available' }: NavProps) {
  return (
    <ScrollArea className="flex-1 overflow-hidden">
      <nav className="p-4">
        {children ? (
          <ul className="space-y-2">{children}</ul>
        ) : (
          <div className="text-center text-sm text-muted-foreground py-4">{emptyMessage}</div>
        )}
      </nav>
    </ScrollArea>
  )
}

// NavItem Component
interface NavItemProps {
  active?: boolean
  onClick: () => void
  icon?: ReactNode
  children: ReactNode
}

function NavItem({ active, onClick, icon, children }: NavItemProps) {
  const { isMobile, setMobileSidebarOpen } = useContext(SidebarDialogContext)

  const handleClick = () => {
    onClick()
    // Close mobile sidebar when item is clicked
    if (isMobile) {
      setMobileSidebarOpen(false)
    }
  }

  return (
    <li>
      <button
        onClick={handleClick}
        className={cn(
          'w-full flex items-center gap-3 px-3 py-2.5 rounded-md text-left transition-colors',
          'hover:bg-muted/50',
          active
            ? 'bg-primary/10 text-primary border border-primary/20'
            : 'text-muted-foreground hover:text-foreground',
        )}
        type="button"
      >
        {icon && <div className="flex items-center justify-center w-5 h-5">{icon}</div>}
        <span className="text-sm font-medium">{children}</span>
      </button>
    </li>
  )
}

// Footer Component
interface FooterProps {
  children: ReactNode
}

function Footer({ children }: FooterProps) {
  const { isMobile } = useContext(SidebarDialogContext)

  if (isMobile) {
    return null // No footer on mobile
  }

  return <div className="p-4 border-t flex-shrink-0">{children}</div>
}

// Content Component
interface ContentProps {
  title?: string | undefined
  children: ReactNode
}

function Content({ title, children }: ContentProps) {
  const { isMobile, setMobileSidebarOpen } = useContext(SidebarDialogContext)

  return (
    <div className="flex-1 flex flex-col h-full">
      {/* Header with optional hamburger menu on mobile */}
      <div className="p-6 border-b bg-background flex-shrink-0">
        <DialogHeader>
          <div className="flex items-center gap-3">
            {/* Hamburger menu button (mobile only) */}
            {isMobile && (
              <Button
                variant="ghost"
                size="icon"
                onClick={() => setMobileSidebarOpen(true)}
                className="flex-shrink-0"
              >
                <Menu className="h-5 w-5" />
                <span className="sr-only">Open menu</span>
              </Button>
            )}
            {title && <DialogTitle className="text-xl font-semibold">{title}</DialogTitle>}
          </div>
        </DialogHeader>
      </div>

      {/* Content area */}
      <ScrollArea className="flex-1 overflow-hidden">
        <div className="p-6">{children}</div>
      </ScrollArea>
    </div>
  )
}

// Export compound component
export const SidebarDialog = Object.assign(SidebarDialogRoot, {
  Sidebar,
  Search: SearchInput,
  Nav,
  NavItem,
  Footer,
  Content,
})
