import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  createPreviewSession,
  deletePreviewSession,
  getPreviewSession,
  getPreviewSessions,
  updatePreviewSession,
} from '../api/preview'
import type { PreviewConfig } from '../types'

// Add preview query keys to the queryKeys factory
const previewKeys = {
  all: ['preview'] as const,
  sessions: () => [...previewKeys.all, 'sessions'] as const,
  session: (id: string) => [...previewKeys.sessions(), id] as const,
}

/**
 * Hook to fetch all preview sessions
 */
export function usePreviewSessions() {
  return useQuery({
    queryKey: previewKeys.sessions(),
    queryFn: getPreviewSessions,
  })
}

/**
 * Hook to fetch a single preview session
 */
export function usePreviewSession(sessionId: string | undefined) {
  return useQuery({
    queryKey: previewKeys.session(sessionId!),
    queryFn: () => getPreviewSession(sessionId!),
    enabled: !!sessionId,
  })
}

/**
 * Hook to create a new preview session
 */
export function useCreatePreviewSession() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: createPreviewSession,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: previewKeys.sessions() })
    },
  })
}

/**
 * Hook to update a preview session
 */
export function useUpdatePreviewSession(sessionId: string) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (config: Partial<PreviewConfig>) => updatePreviewSession(sessionId, config),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: previewKeys.session(sessionId) })
      queryClient.invalidateQueries({ queryKey: previewKeys.sessions() })
    },
  })
}

/**
 * Hook to delete a preview session
 */
export function useDeletePreviewSession() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: deletePreviewSession,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: previewKeys.sessions() })
    },
  })
}
