import type { TaskResponse } from '@moatless/api-client'

/**
 * Mirror of the reaper's "last activity" rule in
 * `backend/src/task/reaper.rs` (lines 153–191): the max of
 * `lastMessageAt`, `sandboxStartedAt`, and `createdAt` (fallback).
 */
export function getLastActivityMs(task: TaskResponse): number {
  const candidates = [
    task.lastMessageAt ? Date.parse(task.lastMessageAt) : Number.NaN,
    task.sandboxStartedAt ? Date.parse(task.sandboxStartedAt) : Number.NaN,
    task.createdAt ? Date.parse(task.createdAt) : Number.NaN,
  ].filter((n) => Number.isFinite(n))
  return candidates.length > 0 ? Math.max(...candidates) : Date.now()
}

export function getIdleStopDeadlineMs(task: TaskResponse): number | null {
  if (task.ttlMinutes <= 0) {
    return null
  }
  return getLastActivityMs(task) + task.ttlMinutes * 60_000
}
