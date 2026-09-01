import { ArrowDown, Loader2 } from 'lucide-react'
import { type ReactNode, useEffectEvent, useLayoutEffect, useRef, useState } from 'react'
import { VList, type VListHandle } from 'virtua'
import { Button } from '@/lib/components/ui/button.tsx'
import { cn } from '@/lib/utils'
import {
  isAtBottom,
  type ScrollMetrics,
  shouldFetchPreviousPage,
  shouldRearmFetch,
  shouldShowScrollDownButton,
} from './message-history-scroll.ts'

/**
 * A single virtualized row: a stable `id` plus the node to render. The id must
 * be stable across renders and prepends (message uuid / cluster key / footer
 * key) — virtua caches measured row heights by child key, and with `shift`
 * enabled for prepends, index-based keys would associate cached heights with
 * the wrong rows after older messages load.
 */
export interface VirtualRow {
  id: string
  node: ReactNode
}

interface MessageHistoryVirtualListProps {
  /**
   * One row per virtualized entry — the message/cluster rows followed by any
   * footer rows (environment indicator, thinking indicator, last-assistant
   * actions). Footers live inside the list as tail rows so stick-to-bottom
   * accounts for their height.
   */
  rows: VirtualRow[]
  /** Changes when the task switches, to re-run the initial scroll-to-bottom. */
  taskKey?: string | undefined
  /** Changes when existing row content changes without changing row count. */
  rowsVersion: string
  className?: string | undefined
  hasPreviousPage: boolean
  isFetchingPreviousPage: boolean
  fetchPreviousPage?: (() => void) | undefined
}

/**
 * Adapt the VList handle geometry to the shared `ScrollMetrics` shape.
 *
 * `bottomInset` is the height of the bottom breathing-room spacer row. virtua
 * counts that spacer in `scrollSize`, but it's layout padding, not content — so
 * we subtract it here. That makes "distance from bottom" measure real content
 * (matching the non-virtual path, where the spacer is CSS padding excluded at
 * max scroll) and keeps the at-bottom threshold aligned across both paths.
 */
function metricsFromHandle(
  handle: VListHandle,
  offset: number,
  bottomInset: number,
): ScrollMetrics {
  return {
    scrollTop: offset,
    scrollHeight: handle.scrollSize - bottomInset,
    clientHeight: handle.viewportSize,
  }
}

/**
 * Virtualized rendering of the message history (VList from `virtua`), the
 * default renderer. Keeps DOM size proportional to the viewport instead of the
 * conversation length.
 *
 * Scroll behaviours reuse the same pure logic as the non-virtual path
 * (`message-history-scroll.ts`); the geometry is read from the VList
 * handle (`scrollOffset` / `scrollSize` / `viewportSize`) instead of a DOM
 * element. Prepend uses VList's `shift` prop rather than an anchor-restore.
 */
