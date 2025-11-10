import { ChevronDown } from 'lucide-react'
import type React from 'react'
import { useState } from 'react'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from './ui/collapsible'

interface CollapsibleHelpSectionProps {
  title: string
  children: React.ReactNode
}

export function CollapsibleHelpSection({ title, children }: CollapsibleHelpSectionProps) {
  const [isOpen, setIsOpen] = useState(false)

  return (
    <Collapsible open={isOpen} onOpenChange={setIsOpen}>
      <CollapsibleTrigger className="flex items-center justify-between w-full text-left p-3 hover:bg-accent rounded-md text-sm font-medium">
        <span>{title}</span>
        <ChevronDown
          className={`h-4 w-4 transition-transform ${isOpen ? 'transform rotate-180' : ''}`}
        />
      </CollapsibleTrigger>
      <CollapsibleContent className="p-3 rounded-md bg-muted/50">{children}</CollapsibleContent>
    </Collapsible>
  )
}
