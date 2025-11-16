# Derived State Without Effects Pattern

## Prerequisites

Before reading this pattern:
1. Understand [State Management Pattern](state-management.md) for when to use local vs lifted state
2. Review [Performance Optimization](performance-optimization.md) for when to apply `useMemo`
3. See [Component Design](component-design.md) for general component architecture

## Problem

React developers often reach for `useEffect` to synchronize state with props or other state values, leading to:
- Performance issues from unnecessary re-renders
- Infinite update loops
- Complex dependency arrays
- Hard-to-follow data flow
- Race conditions and timing bugs

## Solution

**MUST** calculate derived values directly during rendering instead of storing them in state. **MUST** use `useMemo` for expensive calculations. **SHOULD** use proper state lifting for cross-component communication.

## Task Components Analysis

This analysis identifies specific instances in the task components where useEffect is used for state synchronization that could be refactored to use derived state.

## Implementation

### Basic Derived State from Props

**RULE:** **MUST NOT** use `useEffect` to derive values from props. **MUST** calculate directly during render.

**❌ WRONG: Using effects for derived state from props**
```typescript
function TaskHeader({ task, iterations }) {
  const [currentIteration, setCurrentIteration] = useState(null);
  const [hasChanges, setHasChanges] = useState(false);
  
  // Unnecessary effect - causes extra renders
  useEffect(() => {
    const latest = iterations[iterations.length - 1];
    setCurrentIteration(latest);
  }, [iterations]);
  
  useEffect(() => {
    setHasChanges(task.status === 'completed' && task.hasUncommittedChanges);
  }, [task.status, task.hasUncommittedChanges]);
  
  return <div>{/* ... */}</div>;
}
```

**✅ CORRECT: Calculate directly from props**
```typescript
function TaskHeader({ task, iterations }) {
  // Calculate derived values directly from props
  const currentIteration = iterations[iterations.length - 1];
  const hasChanges = task.status === 'completed' && task.hasUncommittedChanges;
  
  return <div>{/* ... */}</div>;
}
```

### Derived State from Context

**RULE:** **MUST NOT** use `useEffect` to synchronize with context values. **MUST** use context values directly.

**❌ WRONG: Using effects to sync with context**
```typescript
function TaskViewer() {
  const { selectedIteration } = useTaskContext();
  const [displayedIteration, setDisplayedIteration] = useState(null);
  const [iterationStatus, setIterationStatus] = useState('');
  
  // Unnecessary effects
  useEffect(() => {
    setDisplayedIteration(selectedIteration);
  }, [selectedIteration]);
  
  useEffect(() => {
    if (displayedIteration) {
      setIterationStatus(displayedIteration.status === 'completed' ? 'Done' : 'In Progress');
    }
  }, [displayedIteration]);
  
  return <div>{iterationStatus}</div>;
}
```

**✅ CORRECT: Use context values directly**
```typescript
function TaskViewer() {
  const { selectedIteration } = useTaskContext();
  
  // Derive directly from context value
  const iterationStatus = selectedIteration?.status === 'completed' ? 'Done' : 'In Progress';
  
  return <div>{iterationStatus}</div>;
}
```

### Expensive Calculations

**RULE:** **MUST** use `useMemo` for expensive calculations. **MUST NOT** recalculate complex values on every render without memoization.

**❌ WRONG: Recalculating on every render**
```typescript
function TaskList({ tasks, filter }) {
  // This runs on every render, even when tasks/filter haven't changed
  const visibleTasks = tasks.filter(task => 
    task.status === filter || filter === 'all'
  );
  
  return <div>{/* ... */}</div>;
}
```

**✅ CORRECT: Memoize expensive calculations**
```typescript
function TaskList({ tasks, filter }) {
  const visibleTasks = useMemo(() => {
    return tasks.filter(task => 
      task.status === filter || filter === 'all'
    );
  }, [tasks, filter]);
  
  return <div>{/* ... */}</div>;
}
```

### Resetting State When Props Change

**RULE:** **MUST** use `key` prop to reset component state when props change. **MUST NOT** use `useEffect` to manually reset state.

