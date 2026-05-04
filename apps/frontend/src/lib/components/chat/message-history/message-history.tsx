import type { EnvironmentStatus, UiMessage } from '@moatless/api-client'
import { ArrowDown, Loader2 } from 'lucide-react'
import React, {
  Fragment,
  type ReactNode,
  useCallback,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import {
  groupSubagentMessages,
  type UiMessageWithSubagents,
} from '@/features/tasks/utils/group-messages.ts'
import { Button } from '@/lib/components/ui/button.tsx'
import { cn } from '@/lib/utils'
import { getErrorMessage } from '@/lib/utils/error.ts'
import {
  AssistantMessageComponent,
  buildToolResultMap,
  parseMessageContent,
  renderToolCalls,
} from './assistant-message.tsx'
import { ChannelMessageComponent } from './channel-message.tsx'
import { EnvironmentIndicator } from './environment-indicator.tsx'
import { ErrorMessageComponent } from './error-message.tsx'
import { MessageHistorySkeleton } from './message-history-skeleton.tsx'
import { SystemMessageComponent } from './system-message.tsx'
import { ThinkingIndicator } from './thinking-indicator.tsx'
import { ToolCallGroup } from './tool-call-group.tsx'
import { UserMessageComponent } from './user-message.tsx'

type RenderItem =
  | { kind: 'message'; message: UiMessageWithSubagents }
  | { kind: 'cluster'; key: string; messages: UiMessageWithSubagents[] }

// "Clusterable" = a normal assistant message we're willing to fold into a
// command-cluster. Special types (transcript_only, compact_summary, codex
// reasoning/error cards) keep their own rendering and break the cluster.
function isClusterableAssistantMessage(message: UiMessageWithSubagents): boolean {
  if (message.role !== 'assistant') {
    return false
  }
  const messageType = message.messageType as string | undefined
  if (
    messageType === 'transcript_only' ||
    messageType === 'compact_summary' ||
    messageType === 'error'
  ) {
    return false
  }
  if (message.sdkMetadata?.codex_reasoning === true || message.sdkMetadata?.codex_error === true) {
    return false
  }
  return true
}

function messageHasToolCalls(message: UiMessageWithSubagents): boolean {
  return parseMessageContent(message.content).toolCalls.length > 0
}

// Emit render items for one contiguous run of clusterable assistant
// messages. Folds messages from the start of the run through the LAST
// message containing a tool call into a single cluster — intermediate
// "thinking" text gets absorbed, and any trailing text-only messages (the
// agent's final response) stay standalone so the viewer still sees real
// content. If the run has no tool calls at all, every message renders
// standalone.
function emitRun(run: UiMessageWithSubagents[], items: RenderItem[]): void {
  let lastToolIdx = -1
  for (let k = 0; k < run.length; k++) {
    if (messageHasToolCalls(run[k])) {
      lastToolIdx = k
    }
  }
  if (lastToolIdx === -1) {
    for (const message of run) {
      items.push({ kind: 'message', message })
    }
    return
  }
  const clusterMsgs = run.slice(0, lastToolIdx + 1)
  items.push({
    kind: 'cluster',
    key: `cluster-${clusterMsgs[0].uuid}`,
    messages: clusterMsgs,
  })
  for (let k = lastToolIdx + 1; k < run.length; k++) {
    items.push({ kind: 'message', message: run[k] })
  }
}

function buildRenderItems(messages: UiMessageWithSubagents[], cluster: boolean): RenderItem[] {
  if (!cluster) {
    return messages.map((message) => ({ kind: 'message', message }))
  }
  const items: RenderItem[] = []
  let run: UiMessageWithSubagents[] = []
  const flushRun = () => {
    if (run.length > 0) {
      emitRun(run, items)
      run = []
    }
  }
  for (const message of messages) {
    if (isClusterableAssistantMessage(message)) {
      run.push(message)
    } else {
      flushRun()
      items.push({ kind: 'message', message })
    }
  }
  flushRun()
  return items
}

interface CommandClusterProps {
  taskId?: string
  messages: UiMessageWithSubagents[]
  expandedToolCalls: Set<string>
  onToggleToolCall: (toolUseId: string) => void
}

function CommandCluster({
  taskId,
  messages,
  expandedToolCalls,
  onToggleToolCall,
}: CommandClusterProps) {
  let totalCount = 0
  const blocks: ReactNode[] = []
  for (const message of messages) {
    const { toolCalls } = parseMessageContent(message.content)
    totalCount += toolCalls.length
    const node = renderToolCalls(
      taskId,
      toolCalls,
      buildToolResultMap(message),
      expandedToolCalls,
      onToggleToolCall,
      message.hasPendingUserInput,
      message.subagentMessages,
    )
    if (node) {
      blocks.push(<Fragment key={message.uuid}>{node}</Fragment>)
    }
  }
  return (
    <ToolCallGroup count={totalCount}>
      <div className="space-y-2">{blocks}</div>
    </ToolCallGroup>
  )
}

interface MessageHistoryProps {
  taskId?: string
  messages?: UiMessage[]
  isLoading?: boolean
  error?: Error | null
  className?: string
  isRunning?: boolean
  environmentStatus?: EnvironmentStatus
  environmentError?: string
  onPreviewUrlClick?: (path: string) => void
  hasPreviousPage?: boolean
  isFetchingPreviousPage?: boolean
  fetchPreviousPage?: () => void
  /**
   * Wrap each assistant message's tool calls in a single collapsed group
   * (always shown, even for one call) that displays a live "N commands"
   * counter. Defaults to false.
   */
  collapseToolCalls?: boolean
}

// How close to the top (in px) before we trigger a previous-page fetch.
const FETCH_THRESHOLD = 300
// How close to the bottom (in px) before we consider the user "at bottom"
// for stick-to-bottom behaviour and the scroll-down button visibility.
const AT_BOTTOM_THRESHOLD = 50

function MessageHistoryComponent({
  taskId,
  messages,
  isLoading = false,
  error = null,
  className,
  isRunning = false,
  environmentStatus,
  environmentError,
  onPreviewUrlClick,
  hasPreviousPage = false,
  isFetchingPreviousPage = false,
  fetchPreviousPage,
  collapseToolCalls = false,
}: MessageHistoryProps) {
  const [expandedToolCalls, setExpandedToolCalls] = useState<Set<string>>(new Set())
  const scrollContainerRef = useRef<HTMLDivElement | null>(null)
  const [atBottom, setAtBottom] = useState(true)

  // Captured scroll metrics taken right before we trigger
  // `fetchPreviousPage`. After the new page mounts we restore scroll position
  // to `top + (newScrollHeight - oldScrollHeight)` so the user's reading
  // position stays anchored to the same items.
  const prependAnchorRef = useRef<{ top: number; height: number } | null>(null)
  // Tracks whether we've performed the initial scroll-to-bottom for the
  // current task so we don't re-pin to the bottom on every re-render.
  const initialScrolledForTaskRef = useRef<string | undefined>(undefined)
  // Snapshot of the scroll container's metrics at the END of the previous
  // commit (and updated eagerly from `handleScroll`). Read by the
  // stick-to-bottom effect to decide whether the user was at the bottom
  // BEFORE this commit's data change — avoiding the stale-ref race where a
  // mid-scroll user gets snapped down because a queued `scroll` event hasn't
  // run yet when SSE-driven re-render fires.
  const prevScrollHeightRef = useRef(0)
  const prevScrollTopRef = useRef(0)
  // Re-arms the previous-page fetch trigger only after the user scrolls past
  // `FETCH_THRESHOLD`. Prevents a runaway fetch loop when the prepended page
  // is short enough that the anchor-restored scroll position lands back
  // inside the fetch zone — which used to fire `fetchPreviousPage` again on
  // the next scroll event with no user intent.
  const fetchArmedRef = useRef(true)

  const toggleToolCallExpansion = useCallback((toolUseId: string) => {
    setExpandedToolCalls((prev) => {
      const newSet = new Set(prev)
      if (newSet.has(toolUseId)) {
        newSet.delete(toolUseId)
      } else {
        newSet.add(toolUseId)
      }
      return newSet
    })
  }, [])

  // Subagent messages are attached under their parent Agent tool call. We
  // also drop empty-content rows here — backend pagination keeps them in the
  // page count so the cursor stays honest, but they shouldn't render as
  // blank gaps.
  const flattenedMessages = useMemo(() => {
    if (!messages) {
      return []
    }
    const renderable = messages.filter(
      (m) =>
        m.content.length > 0 ||
        (m.toolResults && m.toolResults.length > 0) ||
        m.result != null ||
        m.hasPendingUserInput === true,
    )
    return groupSubagentMessages(renderable)
  }, [messages])

  // Scroll to the bottom on initial load (and on task switch).
  useLayoutEffect(() => {
    if (
      flattenedMessages.length === 0 ||
      !scrollContainerRef.current ||
      initialScrolledForTaskRef.current === taskId
    ) {
      return
    }
    initialScrolledForTaskRef.current = taskId
    const el = scrollContainerRef.current
    el.scrollTop = el.scrollHeight
    fetchArmedRef.current = true
    setAtBottom(true)
  }, [flattenedMessages.length, taskId])

  // After the previous-page fetch commits, restore the scroll position so
  // the user's reading anchor doesn't shift even though the list grew at the
  // top. Runs synchronously after DOM commit (no flicker).
  // biome-ignore lint/correctness/useExhaustiveDependencies: explicit length dep
  useLayoutEffect(() => {
    if (!prependAnchorRef.current || isFetchingPreviousPage || !scrollContainerRef.current) {
      return
    }
    const { top, height } = prependAnchorRef.current
    const el = scrollContainerRef.current
    const delta = el.scrollHeight - height
    if (delta > 0) {
      el.scrollTop = top + delta
    }
    prependAnchorRef.current = null
  }, [isFetchingPreviousPage, flattenedMessages.length])

  // Stick-to-bottom: snap to the new bottom only if the user was at the
  // bottom JUST BEFORE this commit's data change. We compare prev-commit
  // metrics (captured below in the snapshot effect, and updated eagerly by
  // `handleScroll`) to dodge the race where a queued `scroll` event hasn't
  // fired yet when an SSE-driven re-render lands — the old `stickToBottomRef`
  // approach would yank the user down mid-scroll because the ref still held
  // its previous value.
  // biome-ignore lint/correctness/useExhaustiveDependencies: explicit length dep
  useLayoutEffect(() => {
    const el = scrollContainerRef.current
    if (!el) {
      return
    }
    if (prependAnchorRef.current) {
      // Currently servicing a previous-page fetch; let that effect handle
      // scroll position instead.
      return
    }
    const prevSH = prevScrollHeightRef.current
    const prevST = prevScrollTopRef.current
    const ch = el.clientHeight
    const oldDistanceFromBottom = prevSH - prevST - ch
    if (oldDistanceFromBottom > AT_BOTTOM_THRESHOLD) {
      return
    }
    el.scrollTop = el.scrollHeight
  }, [flattenedMessages.length])

  // Snapshot the scroll container's metrics at the end of every commit so
  // the next commit's stick-to-bottom decision sees the user's position
  // BEFORE the upcoming data change. Declared after the stick-to-bottom
  // effect so it runs after it within the same commit.
  useLayoutEffect(() => {
    const el = scrollContainerRef.current
    if (!el) {
      return
    }
    prevScrollHeightRef.current = el.scrollHeight
    prevScrollTopRef.current = el.scrollTop
  })

  const handleScroll = useCallback(() => {
    const el = scrollContainerRef.current
    if (!el) {
      return
    }
    const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight
    const nowAtBottom = distanceFromBottom <= AT_BOTTOM_THRESHOLD
    setAtBottom(nowAtBottom)
    // Eagerly capture the user's scroll position so the next stick-to-bottom
    // decision (which may run before the next React commit) reflects where
    // the user actually is right now.
    prevScrollHeightRef.current = el.scrollHeight
    prevScrollTopRef.current = el.scrollTop

    // Re-arm the trigger once the user scrolls back away from the top.
    if (el.scrollTop > FETCH_THRESHOLD) {
      fetchArmedRef.current = true
    }

    if (
      el.scrollTop <= FETCH_THRESHOLD &&
      fetchArmedRef.current &&
      hasPreviousPage &&
      !isFetchingPreviousPage &&
      fetchPreviousPage &&
      !prependAnchorRef.current
    ) {
      // Disarm immediately so cascading scroll events from the anchor restore
      // (or a short prepended page that lands the user back near the top) do
      // not retrigger the fetch — only a deliberate scroll past the threshold
      // re-arms it.
      fetchArmedRef.current = false
      prependAnchorRef.current = {
        top: el.scrollTop,
        height: el.scrollHeight,
      }
      fetchPreviousPage()
    }
  }, [hasPreviousPage, isFetchingPreviousPage, fetchPreviousPage])

  const scrollToBottom = useCallback(() => {
    const el = scrollContainerRef.current
    if (!el) {
      return
    }
    el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' })
  }, [])

  const renderRow = useCallback(
    (item: UiMessage) => {
      if (item.messageType === 'error') {
        return <ErrorMessageComponent key={item.uuid} message={item} />
      }
      if (item.role === 'user' && item.sourceTaskId) {
        return <ChannelMessageComponent key={item.uuid} message={item} variant="task" />
      }
      if (item.role === 'user' && item.channel) {
        return <ChannelMessageComponent key={item.uuid} message={item} />
      }
      if (item.role === 'user') {
        return (
          <UserMessageComponent
            key={item.uuid}
            message={item}
            onPreviewUrlClick={onPreviewUrlClick}
          />
        )
      }
      if (item.role === 'system') {
        return <SystemMessageComponent key={item.uuid} message={item} />
      }
      return (
        <AssistantMessageComponent
          key={item.uuid}
          taskId={taskId}
          message={item}
          expandedToolCalls={expandedToolCalls}
          onToggleToolCall={toggleToolCallExpansion}
        />
      )
    },
    [expandedToolCalls, onPreviewUrlClick, taskId, toggleToolCallExpansion],
  )

  const renderItems = useMemo(
    () => buildRenderItems(flattenedMessages, collapseToolCalls),
    [flattenedMessages, collapseToolCalls],
  )

  if (isLoading && (!messages || messages.length === 0)) {
    return <MessageHistorySkeleton />
  }

  if (error) {
    return (
      <div
        className={cn('flex-1 min-h-0 flex flex-col items-center justify-center p-4', className)}
      >
        <div className="bg-destructive/10 border border-destructive/20 rounded-lg p-4">
          <div className="text-sm text-destructive text-center">
            Failed to load messages: {getErrorMessage(error)}
          </div>
        </div>
      </div>
    )
  }

  if (messages && messages.length === 0) {
    return (
      <div className={cn('flex-1 overflow-hidden min-h-0', className)}>
        <div className="h-full pt-[74px] pb-[var(--chat-input-height,120px)] px-4">
          <div className="text-center text-muted-foreground py-8">
            <p>No messages yet. Start a conversation!</p>
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className={cn('flex-1 overflow-hidden min-h-0', className)}>
      <div className="h-full relative">
        <div
          ref={scrollContainerRef}
          onScroll={handleScroll}
          className="h-full w-full overflow-y-auto overscroll-contain [&::-webkit-scrollbar]:w-2.5 [&::-webkit-scrollbar-track]:bg-transparent [&::-webkit-scrollbar-thumb]:bg-border [&::-webkit-scrollbar-thumb]:rounded-full [&::-webkit-scrollbar-thumb]:transition-colors"
          style={{
            scrollbarWidth: 'thin',
            scrollbarColor: 'hsl(var(--border)) transparent',
          }}
        >
          <div className="px-4 pt-[74px] pb-[var(--chat-input-height,120px)] space-y-4 w-full max-w-full">
            {isFetchingPreviousPage && (
              <div className="flex items-center justify-center py-2 text-xs text-muted-foreground gap-2">
                <Loader2 className="h-3 w-3 animate-spin" />
                Loading older messages…
              </div>
            )}
            {renderItems.map((item) =>
              item.kind === 'cluster' ? (
                <CommandCluster
                  key={item.key}
                  taskId={taskId}
                  messages={item.messages}
                  expandedToolCalls={expandedToolCalls}
                  onToggleToolCall={toggleToolCallExpansion}
                />
              ) : (
                renderRow(item.message)
              ),
            )}
            {environmentStatus === 'initializing' && <EnvironmentIndicator status="initializing" />}
            {environmentStatus === 'error' && (
              <EnvironmentIndicator status="error" errorMessage={environmentError} />
            )}
            {isRunning && environmentStatus !== 'initializing' && <ThinkingIndicator />}
          </div>
        </div>

        {!atBottom && (
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

export const MessageHistory = React.memo(MessageHistoryComponent)
