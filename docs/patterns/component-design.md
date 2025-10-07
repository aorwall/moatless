# Component Design Principles

AI-focused guidelines for designing React components in this codebase.

## Single Responsibility Principle

**Test:** Can you describe the component in one sentence without using "and" or "or"?

```typescript
// ❌ Violates SRP - does multiple things
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

// ✅ Single responsibility
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

**When to split:**
- Component description contains "and" or "or"
- Component handles multiple unrelated concerns
- Component has >3 distinct sections in JSX

## Component Sizing

**Split when:**
- File length >300 lines
- Component has >3 `useState` hooks
- Render logic >50 lines
- Function has >10 local variables

```typescript
// ❌ Too large - 400 lines, 5 useState, complex render
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

// ✅ Split into smaller components
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

**Rule:** Prefer passing primitive types over objects ("pass the banana, not the jungle").

```typescript
// ❌ Tightly coupled - needs entire task object
function TaskTitle({ task }) {
  return <h1>{task.title}</h1>;
}

// ✅ Loosely coupled - only needs title
function TaskTitle({ title }) {
  return <h1>{title}</h1>;
}

// ❌ Passes entire config object
<TaskCard task={task} config={appConfig} theme={theme} user={user} />

// ✅ Pass only what's needed
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

**Exception:** When component truly needs 5+ related properties from same object, pass the object.

```typescript
// ✅ OK to pass object when many related properties needed
function TaskForm({ task }) {
  // Uses: task.title, task.description, task.status, task.priority,
  //       task.assignee, task.dueDate, task.tags
  return <form>{/* ... */}</form>;
}
```

## Component Composition

**Use composition when:**
- Multiple variants of similar UI (use children/render props)
- Component needs flexible content
- Avoiding prop drilling

```typescript
// ❌ Configuration-based (limited flexibility)
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

// ✅ Composition-based (flexible)
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

**Rule:** Duplication is cheaper than wrong abstraction. Don't generalize until you have 3+ similar use cases.

```typescript
// ❌ Premature abstraction (only 2 use cases)
function GenericList({ items, renderItem, emptyMessage, filterFn, sortFn }) {
  const filtered = items.filter(filterFn);
  const sorted = filtered.sort(sortFn);
  return sorted.length ? sorted.map(renderItem) : <div>{emptyMessage}</div>;
}

// ✅ Separate components (easier to understand and modify)
function TaskList({ tasks }) {
  const sorted = tasks.sort((a, b) => b.priority - a.priority);
  return sorted.map(task => <TaskCard key={task.id} task={task} />);
}

function UserList({ users }) {
  const active = users.filter(u => u.active);
  return active.map(user => <UserCard key={user.id} user={user} />);
}
```

**When to abstract:**
- Same logic in 3+ places
- Abstraction makes code clearer (not more complex)
- You have concrete use cases (not speculative future needs)

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

**Split component when:**
- Can't describe in one sentence without "and"/"or"
- File >300 lines OR >3 useState OR render >50 lines
- Multiple unrelated concerns

**Use primitive props when:**
- Component needs <5 properties from object
- Component should be reusable across contexts

**Use composition when:**
- Building flexible, variant-rich components
- Avoiding configuration prop explosion

**Abstract/generalize when:**
- Same logic in 3+ places
- Abstraction reduces complexity (not adds)
