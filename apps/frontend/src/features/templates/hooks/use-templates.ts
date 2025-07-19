import type { CreateVibeTaskRequest } from '@moatless/types'
import { type UseMutationOptions, useMutation, useQuery } from '@tanstack/react-query'
import { type ApiError, apiClient } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'
import type { RepositoryTemplate } from '../types'

// For now, we'll use hardcoded templates since there's no backend endpoint yet
// This matches the template defined in the backend service
const REPOSITORY_TEMPLATES: RepositoryTemplate[] = [
  {
    id: 'hello-world-ui',
    name: 'Basic Hello Website',
    description: 'A simple website template with HTML, CSS, and JavaScript',
    gitUrl: 'https://github.com/aorwall/hello-world-ui.git',
    setupCommand: 'bun install',
    previewConfig: {
      serverStartCommand: 'bun run dev',
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

type CreateVibeTaskResponse = { taskId: string; status: string }

export function useCreateVibeTask(
  options?: UseMutationOptions<CreateVibeTaskResponse, ApiError, CreateVibeTaskRequest>,
) {
  return useMutation<CreateVibeTaskResponse, ApiError, CreateVibeTaskRequest>({
    mutationFn: async (request: CreateVibeTaskRequest) => {
      return await apiClient.post<CreateVibeTaskResponse>('/api/v1/tasks/vibe', request)
    },
    ...options,
  })
}
