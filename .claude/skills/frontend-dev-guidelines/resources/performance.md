# Performance Optimization

## Table of Contents
- [Overview](#overview)
- [When to Optimize](#when-to-optimize)
- [React Memoization](#react-memoization)
- [Code Splitting](#code-splitting)
- [Image Optimization](#image-optimization)
- [Query Optimization](#query-optimization)
- [Common Patterns](#common-patterns)
- [Anti-Patterns](#anti-patterns)

## Overview

Performance optimization should be done strategically, not prematurely.

### Key Principles
- Profile before optimizing
- Target actual bottlenecks
- Use React DevTools Profiler
- Leverage React Query's caching
- Code split large components

## When to Optimize

### Don't Optimize Prematurely
Only optimize when you have:
1. Measured performance issue (>100ms render)
2. Identified the bottleneck
3. Determined it impacts user experience

### Signs You Need to Optimize
- Slow page loads (>3s)
- Laggy interactions (>100ms)
- Janky scrolling
- High memory usage
- React DevTools shows excessive re-renders

### Profiling Tools
```tsx
// Use React DevTools Profiler
// 1. Open Chrome DevTools
// 2. Go to Profiler tab
// 3. Click record
// 4. Perform action
// 5. Stop recording
// 6. Analyze render times
```

## React Memoization

### useMemo - Expensive Calculations
Use `useMemo` when calculation is >50ms:

```tsx
import { useMemo } from 'react'

function TaskAnalytics({ tasks }: { tasks: Task[] }) {
  // GOOD: Expensive calculation cached
  const statistics = useMemo(() => {
    // Complex statistical analysis (>50ms)
    return calculateStatistics(tasks)
  }, [tasks])

  return <div>{/* Render statistics */}</div>
}
```

**DON'T use for simple calculations**:
```tsx
// BAD: useMemo overhead > calculation cost
const doubledValue = useMemo(() => value * 2, [value])

// GOOD: Just calculate directly
const doubledValue = value * 2
```

### useCallback - Event Handlers in Lists
Use `useCallback` when passing callbacks to memoized children:

```tsx
import { useCallback } from 'react'

function TaskList({ tasks }: { tasks: Task[] }) {
  // GOOD: Callback memoized for memoized children
  const handleTaskClick = useCallback((id: string) => {
    navigate(`/tasks/${id}`)
  }, [navigate])

  return (
    <div>
      {tasks.map(task => (
        <MemoizedTaskCard
          key={task.id}
          task={task}
          onClick={handleTaskClick}  // Stable reference
        />
      ))}
    </div>
  )
}
```

**DON'T use without memo**:
```tsx
// BAD: useCallback without React.memo is useless
function TaskList() {
  const handleClick = useCallback(() => { ... }, [])

  return <TaskCard onClick={handleClick} />  // TaskCard not memoized!
}
```

### React.memo - Prevent Re-renders
Use `React.memo` for expensive components that re-render frequently:

```tsx
import { memo } from 'react'

// GOOD: Expensive component memoized
export const TaskCard = memo(function TaskCard({ task }: TaskCardProps) {
  // Complex rendering logic
  return (
    <div className="border rounded-lg p-4">
      {/* ... */}
    </div>
  )
})

// Parent re-renders but TaskCard only re-renders when task changes
function TaskList({ tasks, filter }: Props) {
  return (
    <div>
      <FilterBar filter={filter} />  {/* Changes frequently */}
      {tasks.map(task => (
        <TaskCard key={task.id} task={task} />  {/* Memoized! */}
      ))}
    </div>
  )
}
```

**DON'T memo everything**:
```tsx
// BAD: Overhead > benefit for simple component
export const SimpleText = memo(({ text }: { text: string }) => {
  return <span>{text}</span>
})

// GOOD: Just use regular component
export function SimpleText({ text }: { text: string }) {
  return <span>{text}</span>
}
```

### When to Use Each

**useMemo**:
- Expensive calculations (>50ms)
- Complex data transformations
- Large array filtering/sorting
- Object/array creation in render

**useCallback**:
- Callbacks passed to memoized children
- Dependencies in useEffect/useMemo
- Callbacks passed to third-party libraries

**React.memo**:
- Components with expensive rendering
- Components that re-render frequently
- List item components (with stable props)

## Code Splitting

### Route-Based Splitting
TanStack Router handles this automatically:

```tsx
// Automatic code splitting per route
export const Route = createFileRoute('/tasks/$taskId')({
  component: TaskDetailPage  // Automatically code-split
})
```

### Component-Level Splitting
Use `React.lazy` for large components:

```tsx
import { lazy, Suspense } from 'react'

// Heavy component loaded on-demand
const HeavyChart = lazy(() => import('@/components/heavy-chart'))

export function Analytics() {
  return (
    <Suspense fallback={<LoadingSpinner />}>
      <HeavyChart data={data} />
    </Suspense>
  )
}
```

### Conditional Loading
```tsx
import { lazy, Suspense, useState } from 'react'

const AdvancedSettings = lazy(() => import('./advanced-settings'))

export function Settings() {
  const [showAdvanced, setShowAdvanced] = useState(false)

  return (
    <div>
      <BasicSettings />

      <button onClick={() => setShowAdvanced(true)}>
        Show Advanced
      </button>

      {showAdvanced && (
        <Suspense fallback={<LoadingSpinner />}>
          <AdvancedSettings />
        </Suspense>
      )}
    </div>
  )
}
```

## Image Optimization

### Responsive Images
```tsx
export function TaskImage({ src, alt }: { src: string; alt: string }) {
  return (
    <img
      src={src}
      alt={alt}
      loading="lazy"  // Lazy load images
      className="w-full h-auto"
    />
  )
}
```

### Image Sizing
```tsx
// Specify dimensions to prevent layout shift
<img
  src={src}
  alt={alt}
  width={400}
  height={300}
  className="object-cover"
/>
```

## Query Optimization

### Selective Queries
Only fetch what you need:

```tsx
// BAD: Fetches all task data
function TaskTitle({ taskId }: { taskId: string }) {
  const { data: task } = useTaskQuery(taskId)
  return <h1>{task.title}</h1>
}

// GOOD: Only fetch title
function TaskTitle({ taskId }: { taskId: string }) {
  const { data: title } = useTaskTitleQuery(taskId)
  return <h1>{title}</h1>
}
```

### Prefetching
Prefetch data before it's needed:

```tsx
import { useQueryClient } from '@tanstack/react-query'

function TaskCard({ task }: { task: Task }) {
  const queryClient = useQueryClient()

  // Prefetch on hover
  const handleMouseEnter = () => {
    queryClient.prefetchQuery({
      queryKey: queryKeys.tasks.detail(task.id),
      queryFn: () => fetchTask(task.id)
    })
  }

  return (
    <Link
      to="/tasks/$taskId"
      params={{ taskId: task.id }}
      onMouseEnter={handleMouseEnter}
    >
      {task.title}
    </Link>
  )
}
```

### Pagination
Use pagination for large lists:

```tsx
function TaskList() {
  const [page, setPage] = useState(1)
  const { data, isLoading } = usePaginatedTasksQuery(page)

  return (
    <div>
      {data.tasks.map(task => <TaskCard key={task.id} {...task} />)}

      <Pagination
        page={page}
        totalPages={data.totalPages}
        onPageChange={setPage}
      />
    </div>
  )
}
```

### Infinite Loading
```tsx
import { useInfiniteQuery } from '@tanstack/react-query'

function TaskList() {
  const {
    data,
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage
  } = useInfiniteTasksQuery()

  return (
    <div>
      {data?.pages.map(page =>
        page.tasks.map(task => <TaskCard key={task.id} {...task} />)
      )}

      {hasNextPage && (
        <button
          onClick={() => fetchNextPage()}
          disabled={isFetchingNextPage}
        >
          {isFetchingNextPage ? 'Loading...' : 'Load More'}
        </button>
      )}
    </div>
  )
}
```

## Common Patterns

### Virtualized Lists
For very long lists (>1000 items):

```tsx
import { useVirtualizer } from '@tanstack/react-virtual'

function VirtualTaskList({ tasks }: { tasks: Task[] }) {
  const parentRef = useRef<HTMLDivElement>(null)

  const virtualizer = useVirtualizer({
    count: tasks.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 100,  // Estimated item height
    overscan: 5  // Render 5 items outside viewport
  })

  return (
    <div ref={parentRef} className="h-screen overflow-auto">
      <div style={{ height: virtualizer.getTotalSize() }}>
        {virtualizer.getVirtualItems().map(virtualItem => (
          <div
            key={virtualItem.key}
            style={{
              position: 'absolute',
              top: 0,
              left: 0,
              width: '100%',
              height: virtualItem.size,
              transform: `translateY(${virtualItem.start}px)`
            }}
          >
            <TaskCard task={tasks[virtualItem.index]} />
          </div>
        ))}
      </div>
    </div>
  )
}
```

### Debounced Search
```tsx
import { useDeferredValue, useState } from 'react'

function TaskSearch() {
  const [searchTerm, setSearchTerm] = useState('')
  const deferredSearchTerm = useDeferredValue(searchTerm)

  // Query uses deferred value (delayed updates)
  const { data: tasks } = useTaskSearchQuery(deferredSearchTerm)

  return (
    <div>
      <input
        value={searchTerm}
        onChange={(e) => setSearchTerm(e.target.value)}
        placeholder="Search tasks..."
      />
      <TaskList tasks={tasks} />
    </div>
  )
}
```

### Transition for Non-Urgent Updates
```tsx
import { useTransition } from 'react'

function TaskList() {
  const [isPending, startTransition] = useTransition()
  const [filter, setFilter] = useState('all')

  const handleFilterChange = (newFilter: string) => {
    startTransition(() => {
      setFilter(newFilter)  // Non-urgent update
    })
  }

  return (
    <div>
      <FilterBar filter={filter} onChange={handleFilterChange} />
      {isPending ? <LoadingOverlay /> : <Tasks filter={filter} />}
    </div>
  )
}
```

## Anti-Patterns

### ❌ Premature useMemo/useCallback
```tsx
// BAD: useMemo for simple calculation
const doubled = useMemo(() => value * 2, [value])

// GOOD: Just calculate
const doubled = value * 2
```

### ❌ Memoizing Everything
```tsx
// BAD: Every component memoized
export const Text = memo(({ text }: { text: string }) => <span>{text}</span>)
export const Button = memo(({ onClick }: Props) => <button onClick={onClick} />)
export const Icon = memo(({ name }: { name: string }) => <i className={name} />)

// GOOD: Only memo expensive or frequently re-rendering components
```

### ❌ useCallback without React.memo
```tsx
// BAD: Callback memoized but child isn't
function Parent() {
  const handleClick = useCallback(() => { ... }, [])
  return <Child onClick={handleClick} />  // Child not memoized!
}

// GOOD: Either memo child or don't use useCallback
const MemoizedChild = memo(Child)
function Parent() {
  const handleClick = useCallback(() => { ... }, [])
  return <MemoizedChild onClick={handleClick} />
}
```

### ❌ Creating Objects in Render
```tsx
// BAD: New object every render
<TaskCard style={{ padding: 16 }} />  // Breaks memo!

// GOOD: Stable reference
const cardStyle = { padding: 16 }
<TaskCard style={cardStyle} />
```

### ❌ Inefficient Array Methods
```tsx
// BAD: Multiple iterations
const filtered = tasks.filter(t => t.status === 'active')
const sorted = filtered.sort((a, b) => a.title.localeCompare(b.title))
const mapped = sorted.map(t => ({ ...t, formatted: format(t) }))

// GOOD: Single iteration
const result = tasks
  .filter(t => t.status === 'active')
  .sort((a, b) => a.title.localeCompare(b.title))
  .map(t => ({ ...t, formatted: format(t) }))
```
