import { Search } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { Input } from '@/lib/components/ui/input'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { CanvasSearchResult, getParentPath, type SearchMatch } from './canvas-search-result'

interface CanvasSearchProps {
  data: unknown | null
  externalSearchQuery?: string
}

/**
 * Format a path array into a readable string
 * Examples:
 * - ['props', 'style', 'color'] => 'props.style.color'
 * - ['children', 0, 'name'] => 'children[0].name'
 * - ['items', 2, 'tags', 0] => 'items[2].tags[0]'
 */
function formatPath(path: (string | number)[]): string {
  let result = ''
  for (let i = 0; i < path.length; i++) {
    const segment = path[i]
    if (typeof segment === 'number') {
      result += `[${segment}]`
    } else {
      result += i === 0 ? segment : `.${segment}`
    }
  }
  return result
}

/**
 * Recursively search JSON data for matches
 * Matches against field names and primitive values
 * Excludes "children" field from value matches but still traverses it
 *
 * @param obj - The object to search
 * @param query - The search query (case-insensitive)
 * @param currentPath - Current path in the object tree
 * @param rootObject - The root object (for extracting parent objects later)
 * @returns Array of search matches
 */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Recursive search requires this complexity
function searchJSON(
  obj: unknown,
  query: string,
  currentPath: (string | number)[] = [],
  rootObject?: unknown,
): SearchMatch[] {
  const results: SearchMatch[] = []
  const lowerQuery = query.toLowerCase()

  // Set root object on first call
  const root = rootObject ?? obj

  // Handle null/undefined
  if (obj === null || obj === undefined) {
    return results
  }

  // Handle primitives - convert to string and match
  if (typeof obj !== 'object') {
    const stringValue = String(obj).toLowerCase()
    if (stringValue.includes(lowerQuery)) {
      results.push({
        path: currentPath,
        fullPath: formatPath(currentPath),
        value: obj,
        matchedObject: obj,
        rootObject: root,
      })
    }
    return results
  }

  // Handle objects and arrays
  if (Array.isArray(obj)) {
    obj.forEach((item, index) => {
      results.push(...searchJSON(item, query, [...currentPath, index], root))
    })
  } else {
    for (const [key, value] of Object.entries(obj)) {
      // Special handling for "children" field
      if (key === 'children') {
        // Skip matching the "children" field itself, but traverse it
        if (Array.isArray(value)) {
          results.push(...searchJSON(value, query, [...currentPath, key], root))
        }
      } else {
        // Match against field name
        if (key.toLowerCase().includes(lowerQuery)) {
          results.push({
            path: [...currentPath, key],
            fullPath: formatPath([...currentPath, key]),
            value,
            matchedObject: obj,
            rootObject: root,
          })
        }
        // Recurse into nested structures
        results.push(...searchJSON(value, query, [...currentPath, key], root))
      }
    }
  }

  return results
}

/**
 * Canvas Search Component
 * Provides search interface for Internal Canvas data with recursive field search
 */
export function CanvasSearch({ data, externalSearchQuery }: CanvasSearchProps) {
  const [searchQuery, setSearchQuery] = useState('')
  const [debouncedQuery, setDebouncedQuery] = useState('')
  const lastExternalQueryRef = useRef<string>('')

  // Sync external search queries with local state
  // Only update when external query changes (not when user types)
  useEffect(() => {
    if (externalSearchQuery && externalSearchQuery !== lastExternalQueryRef.current) {
      lastExternalQueryRef.current = externalSearchQuery
      setSearchQuery(externalSearchQuery)
    }
  }, [externalSearchQuery])

  // Debounce search input (300ms delay)
  useEffect(() => {
    const timer = setTimeout(() => {
      setDebouncedQuery(searchQuery)
    }, 300)
    return () => clearTimeout(timer)
  }, [searchQuery])

  // Memoized search results (filtered to exclude "assetRef" paths and deduplicated by parent)
  const results = useMemo(() => {
    if (!(data && debouncedQuery.trim())) {
      return []
    }

    const allMatches = searchJSON(data, debouncedQuery.trim()).filter(
      (match) => !match.fullPath.includes('assetRef'),
    )

    // Deduplicate by parent path - only keep first match for each parent
    const seenParents = new Set<string>()
    const deduplicated: SearchMatch[] = []

    for (const match of allMatches) {
      const parentPath = getParentPath(match.path)
      const parentKey = formatPath(parentPath)

      if (!seenParents.has(parentKey)) {
        seenParents.add(parentKey)
        deduplicated.push(match)
      }
    }

    return deduplicated.slice(0, 100)
  }, [data, debouncedQuery])

  // Empty state: no data
  if (!data) {
    return (
      <div className="flex h-full items-center justify-center rounded-md border border-dashed p-8">
        <div className="text-center">
          <Search className="mx-auto mb-2 h-12 w-12 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">No Internal Canvas data to search</p>
        </div>
      </div>
    )
  }

  return (
    <div className="flex h-full flex-col">
      {/* Search header */}
      <div className="border-b bg-muted/30 px-4 py-3">
        <div className="relative">
          <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            type="text"
            placeholder="Search fields and values..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="pl-9"
          />
        </div>
        {debouncedQuery && (
          <div className="mt-2 text-xs text-muted-foreground">
            {results.length > 0 ? (
              <>
                {results.length} {results.length === 1 ? 'match' : 'matches'} found
                {results.length === 100 && ' (showing first 100)'}
              </>
            ) : (
              'No matches found'
            )}
          </div>
        )}
      </div>

      {/* Results area */}
      <div className="flex-1 overflow-hidden">
        {!debouncedQuery ? (
          <div className="flex h-full items-center justify-center p-8">
            <p className="text-sm text-muted-foreground">
              Enter a search term to find matching fields
            </p>
          </div>
        ) : results.length === 0 ? (
          <div className="flex h-full items-center justify-center p-8">
            <p className="text-sm text-muted-foreground">No matches found for "{debouncedQuery}"</p>
          </div>
        ) : (
          <ScrollArea className="h-full">
            <div className="p-4">
              {results.map((result) => (
                <CanvasSearchResult key={result.fullPath} result={result} />
              ))}
            </div>
          </ScrollArea>
        )}
      </div>
    </div>
  )
}
