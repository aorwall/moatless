# Derived State Without Effects

## Overview
Calculate derived values during render instead of storing them in state or using effects. This is a critical React pattern that prevents bugs and improves performance.

## Core Principle

**If you can calculate a value from existing state/props, don't store it in state.**

## Common Pattern: Calculate During Render

### ✅ Correct - Calculate During Render
```tsx
function TaskList({ tasks, filter }: Props) {
  // Derived value - calculated during render
  const filteredTasks = tasks.filter(task =>
    filter === 'all' || task.status === filter
  )

  return (
    <div>
      {filteredTasks.map(task => (
        <TaskCard key={task.id} task={task} />
      ))}
    </div>
  )
}
```

### ❌ Wrong - Using Effect
```tsx
function TaskList({ tasks, filter }: Props) {
  const [filteredTasks, setFilteredTasks] = useState(tasks)

  // DON'T DO THIS - Effect for derived value
  useEffect(() => {
    setFilteredTasks(tasks.filter(task =>
      filter === 'all' || task.status === filter
    ))
  }, [tasks, filter])

  return (
    <div>
      {filteredTasks.map(task => (
        <TaskCard key={task.id} task={task} />
      ))}
    </div>
  )
}
```

**Why Wrong**:
- Extra state to manage
- Extra re-render (effect runs after render)
- Can get out of sync
- More complex code

## Examples

### Full Name
```tsx
// ✅ CORRECT
function UserProfile({ firstName, lastName }: Props) {
  const fullName = `${firstName} ${lastName}`
  return <h1>{fullName}</h1>
}

// ❌ WRONG
function UserProfile({ firstName, lastName }: Props) {
  const [fullName, setFullName] = useState('')

  useEffect(() => {
    setFullName(`${firstName} ${lastName}`)
  }, [firstName, lastName])

  return <h1>{fullName}</h1>
}
```

### Task Statistics
```tsx
// ✅ CORRECT
function TaskStats({ tasks }: { tasks: Task[] }) {
  const completed = tasks.filter(t => t.status === 'completed').length
  const pending = tasks.filter(t => t.status === 'pending').length
  const total = tasks.length
  const completionRate = total > 0 ? (completed / total) * 100 : 0

  return (
    <div>
      <span>Total: {total}</span>
      <span>Completed: {completed}</span>
      <span>Pending: {pending}</span>
      <span>Completion: {completionRate.toFixed(0)}%</span>
    </div>
  )
}

// ❌ WRONG
function TaskStats({ tasks }: { tasks: Task[] }) {
  const [stats, setStats] = useState({
    completed: 0,
    pending: 0,
    total: 0,
    completionRate: 0
  })

  useEffect(() => {
    const completed = tasks.filter(t => t.status === 'completed').length
    const pending = tasks.filter(t => t.status === 'pending').length
    const total = tasks.length
    const completionRate = total > 0 ? (completed / total) * 100 : 0

    setStats({ completed, pending, total, completionRate })
  }, [tasks])

  return (
    <div>
      <span>Total: {stats.total}</span>
      <span>Completed: {stats.completed}</span>
      <span>Pending: {stats.pending}</span>
      <span>Completion: {stats.completionRate.toFixed(0)}%</span>
    </div>
  )
}
```

### Filtered and Sorted List
```tsx
// ✅ CORRECT
function TaskList({ tasks, filter, sortBy }: Props) {
  // All derived values calculated during render
  const filteredTasks = tasks.filter(task =>
    filter === 'all' || task.status === filter
  )

  const sortedTasks = [...filteredTasks].sort((a, b) => {
    if (sortBy === 'title') return a.title.localeCompare(b.title)
    if (sortBy === 'date') return b.createdAt.getTime() - a.createdAt.getTime()
    return 0
  })

  return (
    <div>
      {sortedTasks.map(task => (
        <TaskCard key={task.id} task={task} />
      ))}
    </div>
  )
}
```

## When to Use useMemo

Use `useMemo` ONLY when:
1. Calculation is expensive (>50ms)
2. You've profiled and confirmed it's a bottleneck

### Without useMemo (Usually Fine)
```tsx
function TaskList({ tasks }: { tasks: Task[] }) {
  // Simple calculation - no useMemo needed
  const activeTasks = tasks.filter(t => t.status === 'active')

  return <div>{activeTasks.map(...)}</div>
}
```

### With useMemo (Only if Expensive)
```tsx
function TaskAnalytics({ tasks }: { tasks: Task[] }) {
  // Expensive calculation (complex statistical analysis)
  const statistics = useMemo(() => {
    return calculateComplexStatistics(tasks)  // >50ms operation
  }, [tasks])

  return <div>{/* Display statistics */}</div>
}
```

## Form Values

### ✅ Correct - Controlled Inputs with Derived Validation
```tsx
function TaskForm() {
  const [title, setTitle] = useState('')
  const [description, setDescription] = useState('')

  // Derived validation state
  const isTitleValid = title.length >= 3
  const isDescriptionValid = description.length <= 500
  const isFormValid = isTitleValid && isDescriptionValid

  return (
    <form>
      <input
        value={title}
        onChange={(e) => setTitle(e.target.value)}
      />
      {!isTitleValid && <span>Title must be at least 3 characters</span>}

      <textarea
        value={description}
        onChange={(e) => setDescription(e.target.value)}
      />
      {!isDescriptionValid && <span>Description too long</span>}

      <button disabled={!isFormValid}>Submit</button>
    </form>
  )
}
```

