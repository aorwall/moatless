import { Skeleton } from '@/lib/components/ui/skeleton.tsx'

export function MessageHistorySkeleton() {
  return (
    <div className="h-full flex flex-col">
      <div className="flex-1 overflow-hidden p-4">
        <div className="space-y-4">
          {/* User message skeleton */}
          <div className="flex justify-end">
            <div className="bg-primary/10 rounded-lg p-3 max-w-[80%]">
              <Skeleton className="h-4 w-64 mb-2" />
              <Skeleton className="h-4 w-48" />
            </div>
          </div>

          {/* Assistant message skeleton */}
          <div className="flex justify-start">
            <div className="bg-secondary rounded-lg p-3 max-w-[80%]">
              <Skeleton className="h-4 w-80 mb-2" />
              <Skeleton className="h-4 w-64 mb-2" />
              <Skeleton className="h-4 w-48" />
            </div>
          </div>

          {/* Tool call skeleton */}
          <div className="flex justify-start">
            <div className="bg-secondary rounded-lg p-3 max-w-[80%]">
              <div className="flex items-center gap-2 mb-2">
                <Skeleton className="h-4 w-4 rounded" />
                <Skeleton className="h-4 w-32" />
              </div>
              <div className="ml-6 bg-background rounded p-2">
                <Skeleton className="h-4 w-48 mb-1" />
                <Skeleton className="h-4 w-32" />
              </div>
            </div>
          </div>

          {/* Another user message skeleton */}
          <div className="flex justify-end">
            <div className="bg-primary/10 rounded-lg p-3 max-w-[80%]">
              <Skeleton className="h-4 w-56" />
            </div>
          </div>
        </div>
      </div>

      {/* Chat input skeleton */}
      <div className="p-4 border-t border-border">
        <Skeleton className="h-10 w-full rounded-md" />
      </div>
    </div>
  )
}
