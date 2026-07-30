import type { LiveStatus, TaskResponse } from '@moatless/api-client'
import { useQueries } from '@tanstack/react-query'
import { Link, useNavigate, useParams } from '@tanstack/react-router'
import { ChevronDown, ChevronRight, X } from 'lucide-react'
import type React from 'react'
import { useState } from 'react'
import { useAuth } from '@/features/auth/hooks/use-auth'
import { SidebarTasksSkeleton } from '@/features/tasks/components/task-skeletons'
import { TaskStatusIcon } from '@/features/tasks/components/task-status-icon'
import { useCloseTask, useTasksList } from '@/features/tasks/hooks/use-tasks'
import { liveStatusToActivityState } from '@/features/tasks/utils/live-status'
import { taskWorkspaceLabel } from '@/features/tasks/utils/task-workspace-label'
import { taskLiveStatusQueryOptions } from '@/lib/api/task-live-status-queries'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from '@/lib/components/ui/alert-dialog'
import { Button } from '@/lib/components/ui/button'
import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from '@/lib/components/ui/sidebar'
import { TooltipProvider } from '@/lib/components/ui/tooltip'
import { activityToState, type TaskActivity, useTaskActivity } from '../hooks/use-task-activity'
import {
  DEFAULT_SIDEBAR_FILTERS,
  hasActiveSidebarFilters,
  type SidebarTaskFilterValues,
} from './sidebar-task-filter-values'
import { SidebarTaskFilters } from './sidebar-task-filters'
import {
  buildTaskGroups,
  compareActivityDesc,
  type EnrichedTask,
  flattenGroupTasks,
  RUNNING_STATE_ORDER,
  type TaskGroup,
  visibleGroupTaskIds,
} from './sidebar-task-groups'

function getRelativeTime(date: Date | string): string {
  const now = new Date()
  const then = new Date(date)
  const seconds = Math.floor((now.getTime() - then.getTime()) / 1000)

  if (seconds < 60) {
    return 'just now'
  }
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) {
    return `${minutes}m ago`
  }
  const hours = Math.floor(minutes / 60)
  if (hours < 24) {
    return `${hours}h ago`
  }
  const days = Math.floor(hours / 24)
  if (days < 7) {
    return `${days}d ago`
  }
  const weeks = Math.floor(days / 7)
  if (weeks < 4) {
    return `${weeks}w ago`
  }
  const months = Math.floor(days / 30)
  if (months < 12) {
    return `${months}mo ago`
  }
  const years = Math.floor(days / 365)
  return `${years}y ago`
}

/** Merge a task with its live activity, deriving activity state and preview. */
function enrichTask(
  task: TaskResponse,
  activity: TaskActivity | undefined,
  liveStatus: LiveStatus | undefined,
): EnrichedTask {
  // Live stream state wins; fall back to the independently-cached live status.
  const liveState = activityToState(activity)
  const state = liveState !== 'idle' ? liveState : liveStatusToActivityState(liveStatus)
  const lastActivityAt = activity?.lastMessageAt || task.lastMessageAt || task.createdAt

  // Unread when there's activity the user hasn't seen: never opened, or the
  // latest activity is newer than their read marker.
  const unread =
    !task.lastReadAt || new Date(lastActivityAt).getTime() > new Date(task.lastReadAt).getTime()

  return {
    task,
    workspaceName: taskWorkspaceLabel(task),
    state,
    unread,
    preview: activity?.lastMessage || null,
    recentMessages: activity?.recentMessages ?? [],
    lastActivityAt,
  }
}

function useLiveStatusMap(tasks: TaskResponse[]): Map<string, LiveStatus | undefined> {
  const openTasks = tasks.filter(
    (task, index) =>
      task.status === 'open' &&
      tasks.findIndex((candidate) => candidate.taskId === task.taskId) === index,
  )
  const queries = useQueries({
    queries: openTasks.map((task) => taskLiveStatusQueryOptions(task.taskId)),
  })

  return new Map(openTasks.map((task, index) => [task.taskId, queries[index]?.data]))
}

