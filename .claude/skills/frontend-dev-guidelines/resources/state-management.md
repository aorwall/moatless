# State Management

## Overview
React state management follows principles of co-location, lifting when necessary, and avoiding prop drilling with context.

## Core Principles

### 1. Co-locate State
Keep state as close as possible to where it's used.

**✅ Good - Co-located**:
```tsx
function TaskCard({ task }: { task: Task }) {
  // State only used in this component
  const [isExpanded, setIsExpanded] = useState(false)

  return (
    <div>
      <button onClick={() => setIsExpanded(!isExpanded)}>
        {isExpanded ? 'Collapse' : 'Expand'}
      </button>
      {isExpanded && <TaskDetails taskId={task.id} />}
    </div>
  )
}
```

**❌ Bad - Lifted Too High**:
```tsx
function TaskList({ tasks }: { tasks: Task[] }) {
  // State lifted unnecessarily - only TaskCard needs this
  const [expandedTasks, setExpandedTasks] = useState<Set<string>>(new Set())

  return (
    <div>
      {tasks.map(task => (
        <TaskCard
          key={task.id}
          task={task}
          isExpanded={expandedTasks.has(task.id)}
          onToggle={() => {
            const newSet = new Set(expandedTasks)
            if (newSet.has(task.id)) {
              newSet.delete(task.id)
            } else {
              newSet.add(task.id)
            }
            setExpandedTasks(newSet)
          }}
        />
      ))}
    </div>
  )
}
```

### 2. Lift State When Needed
Lift state only when siblings need to share it.

**When to Lift**:
- Two or more siblings need the same state
- Parent needs to coordinate children
- State needs to survive child unmount

```tsx
function TaskManagement() {
  // Lifted because both children need this state
  const [selectedTaskIds, setSelectedTaskIds] = useState<string[]>([])

  return (
    <div>
      {/* Both components use selectedTaskIds */}
      <TaskStats selectedCount={selectedTaskIds.length} />
      <TaskList
        selectedTaskIds={selectedTaskIds}
        onSelectionChange={setSelectedTaskIds}
      />
    </div>
  )
}
```

### 3. Avoid Prop Drilling with Context
Use Context for data needed by many descendants at different levels.

**When to Use Context**:
- Data needed by many components at different nesting levels
- Theme, authentication, locale
- Feature-specific context (not global)

**When NOT to Use Context**:
- Data only needed by 1-2 components (use props)
- Frequently changing data (use state or React Query)
- Data that can be derived (calculate instead)

## Context Patterns

### Basic Context Setup
```tsx
// src/features/tasks/contexts/task-context.tsx
import { createContext, useContext, useState } from 'react'

interface TaskContextValue {
  selectedTaskIds: string[]
  selectTask: (id: string) => void
  deselectTask: (id: string) => void
  clearSelection: () => void
}

const TaskContext = createContext<TaskContextValue | null>(null)

export function TaskProvider({ children }: { children: React.ReactNode }) {
  const [selectedTaskIds, setSelectedTaskIds] = useState<string[]>([])

  const selectTask = (id: string) => {
    setSelectedTaskIds(prev => [...prev, id])
  }

  const deselectTask = (id: string) => {
    setSelectedTaskIds(prev => prev.filter(taskId => taskId !== id))
  }

  const clearSelection = () => {
    setSelectedTaskIds([])
  }

  return (
    <TaskContext.Provider
      value={{ selectedTaskIds, selectTask, deselectTask, clearSelection }}
    >
      {children}
    </TaskContext.Provider>
  )
}

export function useTaskContext() {
  const context = useContext(TaskContext)
  if (!context) {
    throw new Error('useTaskContext must be used within TaskProvider')
  }
  return context
}
```

### Using Context
```tsx
// Wrap feature with provider
function TasksPage() {
  return (
    <TaskProvider>
      <TaskList />
      <TaskActions />
    </TaskProvider>
  )
}

// Use context in descendants
function TaskCard({ task }: { task: Task }) {
  const { selectedTaskIds, selectTask, deselectTask } = useTaskContext()
  const isSelected = selectedTaskIds.includes(task.id)

  return (
    <div onClick={() => isSelected ? deselectTask(task.id) : selectTask(task.id)}>
      {task.title}
    </div>
  )
}

function TaskActions() {
  const { selectedTaskIds, clearSelection } = useTaskContext()

  return (
    <div>
      <span>{selectedTaskIds.length} selected</span>
      <button onClick={clearSelection}>Clear</button>
    </div>
  )
}
```

### Context Splitting
Split contexts to avoid unnecessary re-renders.

**❌ Bad - Single Context**:
```tsx
// Everything re-renders when any value changes
interface AppContextValue {
  user: User | null
  theme: 'light' | 'dark'
  notifications: Notification[]
  settings: Settings
}
```

**✅ Good - Split Contexts**:
```tsx
// Only components using each context re-render
function App() {
  return (
    <UserProvider>
      <ThemeProvider>
        <NotificationProvider>
          <SettingsProvider>
            <Routes />
          </SettingsProvider>
        </NotificationProvider>
      </ThemeProvider>
    </UserProvider>
  )
}
```

## Server State vs Client State

### Use TanStack Query for Server State
```tsx
// ❌ DON'T duplicate server state in React state
function TaskList() {
  const { data: tasks } = useTasksQuery()
  const [localTasks, setLocalTasks] = useState(tasks)  // DON'T!

  return <div>{localTasks.map(...)}</div>
}

// ✅ Use server state directly
function TaskList() {
  const { data: tasks } = useTasksQuery()
  return <div>{tasks.map(...)}</div>
}
```

