# TypeScript Patterns

## Overview
TypeScript best practices for the Moatless Vibe project, covering naming conventions, module patterns, and syntax rules.

## Naming Conventions

### Types and Interfaces
Use **UpperCamelCase** for types and interfaces:

```tsx
// ✅ Correct
interface TaskData { }
type UserRole = 'admin' | 'user'
class TaskService { }

// ❌ Wrong
interface taskData { }
type user_role = 'admin' | 'user'
class task_service { }
```

### Variables and Functions
Use **camelCase** for variables and functions:

```tsx
// ✅ Correct
const taskCount = 10
function getUserName() { }
const handleTaskClick = () => { }

// ❌ Wrong
const TaskCount = 10
function GetUserName() { }
const HandleTaskClick = () => { }
```

### Constants
Use **camelCase** for constants (not SCREAMING_SNAKE_CASE):

```tsx
// ✅ Correct
const maxRetries = 3
const defaultTimeout = 5000

// ❌ Wrong
const MAX_RETRIES = 3
const DEFAULT_TIMEOUT = 5000
```

### Type Parameter Names
Use descriptive names, not single letters:

```tsx
// ✅ Correct
function mapArray<Item, Result>(items: Item[], fn: (item: Item) => Result): Result[] {
  return items.map(fn)
}

// ❌ Wrong (unless truly generic)
function mapArray<T, U>(items: T[], fn: (item: T) => U): U[] {
  return items.map(fn)
}

// ✅ Single letter OK for truly generic utilities
function identity<T>(value: T): T {
  return value
}
```

## Type Safety Rules

### Never Use `any`
```tsx
// ❌ NEVER do this
function processData(data: any) { }
const result: any = fetchData()

// ✅ Use proper types
function processData(data: TaskData) { }
const result: Task = fetchData()

// ✅ If type is truly unknown, use `unknown`
function processData(data: unknown) {
  if (typeof data === 'object' && data !== null) {
    // Type guard to narrow unknown
  }
}
```

### Use Type Guards
```tsx
function isTask(value: unknown): value is Task {
  return (
    typeof value === 'object' &&
    value !== null &&
    'id' in value &&
    'title' in value
  )
}

function processValue(value: unknown) {
  if (isTask(value)) {
    // TypeScript knows value is Task here
    console.log(value.title)
  }
}
```

### Avoid Type Assertions
```tsx
// ❌ Avoid type assertions
const task = data as Task

// ✅ Use type guards instead
if (isTask(data)) {
  const task = data  // TypeScript infers Task
}

// ✅ OK for DOM elements
const input = document.querySelector('#input') as HTMLInputElement
```

## Module Patterns

### Named Exports Only
**NEVER use default exports**:

```tsx
// ❌ WRONG - Default export
export default function TaskCard() { }

// ✅ CORRECT - Named export
export function TaskCard() { }

// ❌ WRONG - Default export
export default TaskService

// ✅ CORRECT - Named export
export { TaskService }
```

**Why**: Named exports provide better refactoring, IDE support, and prevent naming inconsistencies.

### Never Use `import type`
```tsx
// ❌ WRONG
import type { Task } from '@moatless/types'

// ✅ CORRECT
import { Task } from '@moatless/types'
```

**Why**: The TypeScript compiler strips types automatically. `import type` is unnecessary and creates confusion.

### Import Organization
Organize imports in this order:

```tsx
// 1. External libraries
import { useState, useEffect } from 'react'
import { useQuery } from '@tanstack/react-query'

// 2. Shared packages
import { Task } from '@moatless/types'
import { routes, buildPath } from '@moatless/api-contract'

// 3. Internal absolute imports
import { apiClient } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'

// 4. Relative imports
import { TaskCard } from './task-card'
import { useTaskForm } from '../hooks/use-task-form'
```

### Barrel Exports
Use barrel exports (`index.ts`) for clean imports:

```tsx
// src/features/tasks/index.ts
export { TaskList } from './components/task-list'
export { TaskCard } from './components/task-card'
export { useTasksQuery } from './api/use-tasks-query'
export { useCreateTaskMutation } from './api/use-create-task-mutation'

// Usage elsewhere
import { TaskList, useTasksQuery } from '@/features/tasks'
```

### Path Aliases
Use path aliases for cleaner imports:

```tsx
// ✅ CORRECT - Path alias
import { TaskService } from '@/features/tasks/services/task.service'

// ❌ WRONG - Relative path hell
import { TaskService } from '../../../features/tasks/services/task.service'
```

## Type vs Interface

### Use Interface for Object Shapes
```tsx
// ✅ Prefer interface for object shapes
interface TaskProps {
  id: string
  title: string
  onComplete: () => void
}

// ✅ Interface for extending
interface ExtendedTask extends Task {
  metadata: Record<string, unknown>
}
```

### Use Type for Unions and Utilities
```tsx
// ✅ Use type for unions
type TaskStatus = 'pending' | 'running' | 'completed' | 'failed'

// ✅ Use type for utility types
type PartialTask = Partial<Task>
type TaskKeys = keyof Task
```

## Function Typing

### Function Parameters
```tsx
// ✅ Type parameters inline
function createTask(title: string, description?: string): Task {
  return { id: generateId(), title, description }
}

// ✅ For complex types, extract interface
interface CreateTaskOptions {
  title: string
  description?: string
  priority?: 'low' | 'medium' | 'high'
  assignee?: string
}

function createTask(options: CreateTaskOptions): Task {
  // ...
}
```

