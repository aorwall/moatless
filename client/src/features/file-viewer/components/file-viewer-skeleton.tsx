import { Skeleton } from '@/lib/components/ui/skeleton'

export function FileViewerSkeleton() {
  return (
    <div className="flex flex-col h-full">
      <div className="p-4 border-b">
        <Skeleton className="h-4 w-64" />
      </div>
      <div className="flex-1 p-4 space-y-2">
        {Array.from({ length: 20 }).map((_, i) => (
          <div key={i} className="flex gap-2">
            <Skeleton className="h-4 w-8" />
            <Skeleton className="h-4 flex-1" style={{ width: `${Math.random() * 60 + 20}%` }} />
          </div>
        ))}
      </div>
    </div>
  )
}
