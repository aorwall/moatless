# Component Patterns

## Table of Contents
- [React 19 Component Structure](#react-19-component-structure)
- [Component Types](#component-types)
- [Single Responsibility Principle](#single-responsibility-principle)
- [Prop Design](#prop-design)
- [Composition Patterns](#composition-patterns)
- [State Management](#state-management)
- [Common Patterns](#common-patterns)
- [Anti-Patterns](#anti-patterns)

## React 19 Component Structure

### Functional Component Pattern
All components use functional components with TypeScript:

```tsx
import { useState } from 'react'

interface TaskCardProps {
  id: string
  title: string
  status: 'pending' | 'running' | 'completed'
  onStatusChange?: (status: string) => void
}

export function TaskCard({ id, title, status, onStatusChange }: TaskCardProps) {
  const [isExpanded, setIsExpanded] = useState(false)

  return (
    <div className="border rounded-lg p-4">
      <h3 className="text-lg font-semibold">{title}</h3>
      <span className="text-sm text-gray-600">{status}</span>
    </div>
  )
}
```

### Key Principles
- Use named exports (not default exports)
- Define interface above component
- Use descriptive prop names
- Co-locate related components in same file when appropriate

## Component Types

### 1. Feature Components
Located in `src/features/[feature]/components/`

**Purpose**: Feature-specific UI logic

```tsx
// src/features/tasks/components/task-list.tsx
import { useTasksQuery } from '../api/use-tasks-query'
import { TaskCard } from './task-card'

export function TaskList() {
  const { data: tasks, isLoading, error } = useTasksQuery()

  if (isLoading) return <LoadingSpinner />
  if (error) return <ErrorDisplay error={error} />

  return (
    <div className="space-y-4">
      {tasks.map(task => (
        <TaskCard key={task.id} {...task} />
      ))}
    </div>
  )
}
```

### 2. Shared Components
Located in `src/lib/components/`

**Purpose**: Reusable across features

```tsx
// src/lib/components/loading-spinner.tsx
export function LoadingSpinner({ size = 'md' }: { size?: 'sm' | 'md' | 'lg' }) {
  const sizeClasses = {
    sm: 'w-4 h-4',
    md: 'w-8 h-8',
    lg: 'w-12 h-12'
  }

  return (
    <div className={`animate-spin rounded-full border-2 ${sizeClasses[size]}`} />
  )
}
```

### 3. UI Components (shadcn/ui)
Located in `src/lib/components/ui/`

**Purpose**: Base UI primitives from shadcn/ui

```bash
# Install shadcn/ui component
cd apps/frontend && bunx shadcn@latest add button
```

### 4. Page Components
Located in `src/routes/`

**Purpose**: Route-level components that orchestrate features

```tsx
// src/routes/tasks/$taskId.tsx
import { createFileRoute } from '@tanstack/react-router'
import { TaskDetail } from '@/features/tasks/components/task-detail'

export const Route = createFileRoute('/tasks/$taskId')({
  component: TaskDetailPage
})

function TaskDetailPage() {
  const { taskId } = Route.useParams()

  return (
    <div className="container mx-auto p-4">
      <TaskDetail taskId={taskId} />
    </div>
  )
}
```

## Single Responsibility Principle

### Rule: Describable in One Sentence
Every component should have ONE clear purpose describable in a single sentence.

**Good Example**:
```tsx
// Purpose: "Displays a task's title and status"
export function TaskCard({ title, status }: TaskCardProps) {
  return (
    <div>
      <h3>{title}</h3>
      <span>{status}</span>
    </div>
  )
}
```

**Bad Example**:
```tsx
// Too many responsibilities!
export function TaskManagement() {
  // Fetching, filtering, sorting, displaying, editing, deleting...
  // Split this into multiple components!
}
```

### When to Split Components

**Split if the component**:
- Has multiple distinct responsibilities
- Is >200 lines
- Has complex conditional rendering logic
- Mixes different levels of abstraction

**Example - Before Split**:
```tsx
// TOO COMPLEX - does everything
export function TaskList() {
  const [tasks, setTasks] = useState([])
  const [filter, setFilter] = useState('all')
  const [sortBy, setSortBy] = useState('date')

  // Fetching logic
  useEffect(() => { ... }, [])

  // Filtering logic
  const filtered = tasks.filter(...)

  // Sorting logic
  const sorted = filtered.sort(...)

  return (
    <div>
      {/* Filter UI */}
      {/* Sort UI */}
      {/* Task list UI */}
    </div>
  )
}
```

**Example - After Split**:
```tsx
// Main component - orchestrates child components
export function TaskList() {
  const { data: tasks } = useTasksQuery()
  const [filter, setFilter] = useState('all')
  const [sortBy, setSortBy] = useState('date')

  const filteredTasks = useFilteredTasks(tasks, filter)
  const sortedTasks = useSortedTasks(filteredTasks, sortBy)

  return (
    <div>
      <TaskFilters filter={filter} onFilterChange={setFilter} />
      <TaskSort sortBy={sortBy} onSortChange={setSortBy} />
      <TaskItems tasks={sortedTasks} />
    </div>
  )
}

// Separate components with single responsibilities
function TaskFilters({ filter, onFilterChange }) { ... }
function TaskSort({ sortBy, onSortChange }) { ... }
function TaskItems({ tasks }) { ... }
```

## Prop Design

### Prefer Primitives Over Objects
Pass primitives (strings, numbers, booleans) instead of entire objects when possible.

**Good**:
```tsx
interface TaskTitleProps {
  title: string
  isCompleted: boolean
}

function TaskTitle({ title, isCompleted }: TaskTitleProps) {
  return <h3 className={isCompleted ? 'line-through' : ''}>{title}</h3>
}

// Usage
<TaskTitle title={task.title} isCompleted={task.isCompleted} />
```

**Bad**:
```tsx
interface TaskTitleProps {
  task: Task  // Entire object!
}

function TaskTitle({ task }: TaskTitleProps) {
  return <h3 className={task.isCompleted ? 'line-through' : ''}>{task.title}</h3>
}

// Usage
<TaskTitle task={task} />
```

### Exceptions to Primitive Props Rule

**Pass objects when**:
1. **5+ related props** - Too many primitives become unwieldy
2. **Complex nested data** - Like form validation errors
3. **Opaque data** - Component just passes it through

```tsx
// GOOD - 5+ props, pass object instead
interface UserProfileProps {
  user: {
    name: string
    email: string
    avatar: string
    role: string
    lastLogin: Date
    preferences: Record<string, unknown>
  }
}
```

### Optional Props and Handlers
Use `?` for optional props, especially event handlers:

```tsx
interface ButtonProps {
  label: string
  onClick?: () => void
  disabled?: boolean
  variant?: 'primary' | 'secondary'
}
```

### Children Prop
Use `children` for composition:

```tsx
interface CardProps {
  children: React.ReactNode
  className?: string
}

export function Card({ children, className }: CardProps) {
  return (
    <div className={`border rounded-lg p-4 ${className || ''}`}>
      {children}
    </div>
  )
}
```

## Composition Patterns

### Compound Components
For components with related sub-components:

```tsx
// Card.tsx
function CardRoot({ children, className }: CardProps) {
  return <div className={`card ${className || ''}`}>{children}</div>
}

function CardHeader({ children }: { children: React.ReactNode }) {
  return <div className="card-header">{children}</div>
}

function CardBody({ children }: { children: React.ReactNode }) {
  return <div className="card-body">{children}</div>
}

function CardFooter({ children }: { children: React.ReactNode }) {
  return <div className="card-footer">{children}</div>
}

// Export as compound component
export const Card = Object.assign(CardRoot, {
  Header: CardHeader,
  Body: CardBody,
  Footer: CardFooter
})

// Usage
<Card>
  <Card.Header>
    <h2>Task Details</h2>
  </Card.Header>
  <Card.Body>
    <p>Task content...</p>
  </Card.Body>
  <Card.Footer>
    <button>Close</button>
  </Card.Footer>
</Card>
```

### Render Props
For sharing logic with flexible rendering:

```tsx
interface DataFetcherProps<T> {
  queryKey: string[]
  children: (data: T, isLoading: boolean, error: Error | null) => React.ReactNode
}

function DataFetcher<T>({ queryKey, children }: DataFetcherProps<T>) {
  const { data, isLoading, error } = useQuery<T>({ queryKey })
  return <>{children(data, isLoading, error)}</>
}

// Usage
<DataFetcher<Task[]> queryKey={['tasks']}>
  {(tasks, isLoading, error) => {
    if (isLoading) return <Loading />
    if (error) return <Error />
    return <TaskList tasks={tasks} />
  }}
</DataFetcher>
```

## State Management

### Co-locate State
Keep state as close as possible to where it's used:

**Good**:
```tsx
function TaskCard({ id, title }: TaskCardProps) {
  // State only used in this component
  const [isExpanded, setIsExpanded] = useState(false)

  return (
    <div>
      <button onClick={() => setIsExpanded(!isExpanded)}>
        {isExpanded ? 'Collapse' : 'Expand'}
      </button>
      {isExpanded && <TaskDetails id={id} />}
    </div>
  )
}
```

**Bad**:
```tsx
// State lifted unnecessarily high
function TaskList() {
  const [expandedTasks, setExpandedTasks] = useState<Set<string>>(new Set())

  return tasks.map(task => (
    <TaskCard
      {...task}
      isExpanded={expandedTasks.has(task.id)}
      onToggle={() => {/* complex logic */}}
    />
  ))
}
```

### Lift State When Needed
Lift state only when siblings need to communicate:

```tsx
function TaskManagement() {
  // Shared state between TaskList and TaskStats
  const [selectedTaskIds, setSelectedTaskIds] = useState<string[]>([])

  return (
    <>
      <TaskStats selectedCount={selectedTaskIds.length} />
      <TaskList
        selectedTaskIds={selectedTaskIds}
        onSelectionChange={setSelectedTaskIds}
      />
    </>
  )
}
```

### Avoid Derived State
Calculate derived values during render instead of storing in state:

**Good**:
```tsx
function TaskList({ tasks }: { tasks: Task[] }) {
  const [filter, setFilter] = useState('all')

  // Derived value - calculated during render
  const filteredTasks = tasks.filter(task =>
    filter === 'all' || task.status === filter
  )

  return <div>{filteredTasks.map(...)}</div>
}
```

**Bad**:
```tsx
function TaskList({ tasks }: { tasks: Task[] }) {
  const [filter, setFilter] = useState('all')
  const [filteredTasks, setFilteredTasks] = useState(tasks)

  // DON'T DO THIS - use derived value instead
  useEffect(() => {
    setFilteredTasks(tasks.filter(task =>
      filter === 'all' || task.status === filter
    ))
  }, [tasks, filter])

  return <div>{filteredTasks.map(...)}</div>
}
```

## Common Patterns

### Loading and Error States
Always handle loading and error states:

```tsx
function TaskDetail({ taskId }: { taskId: string }) {
  const { data: task, isLoading, error } = useTaskQuery(taskId)

  if (isLoading) {
    return <LoadingSpinner />
  }

  if (error) {
    return <ErrorDisplay error={error} retry={() => refetch()} />
  }

  if (!task) {
    return <EmptyState message="Task not found" />
  }

  return <div>{/* Render task */}</div>
}
```

### Conditional Rendering
Use early returns for cleaner code:

**Good**:
```tsx
function UserProfile({ userId }: { userId: string }) {
  const { data: user, isLoading } = useUserQuery(userId)

  if (isLoading) return <Loading />
  if (!user) return <NotFound />

  return (
    <div>
      <h1>{user.name}</h1>
      <p>{user.email}</p>
    </div>
  )
}
```

**Avoid**:
```tsx
function UserProfile({ userId }: { userId: string }) {
  const { data: user, isLoading } = useUserQuery(userId)

  return (
    <div>
      {isLoading ? (
        <Loading />
      ) : user ? (
        <>
          <h1>{user.name}</h1>
          <p>{user.email}</p>
        </>
      ) : (
        <NotFound />
      )}
    </div>
  )
}
```

### Event Handlers
Name handlers with `handle` prefix:

```tsx
function TaskForm() {
  const [title, setTitle] = useState('')

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    // Submit logic
  }

  const handleTitleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setTitle(e.target.value)
  }

  return (
    <form onSubmit={handleSubmit}>
      <input value={title} onChange={handleTitleChange} />
      <button type="submit">Submit</button>
    </form>
  )
}
```

## Anti-Patterns

### ❌ Using Default Exports
```tsx
// WRONG
export default function TaskCard() { ... }

// CORRECT
export function TaskCard() { ... }
```

### ❌ Using `any` Type
```tsx
// WRONG
function processData(data: any) { ... }

// CORRECT
function processData(data: TaskData) { ... }
// or if truly unknown
function processData(data: unknown) { ... }
```

### ❌ Duplicating Server State
```tsx
// WRONG
function TaskList() {
  const { data: tasks } = useQuery({ queryKey: ['tasks'] })
  const [localTasks, setLocalTasks] = useState(tasks) // DON'T!

  return <div>{localTasks.map(...)}</div>
}

// CORRECT
function TaskList() {
  const { data: tasks } = useQuery({ queryKey: ['tasks'] })
  return <div>{tasks.map(...)}</div>
}
```

### ❌ Premature Abstraction
Wait for 3+ use cases before abstracting:

```tsx
// WRONG - Only used once
function Button({ variant, size, onClick, children }) {
  // Complex abstraction for single use
}

// CORRECT - Keep it simple until reused
<button className="px-4 py-2 bg-blue-500" onClick={...}>
  Click me
</button>
```
