/**
 * Format an ISO-8601 timestamp as an absolute local date-time string.
 *
 * A missing value renders `-`; an unparseable one renders verbatim, so a
 * backend value this never anticipated is still visible.
 */
export function formatTimestamp(value: string | null | undefined): string {
  if (!value) {
    return '-'
  }
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) {
    return value
  }
  return date.toLocaleString()
}
