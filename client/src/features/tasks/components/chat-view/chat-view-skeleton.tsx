import { Skeleton } from '@/lib/components/ui/skeleton'

export function ChatViewSkeleton() {
  return (
    <div className="h-full flex flex-col">
      {/* Header skeleton */}
      <div className="border-b p-4">
        <Skeleton className="h-6 w-48" />
      </div>

      {/* Messages area skeleton */}
      <div className="flex-1 overflow-hidden p-4 space-y-4">
        {/* Initial message skeleton */}
        <div className="bg-secondary rounded-lg p-4">
          <Skeleton className="h-4 w-full mb-2" />
          <Skeleton className="h-4 w-3/4" />
        </div>

        {/* Iteration info skeleton */}
        <div className="flex justify-center py-2">
          <div className="flex items-center gap-4">
            <Skeleton className="h-4 w-16" />
            <Skeleton className="h-4 w-20" />
            <Skeleton className="h-4 w-16" />
          </div>
        </div>

        {/* Result skeleton */}
        <div className="space-y-2">
          <Skeleton className="h-4 w-full" />
          <Skeleton className="h-4 w-5/6" />
          <Skeleton className="h-4 w-4/6" />
        </div>
      </div>

      {/* Chat input skeleton */}
      <div className="p-4 border-t">
        <Skeleton className="h-10 w-full" />
      </div>
    </div>
  )
}
