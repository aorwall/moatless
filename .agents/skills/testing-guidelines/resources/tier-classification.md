# Tier Classification System

## Overview
Not all code needs the same test coverage. Use this tier system to prioritize testing efforts.

## Tier 1: Essential (80-90% Coverage)

### What Goes Here
- Complex state machines
- Critical business logic
- Data transformations
- Security-sensitive code
- Payment/billing logic
- User authentication flows

### Examples
```tsx
// Tier 1: Complex state machine
function useTaskStateMachine() {
  const [status, setStatus] = useState<TaskStatus>('idle')
  const [error, setError] = useState<Error | null>(null)

  const start = () => {
    if (status !== 'idle') throw new Error('Invalid transition')
    setStatus('running')
  }

  const complete = () => {
    if (status !== 'running') throw new Error('Invalid transition')
    setStatus('completed')
  }

  const fail = (error: Error) => {
    if (status !== 'running') throw new Error('Invalid transition')
    setStatus('failed')
    setError(error)
  }

  const reset = () => {
    setStatus('idle')
    setError(null)
  }

  return { status, error, start, complete, fail, reset }
}
```

**Why Tier 1**: Complex state transitions, multiple code paths, critical functionality.

## Tier 2: Moderate (50-60% Coverage)

### What Goes Here
- Conditional rendering logic
- Data fetching components
- Form handling
- Error states
- Loading states
- User interaction handlers

### Examples
```tsx
// Tier 2: Conditional rendering
function TaskCard({ task }: { task: Task }) {
  if (!task) {
    return <EmptyState message="Task not found" />
  }

  if (task.status === 'running') {
    return <RunningTaskCard task={task} />
  }

  if (task.status === 'completed') {
    return <CompletedTaskCard task={task} />
  }

  return <PendingTaskCard task={task} />
}
```

**Why Tier 2**: Multiple code paths but simpler logic than Tier 1.

## Tier 3: Skip (0% Coverage)

### What Goes Here
- Simple presentational components
- Static layouts
- Pure CSS components
- Trivial getters/setters
- Simple props pass-through

### Examples
```tsx
// Tier 3: Simple presentational component
function TaskTitle({ title }: { title: string }) {
  return <h1 className="text-2xl font-bold">{title}</h1>
}

// Tier 3: Props pass-through
function PageLayout({ children }: { children: ReactNode }) {
  return (
    <div className="container mx-auto">
      <Header />
      <main className="p-4">{children}</main>
      <Footer />
    </div>
  )
}
```

**Why Skip**: No logic to test, just rendering props or children.

## Classification Decision Tree

```
Is it complex business logic or state machine?
├─ YES → Tier 1 (80-90% coverage)
└─ NO ↓

Does it have conditional rendering or data fetching?
├─ YES → Tier 2 (50-60% coverage)
└─ NO ↓

Is it just displaying props or static content?
└─ YES → Tier 3 (0% coverage - skip)
```

## Quick Classification Guide

| Characteristic | Tier 1 | Tier 2 | Tier 3 |
|---------------|--------|--------|--------|
| Complexity | High | Medium | Low |
| Code paths | Many | Some | One |
| Business logic | Yes | Some | No |
| Edge cases | Many | Few | None |
| User impact | Critical | Important | Minor |
| Test priority | Must test | Should test | Skip |

## Real-World Examples

```tsx
// Tier 1: Complex form with validation
function TaskForm() {
  const [data, setData] = useState(initialData)
  const [errors, setErrors] = useState({})
  const [step, setStep] = useState(1)

  const validate = () => {
    // Complex validation logic
  }

  const handleSubmit = async () => {
    // Multi-step submission
  }

  // ... complex logic
}

// Tier 2: Data fetching component
function TaskList() {
  const { data, isLoading, error } = useTasksQuery()

  if (isLoading) return <Loading />
  if (error) return <Error error={error} />

  return <div>{data.map(task => <TaskCard key={task.id} task={task} />)}</div>
}

// Tier 3: Presentational component
function TaskBadge({ status }: { status: string }) {
  return <span className="badge">{status}</span>
}
```

## When in Doubt

If you're unsure which tier:
1. Start with Tier 2 (moderate coverage)
2. If tests reveal complexity → upgrade to Tier 1
3. If tests feel pointless → downgrade to Tier 3
