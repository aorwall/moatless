/**
 * Format a unix-ms timestamp as a relative duration string.
 *
 * Future timestamps produce "in Xd Yh", "in Xh Ym", or "in Xm Ys" depending on
 * magnitude. Past timestamps produce "Xd ago", etc.
 */
export function formatRelativeTime(unixMs: number, now: number = Date.now()): string {
  const diffSec = Math.round((unixMs - now) / 1000)
  const abs = Math.abs(diffSec)
  const days = Math.floor(abs / 86_400)
  const hours = Math.floor((abs % 86_400) / 3600)
  const minutes = Math.floor((abs % 3600) / 60)
  const seconds = abs % 60

  const parts: string[] = []
  if (days > 0) {
    parts.push(`${days}d`)
    if (hours > 0) {
      parts.push(`${hours}h`)
    }
  } else if (hours > 0) {
    parts.push(`${hours}h`)
    if (minutes > 0) {
      parts.push(`${minutes}m`)
    }
  } else if (minutes > 0) {
    parts.push(`${minutes}m`)
  } else {
    parts.push(`${seconds}s`)
  }

  const formatted = parts.join(' ')
  return diffSec >= 0 ? `in ${formatted}` : `${formatted} ago`
}