**❌ WRONG: Using effects to reset state**
```typescript
function TaskDetail({ taskId }) {
  const [selectedIteration, setSelectedIteration] = useState(null);
  
  // Causes extra render
  useEffect(() => {
    setSelectedIteration(null);
  }, [taskId]);
  
  return <div>{/* ... */}</div>;
}
```

**✅ CORRECT: Use key prop to reset component**
```typescript
function TaskDetailWrapper({ taskId }) {
  // Key change tells React to create new component instance
  return <TaskDetail key={taskId} taskId={taskId} />;
}

function TaskDetail({ taskId }) {
  // State automatically resets when component remounts
  const [selectedIteration, setSelectedIteration] = useState(null);
  return <div>{/* ... */}</div>;
}
```

### Event Handler Logic

**RULE:** **MUST** handle user interactions directly in event handlers. **NEVER** use `useEffect` to respond to user events.

**❌ WRONG: Using effects for event logic**
```typescript
function ChatInput({ onSend }) {
  const [message, setMessage] = useState('');
  const [shouldSend, setShouldSend] = useState(false);
  
  useEffect(() => {
    if (shouldSend && message) {
      onSend(message);
      setMessage('');
      setShouldSend(false);
    }
  }, [shouldSend, message, onSend]);
  
  return (
    <button onClick={() => setShouldSend(true)}>
      Send
    </button>
  );
}
```

**✅ CORRECT: Handle in event directly**
```typescript
function ChatInput({ onSend }) {
  const [message, setMessage] = useState('');
  
  const handleSend = () => {
    if (message) {
      onSend(message);
      setMessage('');
    }
  };
  
  return (
    <button onClick={handleSend}>
      Send
    </button>
  );
}
```

### Parent-Child Communication

**RULE:** **MUST** handle state updates in the same event that triggers them. **MUST NOT** use effects in child components to notify parents.

**❌ WRONG: Effects for state synchronization**
```typescript
function Parent() {
  const [parentState, setParentState] = useState(0);
  const [childNotified, setChildNotified] = useState(false);
  
  return (
    <Child 
      value={parentState} 
      onNotify={() => setChildNotified(true)} 
    />
  );
}

function Child({ value, onNotify }) {
  useEffect(() => {
    if (value > 10) {
      onNotify();
    }
  }, [value, onNotify]);
  
  return <div>{value}</div>;
}
```

**✅ CORRECT: Handle in the same event**
```typescript
function Parent() {
  const [value, setValue] = useState(0);
  const [isNotified, setIsNotified] = useState(false);
  
  const updateValue = (newValue) => {
    setValue(newValue);
    if (newValue > 10) {
      setIsNotified(true);
    }
  };
  
  return <Child value={value} onUpdate={updateValue} />;
}
```

## When Effects ARE Appropriate

**SHOULD** use `useEffect` for:
- External subscriptions (WebSocket, DOM events, SSE connections)
- Side effects (logging, analytics, telemetry)
- Synchronizing with external systems (browser APIs, third-party libraries)
- Cleanup operations (unsubscribe, abort requests, clear timers)

**MUST NOT** use `useEffect` for:
- Deriving values from props or state
- Responding to user events
- Synchronizing state between components
- Calculations based on existing data

## Implementation Guidelines

**MUST** follow these rules when working with state:

1. **Start simple** – **ALWAYS** calculate values directly during render first
2. **Optimize when proven necessary** – **ONLY** add `useMemo` for operations that cause measurable performance issues (see [Performance Optimization](performance-optimization.md))
3. **Use complete dependencies** – **MUST** include all values from component scope used inside the effect
4. **Consider alternatives** – **SHOULD** use `key` prop, event handlers, or state lifting (see [State Management](state-management.md)) before reaching for effects

## Quick Decision Tree

```
Need to compute a value based on props or state?
├─ Is it expensive (>10ms)?
│  ├─ YES → Use useMemo
│  └─ NO → Calculate directly
├─ Is it based on user interaction?
│  └─ YES → Handle in event handler
├─ Need to reset when prop changes?
│  └─ YES → Use key prop
└─ Need to sync with external system?
   └─ YES → Use useEffect
```

## References

- [You Might Not Need an Effect](https://react.dev/learn/you-might-not-need-an-effect)
- [Separating Events from Effects](https://react.dev/learn/separating-events-from-effects)
- [useMemo Hook](https://react.dev/reference/react/useMemo)