function useSidebarTaskLists(userId: string | undefined, filters: SidebarTaskFilterValues) {
  const runningQuery = useTasksList({
    limit: 100,
    status: 'open',
    ...(userId !== undefined && { followedBy: userId }),
  })
  const filteredQueryStatus = filters.status === 'all' ? undefined : filters.status
  const filteredQueryCreatedBy = filters.createdBy === 'all' ? undefined : filters.createdBy
  const filteredQueryFollowedBy = filters.onlyFollowed ? userId : undefined
  const filteredQuery = useTasksList({
    limit: 100,
    ...(filteredQueryStatus !== undefined && { status: filteredQueryStatus }),
    ...(filters.tag !== undefined && { tag: filters.tag }),
    ...(filteredQueryCreatedBy !== undefined && { createdBy: filteredQueryCreatedBy }),
    ...(filters.repositoryId !== undefined && { repositoryId: filters.repositoryId }),
    ...(filteredQueryFollowedBy !== undefined && { followedBy: filteredQueryFollowedBy }),
  })

  return { runningQuery, filteredQuery }
}

/**
 * Live activity for a running task: the last few streamed previews rendered as
 * separate rows (newest at full strength, older ones dimmed), mirroring the
 * task dashboard so running rows fill a few lines. Falls back to the single
 * latest preview until enough messages have streamed in.
 */
function RunningActivity({
  recentMessages,
  preview,
}: {
  recentMessages: string[]
  preview: string | null
}) {
  const lines = recentMessages.length > 0 ? recentMessages : preview ? [preview] : []
  if (lines.length === 0) {
    return null
  }

  return (
    <div className="flex flex-col gap-0.5">
      {lines.map((line, index) => {
        const isLast = index === lines.length - 1
        // Static preview lines never reorder, so a positional key is stable here.
        const lineKey = `${index}-${line}`
        return (
          <p
            key={lineKey}
            className={`text-xs truncate leading-snug ${
              isLast ? 'text-sidebar-foreground/80' : 'text-sidebar-foreground/45'
            }`}
          >
            {line}
          </p>
        )
      })}
    </div>
  )
}

function sidebarItemClass(depth: 0 | 1) {
  return `relative border-b border-sidebar-border/60 last:border-b-0 ${
    depth === 1 ? 'border-sidebar-border/35 bg-sidebar-accent/10' : ''
  }`
}

function sidebarButtonClass(isActive: boolean, hasActiveChild: boolean) {
  const activeClass = isActive ? 'bg-sidebar-accent' : hasActiveChild ? 'bg-sidebar-accent/50' : ''
  return `h-auto rounded-none p-2 hover:bg-sidebar-accent ${activeClass}`
}

function sidebarRowIndentClass(depth: 0 | 1, hasChildren: boolean) {
  if (depth === 1) {
    return 'pl-2'
  }
  return hasChildren ? 'pl-5' : ''
}

function ParentCountBadge({ childCount }: { childCount: number }) {
  const childCountLabel = `${childCount} ${childCount === 1 ? 'child' : 'children'}`

  return (
    <span className="shrink-0 rounded-sm bg-sidebar-border/45 px-1.5 py-0.5 text-[10px] font-medium leading-none text-sidebar-foreground/70">
      Parent · {childCountLabel}
    </span>
  )
}

function TaskTitleRow({
  taskTitle,
  showUnread,
  childCount,
}: {
  taskTitle: string
  showUnread: boolean
  childCount: number
}) {
  return (
    <div className="flex items-center gap-1.5 pr-5">
      <span
        className={`truncate text-sm leading-tight ${
          showUnread ? 'font-semibold text-sidebar-foreground' : 'font-medium'
        }`}
      >
        {taskTitle}
      </span>
      {childCount > 0 && <ParentCountBadge childCount={childCount} />}
    </div>
  )
}

