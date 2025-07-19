import { Search, XIcon } from 'lucide-react'
import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '../ui/dialog'
import { Input } from '../ui/input'
import { ScrollArea } from '../ui/scroll-area'

export interface ConfigurationSection {
  id: string
  label: string
  icon?: ReactNode
  component: ReactNode
}

export interface ConfigurationDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description?: string
  sections: ConfigurationSection[]
  activeSection: string
  onSectionChange: (sectionId: string) => void
  className?: string
  // New search props
  searchable?: boolean
  searchPlaceholder?: string
  onSearchChange?: (query: string) => void
  searchValue?: string
  // Optional custom sidebar footer
  sidebarFooter?: ReactNode
}

export default function ConfigurationDialogDesktop({
  open,
  onOpenChange,
  title,
  description,
  sections,
  activeSection,
  onSectionChange,
  className,
  searchable,
  searchPlaceholder = 'Search...',
  onSearchChange,
  searchValue = '',
  sidebarFooter,
}: ConfigurationDialogProps) {
  const currentSection = sections.find((section) => section.id === activeSection)

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className={cn(
          'sm:max-w-[1200px] max-w-[95vw] w-[95vw] h-[85vh] max-h-[900px] p-0 gap-0',
          className,
        )}
        showCloseButton={false}
      >
        <div className="flex h-full overflow-hidden relative">
          {/* Close button in upper right corner */}
          <button
            onClick={() => onOpenChange(false)}
            className="absolute top-4 right-4 flex items-center justify-center w-8 h-8 rounded-md hover:bg-muted transition-colors z-10"
            type="button"
          >
            <XIcon className="h-4 w-4" />
            <span className="sr-only">Close</span>
          </button>

          {/* Sidebar */}
          <div className="w-64 bg-muted/30 border-r flex flex-col h-full">
            {/* Header */}
            <div className="p-6 border-b flex-shrink-0">
              <div className="flex items-center gap-3">
                <div>
                  <DialogTitle className="text-lg font-semibold">{title}</DialogTitle>
                  {description && (
                    <DialogDescription className="text-sm text-muted-foreground mt-1">
                      {description}
                    </DialogDescription>
                  )}
                </div>
              </div>
            </div>

            {/* Add Search Box after header if searchable */}
            {searchable && (
              <div className="p-4 border-b flex-shrink-0">
                <div className="relative">
                  <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
                  <Input
                    placeholder={searchPlaceholder}
                    value={searchValue}
                    onChange={(e) => onSearchChange?.(e.target.value)}
                    className="pl-9"
                  />
                </div>
              </div>
            )}

            {/* Navigation */}
            <ScrollArea className="flex-1 overflow-hidden">
              <nav className="p-4">
                {sections.length === 0 ? (
                  <div className="text-center text-sm text-muted-foreground py-4">
                    {searchValue ? 'No results found' : 'No sections available'}
                  </div>
                ) : (
                  <ul className="space-y-2">
                    {sections.map((section) => (
                      <li key={section.id}>
                        <button
                          onClick={() => onSectionChange(section.id)}
                          className={cn(
                            'w-full flex items-center gap-3 px-3 py-2.5 rounded-md text-left transition-colors',
                            'hover:bg-muted/50',
                            activeSection === section.id
                              ? 'bg-primary/10 text-primary border border-primary/20'
                              : 'text-muted-foreground hover:text-foreground',
                          )}
                          type="button"
                        >
                          {section.icon && (
                            <div className="flex items-center justify-center w-5 h-5">
                              {section.icon}
                            </div>
                          )}
                          <span className="text-sm font-medium">{section.label}</span>
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
              </nav>
            </ScrollArea>

            {/* Optional sidebar footer */}
            {sidebarFooter && <div className="p-4 border-t flex-shrink-0">{sidebarFooter}</div>}
          </div>

          {/* Content Area */}
          <div className="flex-1 flex flex-col h-full">
            {/* Content Header */}
            <div className="p-6 border-b bg-background flex-shrink-0">
              <DialogHeader>
                <DialogTitle className="text-xl font-semibold">{currentSection?.label}</DialogTitle>
              </DialogHeader>
            </div>

            {/* Content Body */}
            <ScrollArea className="flex-1 overflow-hidden">
              <div className="p-6">
                {currentSection?.component || (
                  <div className="flex items-center justify-center h-32 text-muted-foreground">
                    Section not found
                  </div>
                )}
              </div>
            </ScrollArea>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}
