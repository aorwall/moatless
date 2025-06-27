import { useState } from 'react'

export function useFileTreeState() {
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set())
  const [selectedPath, setSelectedPath] = useState<string | null>(null)
  const [searchQuery, setSearchQuery] = useState('')

  const toggleExpanded = (path: string) => {
    setExpandedPaths((prev) => {
      const next = new Set(prev)
      if (next.has(path)) {
        next.delete(path)
      } else {
        next.add(path)
      }
      return next
    })
  }

  const expandPath = (path: string) => {
    setExpandedPaths((prev) => new Set(prev).add(path))
  }

  const collapsePath = (path: string) => {
    setExpandedPaths((prev) => {
      const next = new Set(prev)
      next.delete(path)
      return next
    })
  }

  const clearExpanded = () => {
    setExpandedPaths(new Set())
  }

  return {
    expandedPaths,
    selectedPath,
    searchQuery,
    toggleExpanded,
    expandPath,
    collapsePath,
    clearExpanded,
    setSelectedPath,
    setSearchQuery,
  }
}
