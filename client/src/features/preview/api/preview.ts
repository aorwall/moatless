import { apiClient } from '@/lib/api/client';
import type { PreviewConfig } from '../types';

export interface PreviewSession {
  id: string;
  config: PreviewConfig;
  createdAt: string;
  updatedAt: string;
}

/**
 * Create a new preview session
 */
export async function createPreviewSession(config: PreviewConfig): Promise<PreviewSession> {
  return apiClient.post<PreviewSession>('/api/preview/sessions', config);
}

/**
 * Get preview session by ID
 */
export async function getPreviewSession(sessionId: string): Promise<PreviewSession> {
  return apiClient.get<PreviewSession>(`/api/preview/sessions/${sessionId}`);
}

/**
 * Update preview session configuration
 */
export async function updatePreviewSession(
  sessionId: string,
  config: Partial<PreviewConfig>
): Promise<PreviewSession> {
  return apiClient.patch<PreviewSession>(
    `/api/preview/sessions/${sessionId}`,
    config
  );
}

/**
 * Delete preview session
 */
export async function deletePreviewSession(sessionId: string): Promise<void> {
  await apiClient.delete(`/api/preview/sessions/${sessionId}`);
}

/**
 * Get all preview sessions
 */
export async function getPreviewSessions(): Promise<PreviewSession[]> {
  return apiClient.get<PreviewSession[]>('/api/preview/sessions');
}