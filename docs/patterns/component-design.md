# Component Design Principles

## Prerequisites

Before reading this pattern:
1. Review [Derived State Pattern](derived-state.md) for avoiding unnecessary state
2. See [State Management](state-management.md) for when to lift state
3. Understand [React Query Patterns](react-query.md) for server state management

AI-focused guidelines for designing React components in this codebase.

## TypeScript Naming Rules

- MUST declare exported components, types, enums, and decorators with UpperCamelCase identifiers (e.g. `TaskCard`). Internal variables, hooks, and functions MUST use lowerCamelCase.
- MUST treat abbreviations as whole words (`loadHttpUrl`, not `loadHTTPURL`).
- MUST NOT prefix or suffix identifiers with `_`, including for unused parameters. Use destructuring gaps (`const [first,,third] = values`) instead.
- SHOULD align with third-party casing only when integrating well-known libraries that require it (e.g. `rxjs$` streams).

```tsx
// CORRECT – naming follows ts.dev style rules
export function TaskSummaryCard({ taskId }: { taskId: string }) {
  const taskStats = useTaskStats(taskId)
  return <TaskStatsPanel taskStats={taskStats} />
}

// WRONG – camelcase is inconsistent and uses underscore sentinel
export function task_summary({ _task }) {
  const Task_stats = useTaskStats(_task.id)
  return <taskstats-panel stats={Task_stats} />
}
```

## Single Responsibility Principle

**RULE:** **MUST** design components with a single, focused responsibility. A component **MUST** be describable in one sentence without using "and" or "or".

```typescript
// ❌ WRONG: Violates SRP - does multiple things
function TaskCardWithActionsAndStats({ task }) {
  return (
    <div>
      <TaskHeader task={task} />
      <TaskStats task={task} />
      <TaskActions task={task} />
      <TaskTimeline task={task} />
    </div>
  );
}

// ✅ CORRECT: Single responsibility
function TaskCard({ task }) {
  return (
    <div>
      <TaskHeader task={task} />
      <TaskContent task={task} />
    </div>
  );
}

function TaskStats({ task }) {
  return <div>{/* stats only */}</div>;
}
```

**MUST split components when:**
- Component description contains "and" or "or"
- Component handles multiple unrelated concerns
- Component has >3 distinct sections in JSX

## Component Sizing

**RULE:** **MUST** split components when they exceed complexity thresholds.

**MUST split when:**
- File length >300 lines
- Component has >3 `useState` hooks
- Render logic >50 lines
- Function has >10 local variables

```typescript
// ❌ WRONG: Too large - 400 lines, 5 useState, complex render
function TaskDetail({ taskId }) {
  const [selectedTab, setSelectedTab] = useState('chat');
  const [isEditing, setIsEditing] = useState(false);
  const [showActions, setShowActions] = useState(false);
  const [filter, setFilter] = useState('all');
  const [sortBy, setSortBy] = useState('date');

  return (
    <div>
      {/* 300+ lines of JSX */}
    </div>
  );
}

// ✅ CORRECT: Split into smaller components
function TaskDetail({ taskId }) {
  return (
    <>
      <TaskDetailHeader taskId={taskId} />
      <TaskDetailTabs taskId={taskId} />
      <TaskDetailActions taskId={taskId} />
    </>
  );
}
```

## Prop Design

**RULE:** **SHOULD** pass primitive types over objects ("pass the banana, not the jungle"). **MUST** only pass objects when component needs 5+ related properties.

```typescript
// ❌ WRONG: Tightly coupled - needs entire task object
function TaskTitle({ task }) {
  return <h1>{task.title}</h1>;
}

// ✅ CORRECT: Loosely coupled - only needs title
function TaskTitle({ title }) {
  return <h1>{title}</h1>;
}

// ❌ WRONG: Passes entire config object
<TaskCard task={task} config={appConfig} theme={theme} user={user} />

// ✅ CORRECT: Pass only what's needed
<TaskCard
  taskId={task.id}
  title={task.title}
  status={task.status}
  isDarkMode={theme.mode === 'dark'}
/>
```

**Benefits:**
- Component can be reused in different contexts
- Props are self-documenting
- Easier to test (fewer mocks needed)

**Exception:** **MAY** pass object when component needs 5+ related properties from same object.

```typescript
// ✅ CORRECT: OK to pass object when many related properties needed
function TaskForm({ task }) {
  // Uses: task.title, task.description, task.status, task.priority,
  //       task.assignee, task.dueDate, task.tags
  return <form>{/* ... */}</form>;
}
```

