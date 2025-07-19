import { Search, Settings, XIcon } from 'lucide-react'
import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '../ui/dialog'
import { Input } from '../ui/input'

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

export function ConfigurationDialog({
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
          'sm:max-w-[1200px] max-w-[95vw] w-[95vw] h-[85vh] max-h-[900px] p-0 gap-0 overflow-hidden',
          className,
        )}
        showCloseButton={false}
      >
        <div className="flex h-full">
          {/* Sidebar */}
          <div className="w-64 bg-muted/30 border-r flex flex-col">
            {/* Header */}
            <div className="p-6 border-b">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <div className="flex items-center justify-center w-8 h-8 bg-primary/10 rounded-md">
                    <Settings className="h-4 w-4 text-primary" />
                  </div>
                  <div>
                    <DialogTitle className="text-lg font-semibold">{title}</DialogTitle>
                    {description && (
                      <DialogDescription className="text-sm text-muted-foreground mt-1">
                        {description}
                      </DialogDescription>
                    )}
                  </div>
                </div>
                <button
                  onClick={() => onOpenChange(false)}
                  className="flex items-center justify-center w-8 h-8 rounded-md hover:bg-muted transition-colors"
                  type="button"
                >
                  <XIcon className="h-4 w-4" />
                  <span className="sr-only">Close</span>
                </button>
              </div>
            </div>

            {/* Add Search Box after header if searchable */}
            {searchable && (
              <div className="p-4 border-b">
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
            <nav className="flex-1 p-4">
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
                        <span className="font-medium">{section.label}</span>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </nav>

            {/* Optional sidebar footer */}
            {sidebarFooter && <div className="p-4 border-t">{sidebarFooter}</div>}
          </div>

          {/* Content Area */}
          <div className="flex-1 flex flex-col overflow-hidden">
            {/* Content Header */}
            <div className="p-6 border-b bg-background">
              <DialogHeader>
                <DialogTitle className="text-xl font-semibold">{currentSection?.label}</DialogTitle>
              </DialogHeader>
            </div>

            {/* Content Body */}
            <div className="flex-1 overflow-y-auto p-6">
              {currentSection?.component || (
                <div className="flex items-center justify-center h-32 text-muted-foreground">
                  Section not found
                </div>
              )}
            </div>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}

// Responsive variant for mobile
export function MobileConfigurationDialog({
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
}: ConfigurationDialogProps) {
  const currentSection = sections.find((section) => section.id === activeSection)

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className={cn('w-[95vw] h-[90vh] p-0 gap-0 overflow-hidden sm:hidden', className)}
        showCloseButton={false}
      >
        <div className="flex flex-col h-full">
          {/* Mobile Header */}
          <div className="p-4 border-b bg-background">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-3">
                <div className="flex items-center justify-center w-8 h-8 bg-primary/10 rounded-md">
                  <Settings className="h-4 w-4 text-primary" />
                </div>
                <div>
                  <DialogTitle className="text-lg font-semibold">{title}</DialogTitle>
                  {description && (
                    <DialogDescription className="text-sm text-muted-foreground">
                      {description}
                    </DialogDescription>
                  )}
                </div>
              </div>
              <button
                onClick={() => onOpenChange(false)}
                className="flex items-center justify-center w-8 h-8 rounded-md hover:bg-muted transition-colors"
                type="button"
              >
                <XIcon className="h-4 w-4" />
                <span className="sr-only">Close</span>
              </button>
            </div>
          </div>

          {/* Add Search Box for mobile if searchable */}
          {searchable && (
            <div className="p-4 border-b">
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

          {/* Mobile Section Tabs */}
          <div className="p-4 border-b bg-muted/30">
            {sections.length === 0 ? (
              <div className="text-center text-sm text-muted-foreground py-4">
                {searchValue ? 'No results found' : 'No sections available'}
              </div>
            ) : (
              <div className="flex gap-2 overflow-x-auto">
                {sections.map((section) => (
                  <button
                    key={section.id}
                    onClick={() => onSectionChange(section.id)}
                    className={cn(
                      'flex items-center gap-2 px-4 py-2 rounded-md whitespace-nowrap transition-colors',
                      activeSection === section.id
                        ? 'bg-primary text-primary-foreground'
                        : 'bg-background text-muted-foreground hover:text-foreground hover:bg-muted',
                    )}
                    type="button"
                  >
                    {section.icon && <div className="w-4 h-4">{section.icon}</div>}
                    <span className="text-sm font-medium">{section.label}</span>
                  </button>
                ))}
              </div>
            )}
          </div>

          {/* Mobile Content */}
          <div className="flex-1 overflow-y-auto p-4">
            {currentSection?.component || (
              <div className="flex items-center justify-center h-32 text-muted-foreground">
                Section not found
              </div>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}

// Responsive wrapper that switches between desktop and mobile versions
export function ResponsiveConfigurationDialog(props: ConfigurationDialogProps) {
  return (
    <>
      {/* Desktop */}
      <div className="hidden sm:block">
        <ConfigurationDialog {...props} />
      </div>
      {/* Mobile */}
      <div className="sm:hidden">
        <MobileConfigurationDialog {...props} />
      </div>
    </>
  )
}
