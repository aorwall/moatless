import { useQuery } from '@tanstack/react-query'

interface FileContentResponse {
  content: string
  oldContent?: string
  additions?: number
  deletions?: number
  isModified?: boolean
}

export function useFileContent(filePath: string | null, withChanges = true) {
  return useQuery<FileContentResponse>({
    queryKey: ['file-content', filePath, withChanges],
    queryFn: async () => {
      if (!filePath) throw new Error('No file path')
      const params = new URLSearchParams({ path: filePath })
      if (withChanges) params.set('changes', 'true')
      const res = await fetch(`/api/file?${params}`)
      if (!res.ok) throw new Error(`Failed to fetch file: ${res.statusText}`)
      return res.json()
    },
    enabled: !!filePath,
  })
}
