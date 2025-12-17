import { Code } from 'lucide-react'
import { useState } from 'react'
import { JsonViewer } from '@/lib/components/json-viewer'
import { Card } from '@/lib/components/ui/card'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/lib/components/ui/collapsible'

export interface SearchMatch {
  path: (string | number)[] // ['children', 0, 'name']
  fullPath: string // 'children[0].name'
  value: unknown
  matchedObject: unknown
  rootObject: unknown // Root object to extract parent from
}

interface CanvasSearchResultProps {
  result: SearchMatch
}

/**
 * Truncate a value for display
 * Converts to string and limits to 100 characters
 */
function truncateValue(value: unknown): string {
  const str = typeof value === 'string' ? value : JSON.stringify(value)
  return str.length > 100 ? `${str.slice(0, 100)}...` : str
}

/**
 * Get the parent path up to the last "children" array element
 * For path like ['children', 120, 'children', 185, 'properties', 'userFacingVersion']
 * Returns ['children', 120, 'children', 185]
 */
export function getParentPath(path: (string | number)[]): (string | number)[] {
  // Find the last occurrence of a numeric index (which indicates a children array element)
  let lastChildIndex = -1
  for (let i = path.length - 1; i >= 0; i--) {
    if (typeof path[i] === 'number') {
      lastChildIndex = i
      break
    }
  }

  // If we found a child index, return path up to and including it
  if (lastChildIndex >= 0) {
    return path.slice(0, lastChildIndex + 1)
  }

  // Otherwise return the full path minus the last segment
  return path.slice(0, -1)
}

/**
 * Get the parent object from a path
 * For path like ['children', 120, 'children', 185, 'properties', 'userFacingVersion']
 * Returns the object at ['children', 120, 'children', 185]
 */
function getParentObject(rootObject: unknown, path: (string | number)[]): unknown {
  // If path is empty or has only one segment, return the root
  if (path.length <= 1) {
    return rootObject
  }

  // Get the parent path using the helper function
  const parentPath = getParentPath(path)
  let current = rootObject

  for (const segment of parentPath) {
    if (current === null || current === undefined) {
      return null
    }
    if (typeof current === 'object' && current !== null) {
      current = (current as Record<string | number, unknown>)[segment]
    } else {
      return null
    }
  }

  return current
}

/**
 * Canvas Search Result Component
 * Individual result card showing path, value preview, and expandable JSON
 */
export function CanvasSearchResult({ result }: CanvasSearchResultProps) {
  const [isOpen, setIsOpen] = useState(false)

  // Extract the parent object from the path
  const parentObject = getParentObject(result.rootObject, result.path)

  return (
    <Card className="mb-2">
      <Collapsible open={isOpen} onOpenChange={setIsOpen}>
        <CollapsibleTrigger className="w-full p-3 text-left hover:bg-muted/50">
          <div className="flex items-start gap-2">
            <Code className="mt-0.5 h-4 w-4 flex-shrink-0 text-muted-foreground" />
            <div className="flex-1 min-w-0">
              <div className="font-mono text-xs text-primary">{result.fullPath}</div>
              <div className="mt-1 text-xs text-muted-foreground truncate">
                Value: {truncateValue(result.value)}
              </div>
            </div>
          </div>
        </CollapsibleTrigger>
        <CollapsibleContent className="border-t p-3">
          <div className="text-xs text-muted-foreground mb-2">Parent Object:</div>
          <JsonViewer data={parentObject} maxHeight="600px" />
        </CollapsibleContent>
      </Collapsible>
    </Card>
  )
}
