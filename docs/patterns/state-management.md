# State Management Patterns

AI-focused rules for managing state in React components.

## State Co-location

**Rule:** Keep state as close as possible to where it's used. Only lift when multiple components need it.

**Keep local when:**
- Only one component uses it (modal open state)
- UI-only state (dropdown open, hover state)
- Temporary/ephemeral state

**Lift state when:**
- Multiple siblings need same state
- State must synchronize across components
- Derived from same source of truth

```typescript
// ❌ Duplicated state
function TaskList() { const [filter, setFilter] = useState('all'); }
function TaskStats() { const [filter, setFilter] = useState('all'); }

// ✅ Lifted to common parent
function TaskPage() {
  const [filter, setFilter] = useState('all');
  return <><TaskList filter={filter} /><TaskStats filter={filter} /></>;
}
```

## useReducer for Interdependent State

**Use when:**
- Multiple state values depend on each other
- Complex state transitions
- State updates need multiple related changes

```typescript
// ❌ Multiple useState with interdependent logic
const [status, setStatus] = useState('idle');
const [error, setError] = useState(null);
const [data, setData] = useState(null);

const handleSubmit = () => {
  setStatus('loading');
  setError(null);
  setData(null);
  // ...
};

// ✅ useReducer for related state
const [state, dispatch] = useReducer(reducer, {
  status: 'idle',
  error: null,
  data: null
});

const handleSubmit = () => {
  dispatch({ type: 'SUBMIT_START' });
  // ...
};
```

## Context Splitting Pattern

Separate state and dispatch into different contexts to prevent unnecessary re-renders.

```typescript
// ❌ All consumers re-render on any state change
const TaskContext = createContext({ state, setState });

// ✅ Split contexts - dispatch-only consumers don't re-render
const TaskStateContext = createContext<TaskState>(null);
const TaskDispatchContext = createContext<TaskDispatch>(null);

function TaskProvider({ children }) {
  const [state, setState] = useState(initialState);
  return (
    <TaskStateContext.Provider value={state}>
      <TaskDispatchContext.Provider value={setState}>
        {children}
      </TaskDispatchContext.Provider>
    </TaskStateContext.Provider>
  );
}

// Component that only updates (no re-render on state change)
const dispatch = useContext(TaskDispatchContext);

// Component that only reads (doesn't get dispatch)
const state = useContext(TaskStateContext);
```

**Real example:** `apps/frontend/src/features/tasks/contexts/task-ui-provider.tsx`

## When to Use Global State vs Context vs Lifting

**Use local state + lifting when:**
- State shared by <5 components
- Components have close common parent
- State updates infrequently

**Use Context when:**
- State needed by many components at different nesting levels
- No close common parent
- Avoiding prop drilling through >3 levels

**Use global state (React Query, Zustand) when:**
- Server state (use React Query)
- Cross-feature state needed app-wide
- State persists across route changes

```typescript
// Local + lifting: Filter state for task list page
function TaskPage() {
  const [filter, setFilter] = useState('all');
  return <><TaskList filter={filter} /><TaskStats filter={filter} /></>;
}

// Context: UI state for task detail page (many nested components need it)
<TaskUIProvider taskId={taskId}>
  <TaskHeader />
  <TaskContent />
  <TaskSidebar />
</TaskUIProvider>

// React Query: Server state (tasks from API)
const { data: tasks } = useTasks();
```

## Decision Tree

```
Where should state live?

Single component needs it?
├─ Yes → Keep local
└─ No → Multiple components need it?
    ├─ Share close parent? → Lift to common parent
    └─ No close parent?
        ├─ Server data? → Use React Query
        ├─ UI state, many nested components? → Use Context
        └─ App-wide state? → Use global state manager
```

## State Location Rules

**Performance consideration:**
- Smaller state scope = smaller re-render tree
- Keep state low in component tree when possible
- Only lift to lowest common ancestor

**Maintainability consideration:**
- State near usage = easier to understand
- Clear ownership reduces bugs
- Less prop drilling = cleaner code

## Iteration and Variable Rules

- MUST default to `const` for bindings and only use `let` when a value is reassigned. NEVER use `var`.
- MUST iterate arrays with `for...of`, `Array.prototype.map`, or reducers. MUST NOT use `for...in` or `.forEach` when the loop depends on control-flow narrowing.
- MUST iterate objects via `Object.keys`, `Object.entries`, or explicit `hasOwnProperty` checks. NEVER rely on prototype-leaking `for...in` loops.

```tsx
// CORRECT – const bindings and for...of preserve type narrowing
for (const task of visibleTasks) {
  if (task.status === 'error') {
    dispatch({ type: 'TASK_FAILED', taskId: task.id })
  }
}

// WRONG – var and for...in leak indices and break control-flow checks
for (var index in visibleTasks) {
  const task = visibleTasks[index]
  if (task.status === 'error') {
    dispatch({ type: 'TASK_FAILED', taskId: task.id })
  }
}
```

## References

- [Choosing the State Structure](https://react.dev/learn/choosing-the-state-structure)
- [Passing Data Deeply with Context](https://react.dev/learn/passing-data-deeply-with-context)
- [Extracting State Logic into a Reducer](https://react.dev/learn/extracting-state-logic-into-a-reducer)
- [React Query Pattern](./react-query.md) - Server state management
- [Derived State Pattern](./derived-state.md) - Avoid storing derived values
