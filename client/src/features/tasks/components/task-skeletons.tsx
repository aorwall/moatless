import { Skeleton } from '@/lib/components/ui/skeleton'

export function TaskListSkeleton() {
  return (
    <div className="space-y-3 p-4">
      {[0, 1, 2, 3, 4].map((num) => (
        <div key={`skeleton-item-${num}`} className="flex items-start space-x-3">
          <Skeleton className="h-12 w-12 rounded-lg" />
          <div className="flex-1 space-y-2">
            <Skeleton className="h-4 w-3/4" />
            <Skeleton className="h-3 w-1/2" />
          </div>
        </div>
      ))}
    </div>
  )
}

export function TaskDetailSkeleton() {
  return (
    <div className="flex flex-col h-screen">
      {/* Header skeleton */}
      <div className="border-b p-4">
        <div className="flex items-center justify-between">
          <div className="space-y-2">
            <Skeleton className="h-6 w-48" />
            <Skeleton className="h-4 w-32" />
          </div>
          <Skeleton className="h-9 w-32" />
        </div>
      </div>

      {/* Content skeleton */}
      <div className="flex flex-1 overflow-hidden">
        {/* Left panel */}
        <div className="w-1/2 border-r p-4">
          <div className="space-y-4">
            <Skeleton className="h-32 w-full" />
            <Skeleton className="h-24 w-full" />
            <Skeleton className="h-40 w-full" />
          </div>
        </div>

        {/* Right panel */}
        <div className="w-1/2 p-4">
          <div className="space-y-4">
            <Skeleton className="h-10 w-full" />
            <Skeleton className="h-64 w-full" />
          </div>
        </div>
      </div>
    </div>
  )
}

export function TaskCardSkeleton() {
  return (
    <div className="p-6 space-y-3">
      <div className="flex items-center justify-between">
        <Skeleton className="h-5 w-24" />
        <Skeleton className="h-5 w-16" />
      </div>
      <Skeleton className="h-4 w-full" />
      <Skeleton className="h-4 w-3/4" />
      <div className="flex items-center space-x-2 pt-2">
        <Skeleton className="h-4 w-20" />
        <Skeleton className="h-4 w-24" />
      </div>
    </div>
  )
}

export function SidebarTasksSkeleton() {
  return (
    <div className="space-y-1 p-2">
      {[0, 1, 2].map((num) => (
        <Skeleton key={`sidebar-item-${num}`} className="h-8 w-full rounded" />
      ))}
    </div>
  )
}
