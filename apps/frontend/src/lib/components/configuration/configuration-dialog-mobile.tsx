import { Search, Settings, XIcon } from 'lucide-react'
import { cn } from '@/lib/utils'
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '../ui/dialog'
import { Input } from '../ui/input'
import type { ConfigurationDialogProps } from './configuration-dialog-desktop'

export default function ConfigurationDialogMobile({
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
