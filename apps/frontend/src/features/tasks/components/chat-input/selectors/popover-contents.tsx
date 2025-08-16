import { Check, Search } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { cn } from '@/lib/utils'

export interface SelectableItem {
  id: string
  label: string
  description?: string
}

interface PopoverContentProps {
  items: SelectableItem[]
  value?: string
  onChange: (value: string) => void
  onSelect?: () => void
}

export interface SearchablePopoverContentProps extends PopoverContentProps {
  placeholder?: string
  emptyText?: string
}

export function SearchablePopoverContent({
  items,
  value,
  onChange,
  onSelect,
  placeholder = 'Search...',
  emptyText = 'No items found',
}: SearchablePopoverContentProps) {
  const [search, setSearch] = useState('')

  const filteredItems = items.filter((item) =>
    item.label.toLowerCase().includes(search.toLowerCase()),
  )

  const handleSelect = (itemValue: string) => {
    onChange(itemValue)
    onSelect?.()
  }

  return (
    <div className="flex flex-col">
      {/* Search input */}
      <div className="p-3 border-b">
        <div className="relative">
          <Search className="absolute left-3 top-1/2 transform -translate-y-1/2 h-4 w-4 text-muted-foreground" />
          <Input
            placeholder={placeholder}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="pl-10 h-8"
            autoFocus
          />
        </div>
      </div>

      {/* Items list */}
      <ScrollArea className="max-h-60 overflow-auto">
        <div className="p-1">
          {filteredItems.length === 0 ? (
            <div className="px-3 py-2 text-sm text-muted-foreground text-center">{emptyText}</div>
          ) : (
            filteredItems.map((item) => (
              <Button
                key={item.id}
                variant="ghost"
                className={cn(
                  'w-full justify-between h-auto p-3 text-left overflow-hidden',
                  value === item.id && 'bg-accent',
                )}
                onClick={() => handleSelect(item.id)}
              >
                <div className="flex flex-col items-start flex-1 min-w-0">
                  <span className="font-medium truncate w-full">{item.label}</span>
                  {item.description && (
                    <span className="text-xs text-muted-foreground truncate w-full">
                      {item.description}
                    </span>
                  )}
                </div>
                {value === item.id && <Check className="h-4 w-4 shrink-0" />}
              </Button>
            ))
          )}
        </div>
      </ScrollArea>
    </div>
  )
}

export function SimpleListPopoverContent({
  items,
  value,
  onChange,
  onSelect,
}: PopoverContentProps) {
  const handleSelect = (itemValue: string) => {
    onChange(itemValue)
    onSelect?.()
  }

  return (
    <div className="p-1 flex flex-col w-fit">
      {items.map((item) => (
        <Button
          key={item.id}
          variant="ghost"
          className={cn(
            'w-full justify-between h-auto px-3 py-2 text-left overflow-hidden',
            value === item.id && 'bg-accent',
          )}
          onClick={() => handleSelect(item.id)}
        >
          <div className="flex items-center gap-2">
            <span className="font-medium whitespace-nowrap">{item.label}</span>
            {value === item.id && <Check className="h-4 w-4 shrink-0" />}
          </div>
        </Button>
      ))}
    </div>
  )
}

export interface BranchPopoverContentProps extends PopoverContentProps {
  currentBranch?: string
  enableSearch?: boolean
  placeholder?: string
}

export function BranchPopoverContent({
  items,
  value,
  onChange,
  onSelect,
  currentBranch,
  enableSearch = false,
  placeholder = 'Search branches...',
}: BranchPopoverContentProps) {
  const [search, setSearch] = useState('')

  const filteredItems = enableSearch
    ? items.filter((item) => item.label.toLowerCase().includes(search.toLowerCase()))
    : items

  const handleSelect = (itemValue: string) => {
    onChange(itemValue)
    onSelect?.()
  }

  return (
    <div className="flex flex-col">
      {/* Search input (if enabled) */}
      {enableSearch && (
        <div className="p-3 border-b">
          <div className="relative">
            <Search className="absolute left-3 top-1/2 transform -translate-y-1/2 h-4 w-4 text-muted-foreground" />
            <Input
              placeholder={placeholder}
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              className="pl-10 h-8"
              autoFocus
            />
          </div>
        </div>
      )}

      {/* Branches list */}
      <ScrollArea className="max-h-60 overflow-auto">
        <div className="p-1">
          {filteredItems.length === 0 ? (
            <div className="px-3 py-2 text-sm text-muted-foreground text-center">
              No branches found
            </div>
          ) : (
            filteredItems.map((item) => {
              const isCurrent = currentBranch === item.id
              return (
                <Button
                  key={item.id}
                  variant="ghost"
                  className={cn(
                    'w-full justify-between h-auto p-3 text-left overflow-hidden',
                    value === item.id && 'bg-accent',
                  )}
                  onClick={() => handleSelect(item.id)}
                >
                  <div className="flex flex-col items-start flex-1 min-w-0">
                    <div className="flex items-center gap-2 w-full">
                      <span className="font-medium truncate flex-1">{item.label}</span>
                      {isCurrent && (
                        <span className="text-xs text-status-success shrink-0">(current)</span>
                      )}
                    </div>
                    {item.description && (
                      <span className="text-xs text-muted-foreground truncate w-full">
                        {item.description}
                      </span>
                    )}
                  </div>
                  {value === item.id && <Check className="h-4 w-4 shrink-0" />}
                </Button>
              )
            })
          )}
        </div>
      </ScrollArea>
    </div>
  )
}