## Component Composition

**RULE:** **SHOULD** use composition for flexible, reusable components.

**MUST use composition when:**
- Building multiple variants of similar UI
- Component needs flexible content
- Avoiding prop drilling (see [State Management](state-management.md))

```typescript
// ❌ WRONG: Configuration-based (limited flexibility)
function Modal({ title, content, footer, showClose }) {
  return (
    <div>
      {showClose && <CloseButton />}
      <h2>{title}</h2>
      <div>{content}</div>
      <div>{footer}</div>
    </div>
  );
}

// ✅ CORRECT: Composition-based (flexible)
function Modal({ children }) {
  return <div className="modal">{children}</div>;
}

function Modal.Header({ children }) {
  return <div className="modal-header">{children}</div>;
}

function Modal.Body({ children }) {
  return <div className="modal-body">{children}</div>;
}

// Usage
<Modal>
  <Modal.Header>
    <h2>Custom Title</h2>
    <CloseButton />
  </Modal.Header>
  <Modal.Body>
    <CustomContent />
  </Modal.Body>
</Modal>
```

**Compound components pattern** (see codebase examples):
- `apps/frontend/src/lib/components/configuration/` - Configuration dialog components
- `apps/frontend/src/lib/components/ui/` - shadcn/ui components

## Avoiding Wrong Abstraction

**RULE:** Duplication is cheaper than wrong abstraction. **MUST NOT** generalize until you have 3+ similar use cases.

```typescript
// ❌ WRONG: Premature abstraction (only 2 use cases)
function GenericList({ items, renderItem, emptyMessage, filterFn, sortFn }) {
  const filtered = items.filter(filterFn);
  const sorted = filtered.sort(sortFn);
  return sorted.length ? sorted.map(renderItem) : <div>{emptyMessage}</div>;
}

// ✅ CORRECT: Separate components (easier to understand and modify)
function TaskList({ tasks }) {
  const sorted = tasks.sort((a, b) => b.priority - a.priority);
  return sorted.map(task => <TaskCard key={task.id} task={task} />);
}

function UserList({ users }) {
  const active = users.filter(u => u.active);
  return active.map(user => <UserCard key={user.id} user={user} />);
}
```

**SHOULD abstract only when:**
- Same logic exists in 3+ places
- Abstraction makes code clearer (not more complex)
- You have concrete use cases (not speculative future needs)

**NEVER abstract for:**
- Speculative future requirements
- Only 1-2 similar cases
- When abstraction adds complexity

## Real Examples from Codebase

### Good: Single Responsibility
```typescript
// apps/frontend/src/features/tasks/components/task-status-icon.tsx (36 lines)
// Single job: Display status icon based on task status
export function TaskStatusIcon({ status }: TaskStatusIconProps) {
  // Just returns appropriate icon for status
}
```

### Good: Composition
```typescript
// apps/frontend/src/lib/components/configuration/configuration-dialog-responsive.tsx
// Uses compound components for flexible dialog structure
<ConfigurationDialog>
  <ConfigurationDialogSidebar />
  <ConfigurationDialogContent />
</ConfigurationDialog>
```

### Good: Primitive Props
```typescript
// apps/frontend/src/lib/components/ui/button.tsx
// Accepts primitive props, not entire theme object
interface ButtonProps {
  variant?: 'default' | 'destructive' | 'outline' | 'secondary' | 'ghost' | 'link'
  size?: 'default' | 'sm' | 'lg' | 'icon'
}
```

## Decision Rules Summary

**MUST split component when:**
- Can't describe in one sentence without "and"/"or"
- File >300 lines OR >3 useState OR render >50 lines
- Multiple unrelated concerns

**SHOULD use primitive props when:**
- Component needs <5 properties from object
- Component should be reusable across contexts

**MUST use composition when:**
- Building flexible, variant-rich components
- Avoiding configuration prop explosion
- Creating component libraries

**SHOULD abstract/generalize only when:**
- Same logic exists in 3+ places
- Abstraction reduces complexity (not adds it)
- Have concrete use cases (not speculation)

## Related Patterns

- [Derived State Pattern](derived-state.md) - Avoiding unnecessary state in components
- [State Management](state-management.md) - When to lift state vs keep it local
- [React Query Patterns](react-query.md) - Managing server state
- [Configuration Dialog Pattern](configuration-dialog.md) - Example of compound components