function ExpandChildrenButton({
  isExpanded,
  onClick,
}: {
  isExpanded: boolean
  onClick: (event: React.MouseEvent<HTMLButtonElement>) => void
}) {
  return (
    <button
      type="button"
      aria-label={isExpanded ? 'Collapse child tasks' : 'Expand child tasks'}
      title={isExpanded ? 'Collapse child tasks' : 'Expand child tasks'}
      className={`absolute left-1 top-2 z-10 flex h-5 w-5 items-center justify-center rounded-sm hover:bg-sidebar-accent hover:text-sidebar-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring ${
        isExpanded ? 'bg-sidebar-accent text-sidebar-foreground' : 'text-sidebar-foreground/60'
      }`}
      onClick={onClick}
    >
      {isExpanded ? (
        <ChevronDown className="h-3.5 w-3.5" />
      ) : (
        <ChevronRight className="h-3.5 w-3.5" />
      )}
    </button>
  )
}

// Component for rendering individual task item
function SidebarTaskItem({
  item,
  isActive,
  hasActiveChild = false,
  depth = 0,
  childCount = 0,
  isExpanded = false,
  onToggleExpanded,
  neighborTaskId,
}: {
  item: EnrichedTask
  /** Whether this task is the one currently open in the main view. */
  isActive: boolean
  /** Whether this parent row contains the currently open task as a child. */
  hasActiveChild?: boolean
  /** Visual nesting depth. Children render one level deeper than main tasks. */
  depth?: 0 | 1
  /** Number of child rows nested under this task in the current sidebar data. */
  childCount?: number
  /** Whether this task's visible children are expanded. */
  isExpanded?: boolean
  /** Toggle child visibility for parent rows. */
  onToggleExpanded?: ((event: React.MouseEvent<HTMLButtonElement>) => void) | undefined
  /** Task to navigate to when closing the active task, or `null` if none. */
  neighborTaskId: string | null
}) {
  const { task, workspaceName, state, unread, preview, recentMessages, lastActivityAt } = item
  const agentWorking = state === 'agent-working'
  // The open task is being read right now, so never flag it as unread.
  const showUnread = unread && !isActive
  const taskTitle = task.name || task.description?.split('\n')[0] || 'Untitled'
  const navigate = useNavigate()
  const closeTask = useCloseTask(task.taskId)
  const hasChildren = childCount > 0

  const close = async (closeChildren?: boolean) => {
    // If we're closing the task that's currently open, move to the neighbouring
    // task instead of bouncing back to the dashboard.
    if (isActive) {
      if (neighborTaskId) {
        await navigate({ to: '/tasks/$task-id', params: { 'task-id': neighborTaskId } })
      } else {
        await navigate({ to: '/' })
      }
    }

    closeTask.mutate(closeChildren)
  }

  return (
    <SidebarMenuItem className={sidebarItemClass(depth)}>
      <SidebarMenuButton
        className={sidebarButtonClass(isActive, hasActiveChild)}
        aria-expanded={hasChildren ? isExpanded : undefined}
        data-parent={hasChildren ? true : undefined}
        asChild
      >
        <Link to="/tasks/$task-id" params={{ 'task-id': task.taskId }}>
          <div
            className={`flex w-full items-start gap-2 min-w-0 ${sidebarRowIndentClass(
              depth,
              hasChildren,
            )}`}
          >
            {depth === 1 && (
              <span aria-hidden="true" className="mt-2.5 h-px w-3 shrink-0 bg-sidebar-border" />
            )}
            <div className="flex items-center gap-1 pt-0.5">
              <TaskStatusIcon task={task} activityState={state} showText={false} />
            </div>
            <div className="flex flex-col min-w-0 flex-1 gap-0.5">
              <TaskTitleRow taskTitle={taskTitle} showUnread={showUnread} childCount={childCount} />
              {agentWorking ? (
                <RunningActivity recentMessages={recentMessages} preview={preview} />
              ) : (
                preview && (
                  <p className="text-xs text-sidebar-foreground/70 line-clamp-3 leading-snug">
                    {preview}
                  </p>
                )
              )}
              <div className="flex items-center justify-between gap-2 text-xs text-sidebar-foreground/60">
                <span className="truncate">{workspaceName}</span>
                <span className="shrink-0">{getRelativeTime(lastActivityAt)}</span>
              </div>
            </div>
          </div>
        </Link>
      </SidebarMenuButton>
      {hasChildren && onToggleExpanded && (
        <ExpandChildrenButton isExpanded={isExpanded} onClick={onToggleExpanded} />
      )}
      {showUnread && (
        <span
          title="Unread messages"
          className="pointer-events-none absolute right-2.5 top-2.5 h-2 w-2 rounded-full bg-sky-500 transition-opacity group-hover/menu-item:opacity-0"
        />
      )}
      {hasChildren ? (
        <AlertDialog>
          <AlertDialogTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              aria-label="Close task"
              className="absolute right-1 top-1 z-10 h-5 w-5 opacity-0 transition-opacity group-hover/menu-item:opacity-100 focus-visible:opacity-100"
              // Keep the click off the row's Link, but never call
              // `preventDefault()` here: Radix composes the trigger's own
              // open handler behind a `defaultPrevented` check, so preventing
              // the default swallows the dialog and the button does nothing.
              onClick={(event) => event.stopPropagation()}
              disabled={closeTask.isPending}
            >
              <X className="h-3.5 w-3.5" />
              <span className="sr-only">Close task</span>
            </Button>
          </AlertDialogTrigger>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>Close parent task?</AlertDialogTitle>
              <AlertDialogDescription>
                Choose whether to leave its child tasks open or close the entire task tree.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>Cancel</AlertDialogCancel>
              <AlertDialogAction onClick={() => close(false)}>Keep children open</AlertDialogAction>
              <AlertDialogAction
                className="bg-destructive text-white hover:bg-destructive/90"
                onClick={() => close(true)}
              >
                Close children too
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      ) : (
        <Button
          variant="ghost"
          size="icon"
          aria-label="Close task"
          className="absolute right-1 top-1 z-10 h-5 w-5 opacity-0 transition-opacity group-hover/menu-item:opacity-100 focus-visible:opacity-100"
          onClick={(event) => {
            event.preventDefault()
            event.stopPropagation()
            close().catch(() => undefined)
          }}
          disabled={closeTask.isPending}
        >
          <X className="h-3.5 w-3.5" />
          <span className="sr-only">Close task</span>
        </Button>
      )}
    </SidebarMenuItem>
  )
}

