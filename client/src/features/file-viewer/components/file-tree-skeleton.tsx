import { Skeleton } from '@/lib/components/ui/skeleton'

export function FileTreeSkeleton() {
  return (
    <div className="p-2 space-y-1">
      {Array.from({ length: 8 }).map((_, i) => (
        <div key={i} className="flex items-center gap-2 py-1">
          <Skeleton className="h-4 w-4" />
          <Skeleton
            className="h-4 w-full max-w-[180px]"
            style={{ width: `${Math.random() * 50 + 50}%` }}
          />
        </div>
      ))}
    </div>
  )
}
