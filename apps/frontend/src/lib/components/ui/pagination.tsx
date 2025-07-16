import { ChevronLeft, ChevronRight, MoreHorizontal } from 'lucide-react'
import { cn } from '@/lib/utils'
import { Button } from './button'

interface PaginationProps {
  currentPage: number
  totalPages: number
  onPageChange: (page: number) => void
  className?: string
  showPageNumbers?: boolean
  siblingCount?: number
}

export function Pagination({
  currentPage,
  totalPages,
  onPageChange,
  className,
  showPageNumbers = true,
  siblingCount = 1,
}: PaginationProps) {
  const generatePaginationRange = () => {
    const totalNumbers = siblingCount * 2 + 3
    const totalBlocks = totalNumbers + 2

    if (totalPages <= totalBlocks) {
      return Array.from({ length: totalPages }, (_, i) => i + 1)
    }

    const leftSiblingIndex = Math.max(currentPage - siblingCount, 1)
    const rightSiblingIndex = Math.min(currentPage + siblingCount, totalPages)

    const shouldShowLeftDots = leftSiblingIndex > 2
    const shouldShowRightDots = rightSiblingIndex < totalPages - 1

    if (!shouldShowLeftDots && shouldShowRightDots) {
      const leftItemCount = 3 + 2 * siblingCount
      const leftRange = Array.from({ length: leftItemCount }, (_, i) => i + 1)
      return [...leftRange, 'dots', totalPages]
    }

    if (shouldShowLeftDots && !shouldShowRightDots) {
      const rightItemCount = 3 + 2 * siblingCount
      const rightRange = Array.from(
        { length: rightItemCount },
        (_, i) => totalPages - rightItemCount + i + 1,
      )
      return [1, 'dots', ...rightRange]
    }

    if (shouldShowLeftDots && shouldShowRightDots) {
      const middleRange = Array.from(
        { length: rightSiblingIndex - leftSiblingIndex + 1 },
        (_, i) => leftSiblingIndex + i,
      )
      return [1, 'dots', ...middleRange, 'dots', totalPages]
    }

    return []
  }

  const paginationRange = generatePaginationRange()

  return (
    <nav className={cn('flex items-center gap-2', className)} aria-label="Pagination">
      <Button
        variant="outline"
        size="sm"
        onClick={() => onPageChange(currentPage - 1)}
        disabled={currentPage === 1}
        aria-label="Previous page"
      >
        <ChevronLeft className="size-4" />
        <span className="sr-only sm:not-sr-only">Previous</span>
      </Button>

      {showPageNumbers && (
        <div className="flex items-center gap-1">
          {paginationRange.map((pageNumber, index) => {
            if (pageNumber === 'dots') {
              return (
                <div
                  key={`dots-${index}-${paginationRange[index - 1]}-${paginationRange[index + 1]}`}
                  className="flex size-9 items-center justify-center"
                >
                  <MoreHorizontal className="size-4 text-muted-foreground" />
                </div>
              )
            }

            return (
              <Button
                key={pageNumber}
                variant={currentPage === pageNumber ? 'default' : 'outline'}
                size="sm"
                onClick={() => onPageChange(pageNumber as number)}
                className="size-9 p-0"
                aria-label={`Go to page ${pageNumber}`}
                aria-current={currentPage === pageNumber ? 'page' : undefined}
              >
                {pageNumber}
              </Button>
            )
          })}
        </div>
      )}

      <Button
        variant="outline"
        size="sm"
        onClick={() => onPageChange(currentPage + 1)}
        disabled={currentPage === totalPages}
        aria-label="Next page"
      >
        <span className="sr-only sm:not-sr-only">Next</span>
        <ChevronRight className="size-4" />
      </Button>
    </nav>
  )
}

interface PaginationInfoProps {
  currentPage: number
  pageSize: number
  totalItems: number
  className?: string
}

export function PaginationInfo({
  currentPage,
  pageSize,
  totalItems,
  className,
}: PaginationInfoProps) {
  const startItem = (currentPage - 1) * pageSize + 1
  const endItem = Math.min(currentPage * pageSize, totalItems)

  return (
    <div className={cn('text-sm text-muted-foreground', className)}>
      Showing {startItem} to {endItem} of {totalItems} results
    </div>
  )
}