function SidebarTaskGroup({
  group,
  activeTaskId,
  expandedGroupIds,
  onToggleExpanded,
  neighborOf,
  depth = 0,
}: {
  group: TaskGroup
  activeTaskId: string | null
  expandedGroupIds: Set<string>
  onToggleExpanded: (taskId: string) => void
  neighborOf: (taskId: string) => string | null
  depth?: 0 | 1
}) {
  const isExpanded = expandedGroupIds.has(group.main.task.taskId)
  const hasActiveChild = group.children.some((child) =>
    flattenGroupTasks(child).some((item) => item.task.taskId === activeTaskId),
  )
  const hasChildren = group.children.length > 0
  const handleToggleExpanded = (event: React.MouseEvent<HTMLButtonElement>) => {
    event.preventDefault()
    event.stopPropagation()
    onToggleExpanded(group.main.task.taskId)
  }

  return (
    <>
      <SidebarTaskItem
        item={group.main}
        isActive={group.main.task.taskId === activeTaskId}
        hasActiveChild={hasActiveChild}
        depth={depth}
        childCount={group.children.length}
        isExpanded={isExpanded}
        onToggleExpanded={hasChildren ? handleToggleExpanded : undefined}
        neighborTaskId={neighborOf(group.main.task.taskId)}
      />
      {isExpanded && (
        <div className="ml-3 border-l border-sidebar-border/70 bg-sidebar-accent/5">
          {group.children.map((childGroup) => (
            <SidebarTaskGroup
              key={childGroup.main.task.taskId}
              group={childGroup}
              activeTaskId={activeTaskId}
              expandedGroupIds={expandedGroupIds}
              onToggleExpanded={onToggleExpanded}
              neighborOf={neighborOf}
              depth={1}
            />
          ))}
        </div>
      )}
    </>
  )
}

