import type { ReactNode } from 'react'

/**
 * A single labelled value in a detail view, rendered as a `dt`/`dd` pair.
 * Intended to be used inside a `dl` grid (see `DetailHeader`/detail routes).
 */
export function DetailItem({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div>
      <dt className="text-xs font-medium text-muted-foreground">{label}</dt>
      <dd className="mt-1 text-sm">{value}</dd>
    </div>
  )
}
