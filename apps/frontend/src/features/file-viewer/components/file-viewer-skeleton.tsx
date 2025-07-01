import { Skeleton } from '@/lib/components/ui/skeleton'

export function FileViewerSkeleton() {
  // Generate skeleton lines with stable widths
  const skeletonLines = Array.from({ length: 20 }, (_, i) => ({
    id: `viewer-line-${i}`,
    width: 20 + ((i * 13) % 60), // Deterministic width based on index
  }))

  return (
    <div className="flex flex-col h-full">
      <div className="p-4 border-b">
        <Skeleton className="h-4 w-64" />
      </div>
      <div className="flex-1 p-4 space-y-2">
        {skeletonLines.map((line) => (
          <div key={line.id} className="flex gap-2">
            <Skeleton className="h-4 w-8" />
            <Skeleton className="h-4 flex-1" style={{ width: `${line.width}%` }} />
          </div>
        ))}
      </div>
    </div>
  )
}