### ❌ Wrong - Storing Validation in State
```tsx
function TaskForm() {
  const [title, setTitle] = useState('')
  const [description, setDescription] = useState('')
  const [isTitleValid, setIsTitleValid] = useState(false)
  const [isDescriptionValid, setIsDescriptionValid] = useState(true)
  const [isFormValid, setIsFormValid] = useState(false)

  useEffect(() => {
    setIsTitleValid(title.length >= 3)
  }, [title])

  useEffect(() => {
    setIsDescriptionValid(description.length <= 500)
  }, [description])

  useEffect(() => {
    setIsFormValid(isTitleValid && isDescriptionValid)
  }, [isTitleValid, isDescriptionValid])

  // ... rest of form
}
```

## Search with Derived Results

### ✅ Correct - Calculate Filtered Results
```tsx
function TaskSearch() {
  const { data: tasks } = useTasksQuery()
  const [searchTerm, setSearchTerm] = useState('')

  // Derived search results
  const searchResults = searchTerm.trim()
    ? tasks.filter(task =>
        task.title.toLowerCase().includes(searchTerm.toLowerCase())
      )
    : tasks

  return (
    <div>
      <input
        value={searchTerm}
        onChange={(e) => setSearchTerm(e.target.value)}
        placeholder="Search tasks..."
      />
      <div>{searchResults.map(task => <TaskCard key={task.id} task={task} />)}</div>
    </div>
  )
}
```

## Complex Derived State

For complex calculations, extract to a custom hook:

```tsx
// Custom hook for derived task statistics
function useTaskStatistics(tasks: Task[]) {
  return useMemo(() => {
    const byStatus = {
      pending: tasks.filter(t => t.status === 'pending').length,
      running: tasks.filter(t => t.status === 'running').length,
      completed: tasks.filter(t => t.status === 'completed').length,
      failed: tasks.filter(t => t.status === 'failed').length
    }

    const byPriority = {
      low: tasks.filter(t => t.priority === 'low').length,
      medium: tasks.filter(t => t.priority === 'medium').length,
      high: tasks.filter(t => t.priority === 'high').length
    }

    const averageCompletionTime = calculateAverageTime(
      tasks.filter(t => t.status === 'completed')
    )

    return { byStatus, byPriority, averageCompletionTime }
  }, [tasks])
}

// Usage
function TaskDashboard() {
  const { data: tasks } = useTasksQuery()
  const statistics = useTaskStatistics(tasks)

  return (
    <div>
      <h2>Task Statistics</h2>
      <StatusChart data={statistics.byStatus} />
      <PriorityChart data={statistics.byPriority} />
      <span>Avg Time: {statistics.averageCompletionTime}ms</span>
    </div>
  )
}
```

## Anti-Patterns

### ❌ Effect for Simple Calculation
```tsx
// WRONG
const [total, setTotal] = useState(0)
useEffect(() => {
  setTotal(items.reduce((sum, item) => sum + item.price, 0))
}, [items])

// CORRECT
const total = items.reduce((sum, item) => sum + item.price, 0)
```

### ❌ Effect Chain
```tsx
// WRONG - Effect depending on another effect
const [filtered, setFiltered] = useState([])
const [sorted, setSorted] = useState([])

useEffect(() => {
  setFiltered(tasks.filter(t => t.status === filter))
}, [tasks, filter])

useEffect(() => {
  setSorted([...filtered].sort((a, b) => a.title.localeCompare(b.title)))
}, [filtered])

// CORRECT - Calculate in one go
const sorted = tasks
  .filter(t => t.status === filter)
  .sort((a, b) => a.title.localeCompare(b.title))
```

### ❌ Storing What Can Be Calculated
```tsx
// WRONG
const [user, setUser] = useState<User>()
const [userName, setUserName] = useState('')

useEffect(() => {
  if (user) {
    setUserName(`${user.firstName} ${user.lastName}`)
  }
}, [user])

// CORRECT
const [user, setUser] = useState<User>()
const userName = user ? `${user.firstName} ${user.lastName}` : ''
```

## When Effects ARE Needed

Effects are for side effects, not derived values:

### ✅ Correct Use of Effects
```tsx
// Fetching data
useEffect(() => {
  fetchTasks().then(setTasks)
}, [])

// Setting up subscriptions
useEffect(() => {
  const unsubscribe = eventBus.on('task:updated', handleUpdate)
  return unsubscribe
}, [])

// Syncing with external system
useEffect(() => {
  localStorage.setItem('theme', theme)
}, [theme])

// Setting document title
useEffect(() => {
  document.title = `${taskCount} tasks - App`
}, [taskCount])
```

## Summary

**Calculate during render**: If value can be computed from state/props
**No effects for derived values**: Effects are for side effects only
**Use useMemo sparingly**: Only for expensive calculations (>50ms)
**Extract complex logic**: Use custom hooks for readability
**Test by profiling**: Don't optimize prematurely
