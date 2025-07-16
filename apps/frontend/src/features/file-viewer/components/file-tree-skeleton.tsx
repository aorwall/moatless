import { Skeleton } from '@/lib/components/ui/skeleton'

export function FileTreeSkeleton() {
  // Generate skeleton items with stable widths
  const skeletonItems = Array.from({ length: 8 }, (_, i) => ({
    id: `tree-skeleton-${i}`,
    width: 50 + ((i * 7) % 50), // Deterministic width based on index
  }))

  return (
    <div className="p-2 space-y-1" data-testid="file-tree-skeleton">
      {skeletonItems.map((item) => (
        <div key={item.id} className="flex items-center gap-2 py-1">
          <Skeleton className="h-4 w-4" />
          <Skeleton className="h-4 w-full max-w-[180px]" style={{ width: `${item.width}%` }} />
        </div>
      ))}
    </div>
  )
}
