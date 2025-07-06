import { useMutation, useQuery } from '@tanstack/react-query'
import type { CreateVibeTaskRequest } from '@moatless/shared/types/api'
import { apiClient } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'
import type { RepositoryTemplate } from '../types'

// For now, we'll use hardcoded templates since there's no backend endpoint yet
// This matches the template defined in the backend service
const REPOSITORY_TEMPLATES: RepositoryTemplate[] = [
  {
    id: 'basic-hello-website',
    name: 'Basic Hello Website',
    description: 'A simple website template with HTML, CSS, and JavaScript',
    gitUrl: 'https://github.com/aorwall/basic-hello-website.git',
    setupCommand: 'npm install',
    previewConfig: {
      serverStartCommand: 'npm run dev',
      serverPort: 3000,
      injectionStrategy: {
        method: 'html-injection',
      },
    },
  },
]

export function useTemplatesList(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: queryKeys.templates.list(),
    queryFn: async () => {
      // For now, return hardcoded templates
      // In the future, this could fetch from an API endpoint
      return { templates: REPOSITORY_TEMPLATES }
    },
    staleTime: 5 * 60 * 1000, // 5 minutes
    enabled: options?.enabled,
  })
}

export function useCreateVibeTask(options?: {
  onSuccess?: (data: { taskId: string; status: string }) => void
  onError?: (error: Error) => void
}) {
  return useMutation({
    mutationFn: async (request: CreateVibeTaskRequest) => {
      return await apiClient.post<{ taskId: string; status: string }>('/api/tasks/vibe', request)
    },
    onSuccess: options?.onSuccess,
    onError: options?.onError,
  })
}