### Return Types
Explicit return types are optional but recommended for exported functions:

```tsx
// ✅ Explicit return type (recommended for exports)
export function calculateTotal(items: Item[]): number {
  return items.reduce((sum, item) => sum + item.price, 0)
}

// ✅ Inferred return type (OK for internal functions)
function formatTitle(title: string) {
  return title.trim().toLowerCase()
}
```

### Async Functions
```tsx
// ✅ Async function with explicit Promise type
async function fetchTask(id: string): Promise<Task> {
  const response = await apiClient.get<Task>(`/tasks/${id}`)
  return response.data
}
```

## Generics

### Generic Functions
```tsx
// ✅ Descriptive generic names
function mapTasks<Input, Output>(
  tasks: Input[],
  mapper: (task: Input) => Output
): Output[] {
  return tasks.map(mapper)
}

// ✅ Constrained generics
function processEntity<T extends { id: string }>(entity: T): string {
  return entity.id
}
```

### Generic React Components
```tsx
// ✅ Generic component
interface ListProps<Item> {
  items: Item[]
  renderItem: (item: Item) => React.ReactNode
  keyExtractor: (item: Item) => string
}

export function List<Item>({ items, renderItem, keyExtractor }: ListProps<Item>) {
  return (
    <div>
      {items.map(item => (
        <div key={keyExtractor(item)}>
          {renderItem(item)}
        </div>
      ))}
    </div>
  )
}

// Usage
<List
  items={tasks}
  renderItem={(task) => <TaskCard task={task} />}
  keyExtractor={(task) => task.id}
/>
```

## Utility Types

### Built-in Utility Types
```tsx
// Partial - All properties optional
type PartialTask = Partial<Task>

// Required - All properties required
type RequiredTask = Required<Task>

// Pick - Select specific properties
type TaskSummary = Pick<Task, 'id' | 'title' | 'status'>

// Omit - Exclude specific properties
type CreateTaskData = Omit<Task, 'id' | 'createdAt' | 'updatedAt'>

// Record - Object with string keys
type TaskMap = Record<string, Task>
```

### Custom Utility Types
```tsx
// Make specific properties optional
type PartialBy<T, K extends keyof T> = Omit<T, K> & Partial<Pick<T, K>>

type TaskWithOptionalDescription = PartialBy<Task, 'description'>
```

## Enums vs Union Types

### Prefer Union Types
```tsx
// ✅ PREFER - Union type
type TaskStatus = 'pending' | 'running' | 'completed' | 'failed'

// ❌ AVOID - Enum
enum TaskStatus {
  Pending = 'pending',
  Running = 'running',
  Completed = 'completed',
  Failed = 'failed'
}
```

**Why**: Union types are simpler, compile to nothing, and work better with Zod schemas.

## Type Coercion

### Avoid Implicit Coercion
```tsx
// ❌ Implicit coercion
if (value) { }
if (!value) { }

// ✅ Explicit checks
if (value !== null && value !== undefined) { }
if (value === null || value === undefined) { }

// ✅ Boolean conversion when appropriate
if (Boolean(value)) { }
```

### Nullish Coalescing
```tsx
// ✅ Use ?? for null/undefined
const displayName = user.name ?? 'Anonymous'

// ❌ Don't use || (treats '' and 0 as falsy)
const displayName = user.name || 'Anonymous'
```

## Control Flow

### Early Returns
```tsx
// ✅ Early returns for clarity
function processTask(task: Task | null): string {
  if (!task) {
    return 'No task'
  }

  if (task.status === 'completed') {
    return 'Completed'
  }

  return 'In progress'
}

// ❌ Nested conditionals
function processTask(task: Task | null): string {
  if (task) {
    if (task.status === 'completed') {
      return 'Completed'
    } else {
      return 'In progress'
    }
  } else {
    return 'No task'
  }
}
```

### Switch Statements
```tsx
// ✅ Switch with exhaustive checking
function getStatusColor(status: TaskStatus): string {
  switch (status) {
    case 'pending':
      return 'gray'
    case 'running':
      return 'blue'
    case 'completed':
      return 'green'
    case 'failed':
      return 'red'
    default:
      const _exhaustive: never = status
      return _exhaustive
  }
}
```

## Anti-Patterns

### ❌ Using `any`
```tsx
// WRONG
function process(data: any) { }
```

### ❌ Default Exports
```tsx
// WRONG
export default function Component() { }
```

### ❌ import type
```tsx
// WRONG
import type { Task } from '@moatless/types'
```

### ❌ Non-descriptive Generic Names
```tsx
// WRONG
function map<T, U>(arr: T[], fn: (x: T) => U): U[] { }
```

### ❌ Implicit any
```tsx
// WRONG - parameter implicitly has 'any' type
function process(data) { }

// CORRECT
function process(data: TaskData) { }
```

## Summary

**Naming**: UpperCamelCase for types, camelCase for values
**Type Safety**: Never use `any`, use `unknown` if needed
**Exports**: Named exports only, no default exports
**Imports**: No `import type`, organize by source
**Generics**: Use descriptive names
**Unions**: Prefer union types over enums
**Coercion**: Be explicit, use ?? for nullish coalescing