/** Shared header row with the group label and the filter control. */
function SidebarTasksHeader({
  filters,
  onFiltersChange,
}: {
  filters: SidebarTaskFilterValues
  onFiltersChange: (filters: SidebarTaskFilterValues) => void
}) {
  return (
    <div className="px-2">
      <SidebarGroupLabel className="px-0">Tasks</SidebarGroupLabel>
      <SidebarTaskFilters filters={filters} onChange={onFiltersChange} />
    </div>
  )
}

export function SidebarTasks() {
  const { user } = useAuth()
  const activityByTask = useTaskActivity()

  // Sidebar-local filters (status/tag/creator/repository/followed). Default to
  // the personal task list (open tasks the user follows); users can broaden it
  // via the header filter to surface not-running / other users' tasks.
  const [filters, setFilters] = useState<SidebarTaskFilterValues>(DEFAULT_SIDEBAR_FILTERS)
  const [expandedGroupIds, setExpandedGroupIds] = useState<Set<string>>(() => new Set())

  // The task currently open in the main view (if we're on a task route), used to
  // decide where to navigate when its sidebar row is closed.
  const params = useParams({ strict: false })
  const activeTaskId = (params as Record<string, string | undefined>)['task-id'] ?? null

  // Two independent queries drive the two sections:
  //  - `runningQuery` is the fixed personal list (open tasks the user follows)
  //    that feeds the always-on "Running" section — the filter never touches it.
  //  - `filteredQuery` applies the header filter and feeds the "Tasks" section.
  // When the filter is at its default, both share the same params (and thus the
  // same query key), so React Query dedupes them into a single request.
  const { runningQuery, filteredQuery } = useSidebarTaskLists(user?.id, filters)

  const liveStatusByTask = useLiveStatusMap([
    ...(runningQuery.data?.items ?? []),
    ...(filteredQuery.data?.items ?? []),
  ])

  const isLoadingTasks = runningQuery.isLoading || filteredQuery.isLoading
  const tasksError = runningQuery.error || filteredQuery.error

  // Derive the two sections. Activity state and the live message preview come
  // from the event stream, so this recomputes as those inputs update. Tasks
  // are grouped under visible parents after enrichment so child rows inherit
  // the same live/unread treatment as top-level rows.
  const { running, open } = (() => {
    const enrich = (task: TaskResponse) =>
      enrichTask(task, activityByTask.get(task.taskId), liveStatusByTask.get(task.taskId))

    const byGroupActivityDesc = (a: TaskGroup, b: TaskGroup) =>
      b.lastActivityAt.localeCompare(a.lastActivityAt)
    // Running rows sort by state, then by (immutable) creation time, so a task
    // keeps its place while it runs instead of jumping on every new message.
    const byStateThenCreated = (a: TaskGroup, b: TaskGroup) => {
      const byState = RUNNING_STATE_ORDER[a.state] - RUNNING_STATE_ORDER[b.state]
      if (byState !== 0) {
        return byState
      }
      return b.main.task.createdAt.localeCompare(a.main.task.createdAt)
    }

    // Running section: active task families from the personal list only.
    const running = buildTaskGroups((runningQuery.data?.items ?? []).map(enrich))
      .filter((group) => group.state !== 'idle')
      .sort(byStateThenCreated)
    const runningIds = new Set(
      running.flatMap((group) => flattenGroupTasks(group).map((item) => item.task.taskId)),
    )

    // Tasks section: the filtered list, minus any family member already shown
    // as running above (avoids duplicates while the personal default overlaps
    // the filter).
    const openItems = (filteredQuery.data?.items ?? [])
      .flatMap((task) => {
        const item = enrich(task)
        return runningIds.has(item.task.taskId) ? [] : [item]
      })
      .sort(compareActivityDesc)
    const open = buildTaskGroups(openItems).sort(byGroupActivityDesc)

    return { running, open }
  })()

  // Displayed order (running first, then open) so we can pick the neighbouring
  // task to jump to when the active one is closed.
  const orderedIds = [...running, ...open].flatMap((group) =>
    visibleGroupTaskIds(group, expandedGroupIds),
  )
  const neighborOf = (taskId: string): string | null => {
    const index = orderedIds.indexOf(taskId)
    if (index === -1) {
      return null
    }
    return orderedIds[index + 1] ?? orderedIds[index - 1] ?? null
  }
  const toggleGroupExpanded = (taskId: string) => {
    setExpandedGroupIds((prev) => {
      const next = new Set(prev)
      if (next.has(taskId)) {
        next.delete(taskId)
      } else {
        next.add(taskId)
      }
      return next
    })
  }

  if (isLoadingTasks) {
    return (
      <TooltipProvider>
        <SidebarGroup>
          <SidebarTasksHeader filters={filters} onFiltersChange={setFilters} />
          <SidebarGroupContent>
            <SidebarTasksSkeleton />
          </SidebarGroupContent>
        </SidebarGroup>
      </TooltipProvider>
    )
  }

  if (tasksError) {
    return (
      <TooltipProvider>
        <SidebarGroup>
          <SidebarTasksHeader filters={filters} onFiltersChange={setFilters} />
          <SidebarGroupContent>
            <div className="px-3 py-2 text-sm text-destructive">Failed to load tasks</div>
          </SidebarGroupContent>
        </SidebarGroup>
      </TooltipProvider>
    )
  }

  // Empty-state copy for the Tasks section (Running is rendered separately).
  const emptyMessage = hasActiveSidebarFilters(filters)
    ? 'No tasks match filters'
    : running.length > 0
      ? 'All tasks are running'
      : 'No tasks found'

  return (
    <TooltipProvider>
      {running.length > 0 && (
        <SidebarGroup>
          <SidebarGroupLabel className="flex items-center gap-1.5">
            <span className="relative flex h-1.5 w-1.5">
              <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-75" />
              <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-emerald-500" />
            </span>
            Running
          </SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu className="gap-0">
              {running.map((group) => (
                <SidebarTaskGroup
                  key={group.main.task.taskId}
                  group={group}
                  activeTaskId={activeTaskId}
                  expandedGroupIds={expandedGroupIds}
                  onToggleExpanded={toggleGroupExpanded}
                  neighborOf={neighborOf}
                />
              ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      )}

      <SidebarGroup>
        <SidebarTasksHeader filters={filters} onFiltersChange={setFilters} />
        <SidebarGroupContent>
          {open.length > 0 ? (
            <SidebarMenu className="gap-0">
              {open.map((group) => (
                <SidebarTaskGroup
                  key={group.main.task.taskId}
                  group={group}
                  activeTaskId={activeTaskId}
                  expandedGroupIds={expandedGroupIds}
                  onToggleExpanded={toggleGroupExpanded}
                  neighborOf={neighborOf}
                />
              ))}
            </SidebarMenu>
          ) : (
            <div className="px-3 py-2 text-sm text-muted-foreground">{emptyMessage}</div>
          )}
        </SidebarGroupContent>
      </SidebarGroup>
    </TooltipProvider>
  )
}
