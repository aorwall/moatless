import { useMutation } from '@tanstack/react-query'

interface FileSearchResult {
  files: Array<{ path: string; matches: number }>
  totalMatches: number
}

export function useFileSearch() {
  return useMutation<FileSearchResult, Error, { query: string; caseSensitive?: boolean; maxResults?: number }>({
    mutationFn: async (params) => {
      const res = await fetch('/api/search', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(params),
      })
      if (!res.ok) throw new Error(`Search failed: ${res.statusText}`)
      return res.json()
    },
  })
}
