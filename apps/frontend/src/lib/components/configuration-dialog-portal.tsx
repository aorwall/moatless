import * as DialogPrimitive from '@radix-ui/react-dialog'
import { Settings, XIcon } from 'lucide-react'
import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'

export interface ConfigurationSection {
  id: string
  label: string
  icon?: ReactNode
  component: ReactNode
}

interface ConfigurationDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description?: string
  sections: ConfigurationSection[]
  activeSection: string
  onSectionChange: (sectionId: string) => void
  className?: string
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
}: ConfigurationDialogProps) {
  const currentSection = sections.find((section) => section.id === activeSection)

  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-black/50 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0" />
        <DialogPrimitive.Content
          className={cn(
            'fixed left-[50%] top-[50%] z-50 translate-x-[-50%] translate-y-[-50%]',
            'w-[95vw] max-w-[1200px] h-[85vh] max-h-[900px]',
            'bg-background rounded-lg shadow-lg overflow-hidden',
            'data-[state=open]:animate-in data-[state=closed]:animate-out',
            'data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0',
            'data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95',
            'data-[state=closed]:slide-out-to-left-1/2 data-[state=closed]:slide-out-to-top-[48%]',
            'data-[state=open]:slide-in-from-left-1/2 data-[state=open]:slide-in-from-top-[48%]',
            className,
          )}
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
                      <DialogPrimitive.Title className="text-lg font-semibold">
                        {title}
                      </DialogPrimitive.Title>
                      {description && (
                        <DialogPrimitive.Description className="text-sm text-muted-foreground mt-1">
                          {description}
                        </DialogPrimitive.Description>
                      )}
                    </div>
                  </div>
                  <DialogPrimitive.Close className="flex items-center justify-center w-8 h-8 rounded-md hover:bg-muted transition-colors">
                    <XIcon className="h-4 w-4" />
                    <span className="sr-only">Close</span>
                  </DialogPrimitive.Close>
                </div>
              </div>

              {/* Navigation */}
              <nav className="flex-1 p-4">
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
              </nav>
            </div>

            {/* Content Area */}
            <div className="flex-1 flex flex-col overflow-hidden">
              {/* Content Header */}
              <div className="p-6 border-b bg-background">
                <h2 className="text-xl font-semibold">{currentSection?.label}</h2>
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
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  )
}