### Use React State for Client-Only State
```tsx
// ✅ Client-only UI state
function TaskCard({ task }: { task: Task }) {
  const [isExpanded, setIsExpanded] = useState(false)  // UI state only
  const [isHovered, setIsHovered] = useState(false)    // UI state only

  return (
    <div
      onMouseEnter={() => setIsHovered(true)}
      onMouseLeave={() => setIsHovered(false)}
    >
      {/* ... */}
    </div>
  )
}
```

## State Organization Patterns

### Reducer for Complex State
Use `useReducer` for complex state logic:

```tsx
type TaskFilters = {
  status: TaskStatus | 'all'
  priority: Priority | 'all'
  assignee: string | 'all'
  searchTerm: string
}

type FilterAction =
  | { type: 'SET_STATUS'; status: TaskStatus | 'all' }
  | { type: 'SET_PRIORITY'; priority: Priority | 'all' }
  | { type: 'SET_ASSIGNEE'; assignee: string | 'all' }
  | { type: 'SET_SEARCH'; searchTerm: string }
  | { type: 'RESET' }

function filterReducer(state: TaskFilters, action: FilterAction): TaskFilters {
  switch (action.type) {
    case 'SET_STATUS':
      return { ...state, status: action.status }
    case 'SET_PRIORITY':
      return { ...state, priority: action.priority }
    case 'SET_ASSIGNEE':
      return { ...state, assignee: action.assignee }
    case 'SET_SEARCH':
      return { ...state, searchTerm: action.searchTerm }
    case 'RESET':
      return initialFilters
    default:
      return state
  }
}

function TaskList() {
  const [filters, dispatch] = useReducer(filterReducer, initialFilters)

  return (
    <div>
      <select onChange={(e) => dispatch({ type: 'SET_STATUS', status: e.target.value })}>
        {/* ... */}
      </select>
      <button onClick={() => dispatch({ type: 'RESET' })}>Reset</button>
    </div>
  )
}
```

### State Machines
For complex workflows with valid state transitions:

```tsx
type TaskState = 'idle' | 'loading' | 'success' | 'error'

function useTaskStateMachine() {
  const [state, setState] = useState<TaskState>('idle')
  const [data, setData] = useState<Task | null>(null)
  const [error, setError] = useState<Error | null>(null)

  const loadTask = async (id: string) => {
    if (state !== 'idle') return  // Prevent invalid transitions

    setState('loading')
    try {
      const task = await fetchTask(id)
      setData(task)
      setState('success')
    } catch (err) {
      setError(err as Error)
      setState('error')
    }
  }

  const reset = () => {
    setState('idle')
    setData(null)
    setError(null)
  }

  return { state, data, error, loadTask, reset }
}
```

## Form State

### Controlled Components
```tsx
function TaskForm() {
  const [title, setTitle] = useState('')
  const [description, setDescription] = useState('')

  return (
    <form>
      <input
        value={title}
        onChange={(e) => setTitle(e.target.value)}
      />
      <textarea
        value={description}
        onChange={(e) => setDescription(e.target.value)}
      />
    </form>
  )
}
```

### Form Libraries
For complex forms, use react-hook-form:

```tsx
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { z } from 'zod'

const schema = z.object({
  title: z.string().min(1),
  description: z.string().optional()
})

function TaskForm() {
  const { register, handleSubmit, formState: { errors } } = useForm({
    resolver: zodResolver(schema)
  })

  return (
    <form onSubmit={handleSubmit(onSubmit)}>
      <input {...register('title')} />
      {errors.title && <span>{errors.title.message}</span>}
    </form>
  )
}
```

## Anti-Patterns

### ❌ Duplicating Server State
```tsx
// WRONG
const { data: tasks } = useTasksQuery()
const [localTasks, setLocalTasks] = useState(tasks)
```

### ❌ Lifting State Too Early
```tsx
// WRONG - Lifted before needed by siblings
function Parent() {
  const [value, setValue] = useState('')  // Only Child uses this!
  return <Child value={value} onChange={setValue} />
}
```

### ❌ Prop Drilling Through Many Levels
```tsx
// WRONG - Passing through 5 levels
<GrandParent theme={theme}>
  <Parent theme={theme}>
    <Child theme={theme}>
      <GrandChild theme={theme}>
        <GreatGrandChild theme={theme} />  {/* Finally uses it */}
      </GrandChild>
    </Child>
  </Parent>
</GrandParent>

// CORRECT - Use Context
<ThemeProvider theme={theme}>
  <GrandParent>
    {/* ... */}
    <GreatGrandChild />  {/* Uses useTheme() hook */}
  </GrandParent>
</ThemeProvider>
```

### ❌ Using Context for Everything
```tsx
// WRONG - Context for simple parent-child communication
function Parent() {
  return (
    <ValueProvider value="hello">
      <Child />
    </ValueProvider>
  )
}

// CORRECT - Just use props
function Parent() {
  return <Child value="hello" />
}
```

## Summary

**State Co-location**: Keep state close to where it's used
**Lift When Needed**: Only when siblings share state
**Context for Depth**: Avoid prop drilling through many levels
**Split Contexts**: Prevent unnecessary re-renders
**Server State**: Use TanStack Query, not React state
**Complex State**: Use `useReducer` or state machines
