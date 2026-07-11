export function formatRunningDuration(startTime: Date | string): string {
  const now = new Date()

  // Handle both Date objects and ISO date strings
  const startTimeDate = startTime instanceof Date ? startTime : new Date(startTime)

  // Validate that we have a valid date
  if (Number.isNaN(startTimeDate.getTime())) {
    return '0s'
  }

  const durationMs = now.getTime() - startTimeDate.getTime()

  // Ensure we don't show negative durations
  if (durationMs < 0) {
    return '0s'
  }

  const minutes = Math.floor(durationMs / 60_000)
  const seconds = Math.floor((durationMs % 60_000) / 1000)

  if (minutes > 0) {
    return `${minutes}m ${seconds}s`
  }
  return `${seconds}s`
}
