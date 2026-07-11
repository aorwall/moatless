import type { UseQueryOptions } from '@tanstack/react-query'
import { useQuery } from '@tanstack/react-query'
import type { TaskTemplateResponse } from '@/features/task-templates/api/task-templates'
import { taskTemplatesApi } from '@/features/task-templates/api/task-templates'
import { queryKeys } from '@/lib/api/query-keys'

export function useTaskTemplatesList(
  options?: Omit<UseQueryOptions<TaskTemplateResponse[]>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.taskTemplates.list(),
    queryFn: () => taskTemplatesApi.listTaskTemplates(),
    ...options,
  })
}

export function useFeaturedTaskTemplates(
  { limit }: { limit?: number } = {},
  options?: Omit<UseQueryOptions<TaskTemplateResponse[]>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.taskTemplates.featured(),
    queryFn: () => taskTemplatesApi.getFeaturedTaskTemplates(limit ?? 6),
    ...options,
  })
}

export function useTaskTemplate(
  id: string | undefined,
  options?: Omit<UseQueryOptions<TaskTemplateResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.taskTemplates.detail(id ?? ''),
    queryFn: () =>
      id ? taskTemplatesApi.getTaskTemplate(id) : Promise.reject(new Error('id is required')),
    enabled: !!id,
    ...options,
  })
}
