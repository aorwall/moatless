export function formatRunningDuration(startTime: Date): string {
  const now = new Date()
  const durationMs = now.getTime() - startTime.getTime()
  const minutes = Math.floor(durationMs / 60000)
  const seconds = Math.floor((durationMs % 60000) / 1000)

  if (minutes > 0) {
    return `${minutes}m ${seconds}s`
  }
  return `${seconds}s`
}
