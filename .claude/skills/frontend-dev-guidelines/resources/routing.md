# Routing with TanStack Router

## Table of Contents
- [Overview](#overview)
- [File-Based Routing](#file-based-routing)
- [Creating Routes](#creating-routes)
- [Route Parameters](#route-parameters)
- [Data Loading](#data-loading)
- [Navigation](#navigation)
- [Route Guards](#route-guards)
- [Common Patterns](#common-patterns)
- [Anti-Patterns](#anti-patterns)

## Overview

**TanStack Router** provides type-safe, file-based routing for the application.

### Key Features
- File-based routing convention
- Type-safe route definitions
- Integrated data loading
- Route parameter validation
- Automatic code splitting

### Technology Stack
- `@tanstack/react-router` - Type-safe routing
- File-based route structure in `src/routes/`
- Automatic route tree generation

## File-Based Routing

### Route Structure
Routes are defined by the file structure in `src/routes/`:

```
src/routes/
├── __root.tsx              # Root layout
├── index.tsx               # Home page (/)
├── tasks/
│   ├── index.tsx           # Tasks list (/tasks)
│   ├── $taskId.tsx         # Task detail (/tasks/:taskId)
│   └── new.tsx             # New task (/tasks/new)
├── repositories/
│   ├── index.tsx           # Repositories list (/repositories)
│   └── $repositoryId.tsx   # Repository detail (/repositories/:repositoryId)
└── settings.tsx            # Settings page (/settings)
```

### File Naming Convention
- `index.tsx` - Index route for that path
- `$paramName.tsx` - Dynamic route parameter
- `_layout.tsx` - Layout without path segment
- `(group)/` - Route group (no path segment)

## Creating Routes

### Basic Route
```tsx
// src/routes/tasks/index.tsx
import { createFileRoute } from '@tanstack/react-router'
import { TaskList } from '@/features/tasks/components/task-list'

export const Route = createFileRoute('/tasks/')({
  component: TasksPage
})

function TasksPage() {
  return (
    <div className="container mx-auto p-4">
      <h1 className="text-2xl font-bold mb-4">Tasks</h1>
      <TaskList />
    </div>
  )
}
```

### Route with Layout
```tsx
// src/routes/_authenticated.tsx
import { createFileRoute, Outlet } from '@tanstack/react-router'
import { Sidebar } from '@/features/navigation/components/sidebar'

export const Route = createFileRoute('/_authenticated')({
  component: AuthenticatedLayout
})

function AuthenticatedLayout() {
  return (
    <div className="flex h-screen">
      <Sidebar />
      <main className="flex-1 overflow-auto">
        <Outlet />  {/* Child routes render here */}
      </main>
    </div>
  )
}
```

### Nested Route Under Layout
```tsx
// src/routes/_authenticated/tasks.tsx
import { createFileRoute } from '@tantml:router'
import { TaskList } from '@/features/tasks/components/task-list'

export const Route = createFileRoute('/_authenticated/tasks')({
  component: TasksPage
})

function TasksPage() {
  return <TaskList />
}
```

## Route Parameters

### Dynamic Route Parameter
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

### Multiple Parameters
```tsx
// src/routes/repositories/$repositoryId/tasks/$taskId.tsx
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/repositories/$repositoryId/tasks/$taskId')({
  component: RepositoryTaskDetailPage
})

function RepositoryTaskDetailPage() {
  const { repositoryId, taskId } = Route.useParams()

  return <div>Repository: {repositoryId}, Task: {taskId}</div>
}
```

### Optional Parameters
Use search params for optional parameters:

```tsx
// src/routes/tasks/index.tsx
import { createFileRoute } from '@tanstack/react-router'
import { z } from 'zod'

const searchSchema = z.object({
  filter: z.enum(['all', 'active', 'completed']).optional(),
  sort: z.enum(['date', 'title']).optional(),
  page: z.number().optional()
})

export const Route = createFileRoute('/tasks/')({
  component: TasksPage,
  validateSearch: searchSchema
})

function TasksPage() {
  const { filter, sort, page } = Route.useSearch()

  return <TaskList filter={filter} sort={sort} page={page} />
}
```

## Data Loading

### Loader Function
Load data before rendering component:

```tsx
// src/routes/tasks/$taskId.tsx
import { createFileRoute } from '@tanstack/react-router'
import { queryClient } from '@/lib/api/query-client'
import { queryKeys } from '@/lib/api/query-keys'
import { taskApi } from '@/features/tasks/api'

export const Route = createFileRoute('/tasks/$taskId')({
  loader: async ({ params: { taskId } }) => {
    // Prefetch task data
    await queryClient.ensureQueryData({
      queryKey: queryKeys.tasks.detail(taskId),
      queryFn: () => taskApi.getTask(taskId)
    })
  },
  component: TaskDetailPage
})

function TaskDetailPage() {
  const { taskId } = Route.useParams()
  const { data: task } = useTaskQuery(taskId)

  // Data is already loaded, so component renders immediately
  return <div>{task.title}</div>
}
```

### Dependent Data Loading
```tsx
export const Route = createFileRoute('/tasks/$taskId')({
  loader: async ({ params: { taskId } }) => {
    // Load task first
    const task = await queryClient.ensureQueryData({
      queryKey: queryKeys.tasks.detail(taskId),
      queryFn: () => taskApi.getTask(taskId)
    })

    // Then load related agent
    if (task.agentId) {
      await queryClient.ensureQueryData({
        queryKey: queryKeys.agents.detail(task.agentId),
        queryFn: () => agentApi.getAgent(task.agentId)
      })
    }
  },
  component: TaskDetailPage
})
```

### Parallel Data Loading
```tsx
export const Route = createFileRoute('/dashboard/')({
  loader: async () => {
    // Load multiple resources in parallel
    await Promise.all([
      queryClient.ensureQueryData({
        queryKey: queryKeys.tasks.list(),
        queryFn: () => taskApi.getTasks()
      }),
      queryClient.ensureQueryData({
        queryKey: queryKeys.environments.list(),
        queryFn: () => environmentApi.getEnvironments()
      }),
      queryClient.ensureQueryData({
        queryKey: queryKeys.settings.get(),
        queryFn: () => settingsApi.getSettings()
      })
    ])
  },
  component: DashboardPage
})
```

## Navigation

### Link Component
```tsx
import { Link } from '@tanstack/react-router'

function TaskCard({ task }: { task: Task }) {
  return (
    <Link
      to="/tasks/$taskId"
      params={{ taskId: task.id }}
      className="block p-4 border rounded hover:bg-gray-50"
    >
      <h3>{task.title}</h3>
      <p>{task.status}</p>
    </Link>
  )
}
```

### Link with Search Params
```tsx
<Link
  to="/tasks"
  search={{ filter: 'active', sort: 'date' }}
  className="text-blue-600 hover:underline"
>
  View Active Tasks
</Link>
```

### Programmatic Navigation
```tsx
import { useNavigate } from '@tanstack/react-router'

function CreateTaskForm() {
  const navigate = useNavigate()
  const createTask = useCreateTaskMutation()

  const handleSubmit = async (data: CreateTaskRequest) => {
    const newTask = await createTask.mutateAsync(data)

    // Navigate to new task detail page
    await navigate({
      to: '/tasks/$taskId',
      params: { taskId: newTask.id }
    })
  }

  return <form onSubmit={handleSubmit}>{/* ... */}</form>
}
```

### Navigation with State
```tsx
function TaskList() {
  const navigate = useNavigate()

  const handleTaskClick = (task: Task) => {
    navigate({
      to: '/tasks/$taskId',
      params: { taskId: task.id },
      state: { from: 'task-list' }  // Pass state
    })
  }

  return <div>{/* ... */}</div>
}

// Access state in target route
function TaskDetailPage() {
  const location = useLocation()
  const from = location.state?.from

  return <div>Navigated from: {from}</div>
}
```

### Back Navigation
```tsx
import { useRouter } from '@tanstack/react-router'

function TaskDetailPage() {
  const router = useRouter()

  const handleBack = () => {
    router.history.back()
  }

  return (
    <div>
      <button onClick={handleBack}>← Back</button>
      {/* Task detail content */}
    </div>
  )
}
```

## Route Guards

### Authentication Guard
```tsx
// src/routes/_authenticated.tsx
import { createFileRoute, redirect } from '@tanstack/react-router'
import { authService } from '@/lib/auth'

export const Route = createFileRoute('/_authenticated')({
  beforeLoad: async () => {
    const isAuthenticated = await authService.checkAuth()

    if (!isAuthenticated) {
      throw redirect({
        to: '/login',
        search: {
          redirect: location.href
        }
      })
    }
  },
  component: AuthenticatedLayout
})
```

### Permission Guard
```tsx
export const Route = createFileRoute('/_authenticated/admin')({
  beforeLoad: async () => {
    const user = await authService.getCurrentUser()

    if (user.role !== 'admin') {
      throw redirect({
        to: '/',
        search: {
          error: 'insufficient-permissions'
        }
      })
    }
  },
  component: AdminPage
})
```

## Common Patterns

### Breadcrumb Navigation
```tsx
import { useMatches } from '@tanstack/react-router'

function Breadcrumbs() {
  const matches = useMatches()

  return (
    <nav className="flex space-x-2">
      {matches.map((match, i) => (
        <div key={match.id} className="flex items-center">
          {i > 0 && <span className="mx-2">/</span>}
          <Link
            to={match.pathname}
            className="text-blue-600 hover:underline"
          >
            {match.routeContext?.breadcrumb || match.pathname}
          </Link>
        </div>
      ))}
    </nav>
  )
}
```

### Active Link Styling
```tsx
import { Link } from '@tanstack/react-router'

function Navigation() {
  return (
    <nav>
      <Link
        to="/tasks"
        activeProps={{
          className: 'font-bold text-blue-600'
        }}
        inactiveProps={{
          className: 'text-gray-600 hover:text-gray-900'
        }}
      >
        Tasks
      </Link>
    </nav>
  )
}
```

### Not Found Route
```tsx
// src/routes/__root.tsx
import { createRootRoute, Outlet } from '@tanstack/react-router'
import { NotFound } from '@/lib/components/not-found'

export const Route = createRootRoute({
  component: RootLayout,
  notFoundComponent: NotFound
})

function RootLayout() {
  return (
    <>
      <Header />
      <Outlet />
      <Footer />
    </>
  )
}
```

### Loading State
```tsx
import { createFileRoute } from '@tanstack/react-router'
import { LoadingSpinner } from '@/lib/components/loading-spinner'

export const Route = createFileRoute('/tasks/$taskId')({
  component: TaskDetailPage,
  pendingComponent: () => (
    <div className="flex items-center justify-center h-screen">
      <LoadingSpinner />
    </div>
  )
})
```

### Error Boundary
```tsx
import { createFileRoute, ErrorComponent } from '@tanstack/react-router'

export const Route = createFileRoute('/tasks/$taskId')({
  component: TaskDetailPage,
  errorComponent: ({ error }) => (
    <div className="p-4">
      <h1 className="text-xl font-bold text-red-600">Error Loading Task</h1>
      <p className="text-gray-600">{error.message}</p>
      <button onClick={() => window.location.reload()}>
        Reload Page
      </button>
    </div>
  )
})
```

## Anti-Patterns

### ❌ Hardcoding Routes
```tsx
// WRONG
<a href="/tasks/123">View Task</a>

// CORRECT
<Link to="/tasks/$taskId" params={{ taskId: '123' }}>
  View Task
</Link>
```

### ❌ Not Using Loaders for Data
```tsx
// WRONG - Data loads after component renders
function TaskDetailPage() {
  const { taskId } = Route.useParams()
  const { data, isLoading } = useTaskQuery(taskId)

  if (isLoading) return <Loading />  // User sees loading state

  return <div>{data.title}</div>
}

// CORRECT - Data preloaded via loader
export const Route = createFileRoute('/tasks/$taskId')({
  loader: async ({ params: { taskId } }) => {
    await queryClient.ensureQueryData({
      queryKey: queryKeys.tasks.detail(taskId),
      queryFn: () => taskApi.getTask(taskId)
    })
  },
  component: TaskDetailPage
})

function TaskDetailPage() {
  const { taskId } = Route.useParams()
  const { data } = useTaskQuery(taskId)  // Data already loaded

  return <div>{data.title}</div>  // Renders immediately
}
```

### ❌ Using window.location for Navigation
```tsx
// WRONG
window.location.href = '/tasks/123'

// CORRECT
navigate({ to: '/tasks/$taskId', params: { taskId: '123' } })
```

### ❌ Not Validating Search Params
```tsx
// WRONG - No validation, type safety lost
export const Route = createFileRoute('/tasks/')({
  component: TasksPage
})

function TasksPage() {
  const search = Route.useSearch()  // type is unknown
  const filter = search.filter  // No type safety!
}

// CORRECT - Validate with Zod
const searchSchema = z.object({
  filter: z.enum(['all', 'active', 'completed']).optional()
})

export const Route = createFileRoute('/tasks/')({
  component: TasksPage,
  validateSearch: searchSchema
})

function TasksPage() {
  const { filter } = Route.useSearch()  // Fully typed!
}
```

### ❌ Fetching Data in Component Instead of Loader
```tsx
// WRONG - Data fetched after navigation
function TaskDetailPage() {
  const { taskId } = Route.useParams()
  const { data } = useTaskQuery(taskId)  // Fetches after render

  return <div>{data?.title}</div>
}

// CORRECT - Data fetched before navigation completes
export const Route = createFileRoute('/tasks/$taskId')({
  loader: async ({ params: { taskId } }) => {
    await queryClient.ensureQueryData({
      queryKey: queryKeys.tasks.detail(taskId),
      queryFn: () => taskApi.getTask(taskId)
    })
  },
  component: TaskDetailPage
})
```
