import { apiClient } from '@/lib/api/client'
import type { FileContentResponse, FileTreeResponse, SearchByNameResponse } from '../types'

export function fetchFileTree(
  taskId: string,
  path?: string,
  recursive?: boolean,
): Promise<FileTreeResponse> {
  return apiClient.get<FileTreeResponse>(`/api/sandbox/v1/tasks/${taskId}/files/tree`, {
    path,
    recursive: recursive ? true : undefined,
  })
}

export function fetchFileContent(
  taskId: string,
  path: string,
  changes?: boolean,
): Promise<FileContentResponse> {
  return apiClient.get<FileContentResponse>(`/api/sandbox/v1/tasks/${taskId}/files/read`, {
    path,
    changes,
  })
}

export function searchFilesByName(
  taskId: string,
  query: string,
  opts?: { caseSensitive?: boolean; maxResults?: number },
): Promise<SearchByNameResponse> {
  return apiClient.post<SearchByNameResponse>(
    `/api/sandbox/v1/tasks/${taskId}/files/search-by-name`,
    { query, ...opts },
  )
}