export function MessageHistoryVirtualList({
  rows,
  taskKey,
  rowsVersion,
  className,
  hasPreviousPage,
  isFetchingPreviousPage,
  fetchPreviousPage,
}: MessageHistoryVirtualListProps) {
  const ref = useRef<VListHandle>(null)
  const [atBottom, setAtBottom] = useState(true)
  const atBottomRef = useRef(true)
  const fetchArmedRef = useRef(true)
  const [isPrepending, setIsPrepending] = useState(false)
  const isPrependingRef = useRef(false)
  const initialScrolledForTaskRef = useRef<string | undefined>(undefined)
  const initialPinningRef = useRef(false)
  const bottomSpacerRef = useRef<HTMLDivElement>(null)
  const prevCountRef = useRef(rows.length)
  const prevRowsVersionRef = useRef(rowsVersion)

  const lastIndex = rows.length - 1

  // Snap the last row to the viewport bottom. Row heights are measured
  // asynchronously, so a single scrollToIndex lands short — the estimated total
  // size keeps growing after it runs and the anchor never catches the moving
  // bottom. Re-pin across several frames until the measurements settle, marking
  // `initialPinningRef` so onScroll ignores the settling drift (which would
  // otherwise flip the at-bottom state and hide/show the button spuriously).
  // Non-smooth on purpose: smooth scrolling can't converge on a moving target.
  const pinToBottom = (frames = 12) => {
    atBottomRef.current = true
    fetchArmedRef.current = true
    initialPinningRef.current = true
    setAtBottom(true)
    let frame = 0
    let raf = requestAnimationFrame(function tick() {
      if (ref.current && rows.length > 0) {
        ref.current.scrollToIndex(rows.length - 1, { align: 'end' })
      }
      frame += 1
      if (frame < frames) {
        raf = requestAnimationFrame(tick)
      } else {
        initialPinningRef.current = false
      }
    })
    return () => {
      cancelAnimationFrame(raf)
      initialPinningRef.current = false
    }
  }
  const pinToBottomEvent = useEffectEvent(pinToBottom)

  // Initial scroll-to-bottom on mount and on task switch.
  useLayoutEffect(() => {
    if (rows.length === 0 || initialScrolledForTaskRef.current === taskKey) {
      return
    }
    initialScrolledForTaskRef.current = taskKey
    return pinToBottomEvent()
  }, [taskKey, rows.length])

  // Stick-to-bottom: when rows are appended and the user was at the bottom,
  // pin to the newest row. Skipped while a prepend is in flight (that grows the
  // list at the top, where `shift` keeps the position instead).
  useLayoutEffect(() => {
    const prevCount = prevCountRef.current
    const prevRowsVersion = prevRowsVersionRef.current
    prevCountRef.current = rows.length
    prevRowsVersionRef.current = rowsVersion
    if (isPrependingRef.current) {
      return
    }
    if ((rows.length > prevCount || rowsVersion !== prevRowsVersion) && atBottomRef.current) {
      pinToBottomEvent()
    }
  }, [rows.length, rowsVersion])

  // Clear the prepend shift flag once the prepended rows have committed.
  useLayoutEffect(() => {
    if (isPrepending) {
      isPrependingRef.current = false
      setIsPrepending(false)
    }
  }, [isPrepending])

  const handleScroll = (offset: number) => {
    const handle = ref.current
    if (!handle || initialPinningRef.current) {
      return
    }
    const metrics = metricsFromHandle(handle, offset, bottomSpacerRef.current?.offsetHeight ?? 0)
    const nowAtBottom = isAtBottom(metrics)
    atBottomRef.current = nowAtBottom
    setAtBottom(nowAtBottom)

    if (shouldRearmFetch(offset)) {
      fetchArmedRef.current = true
    }

    if (
      shouldFetchPreviousPage({
        scrollTop: offset,
        armed: fetchArmedRef.current,
        hasPreviousPage,
        isFetchingPreviousPage,
        canFetch: Boolean(fetchPreviousPage),
        anchorPending: isPrependingRef.current,
      }) &&
      fetchPreviousPage
    ) {
      fetchArmedRef.current = false
      isPrependingRef.current = true
      setIsPrepending(true)
      fetchPreviousPage()
    }
  }

  const scrollToBottom = () => {
    if (rows.length > 0) {
      pinToBottom()
    }
  }

  return (
    <div className={cn('flex-1 overflow-hidden min-h-0', className)}>
      <div className="h-full relative">
        <VList
          ref={ref}
          shift={isPrepending}
          reverse
          onScroll={handleScroll}
          className="h-full w-full overscroll-contain [&::-webkit-scrollbar]:w-2.5 [&::-webkit-scrollbar-track]:bg-transparent [&::-webkit-scrollbar-thumb]:bg-border [&::-webkit-scrollbar-thumb]:rounded-full"
          style={{ scrollbarWidth: 'thin', scrollbarColor: 'var(--border) transparent' }}
        >
          {isFetchingPreviousPage && (
            <div className="flex items-center justify-center py-2 text-xs text-muted-foreground gap-2">
              <Loader2 className="h-3 w-3 animate-spin" />
              Loading older messages…
            </div>
          )}
          {/* Top/bottom breathing room to match the non-virtual layout's
              pt-[74px] / pb-[chat-input] padding, as real rows so virtua
              measures them. */}
          <div aria-hidden className="h-[74px]" />
          {rows.map((row, index) => (
            <div
              key={row.id}
              className={cn('px-4 w-full max-w-full', index === lastIndex ? '' : 'pb-4')}
            >
              {row.node}
            </div>
          ))}
          <div ref={bottomSpacerRef} aria-hidden className="h-[var(--chat-input-height,120px)]" />
        </VList>

        {shouldShowScrollDownButton(atBottom) && (
          <div className="absolute bottom-4 right-4 z-10">
            <Button
              variant="outline"
              size="sm"
              onClick={scrollToBottom}
              className="shadow-lg bg-background/80 backdrop-blur-sm border-border/50 hover:bg-background/90 flex items-center gap-2"
            >
              <ArrowDown className="h-4 w-4" />
              Scroll to bottom
            </Button>
          </div>
        )}
      </div>
    </div>
  )
}
