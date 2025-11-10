# Derived State Without Effects Pattern

## Problem

React developers often reach for `useEffect` to synchronize state with props or other state values, leading to:
- Performance issues from unnecessary re-renders
- Infinite update loops
- Complex dependency arrays
- Hard-to-follow data flow
- Race conditions and timing bugs

## Solution

Calculate derived values directly during rendering instead of storing them in state. Use `useMemo` for expensive calculations and proper state lifting for cross-component communication.

## Task Components Analysis

This analysis identifies specific instances in the task components where useEffect is used for state synchronization that could be refactored to use derived state.

## Implementation

### Basic Derived State from Props

**❌ Bad: Using effects for derived state from props**
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

**✅ Good: Calculate directly from props**
```typescript
function TaskHeader({ task, iterations }) {
  // Calculate derived values directly from props
  const currentIteration = iterations[iterations.length - 1];
  const hasChanges = task.status === 'completed' && task.hasUncommittedChanges;
  
  return <div>{/* ... */}</div>;
}
```

### Derived State from Context

**❌ Bad: Using effects to sync with context**
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

**✅ Good: Use context values directly**
```typescript
function TaskViewer() {
  const { selectedIteration } = useTaskContext();
  
  // Derive directly from context value
  const iterationStatus = selectedIteration?.status === 'completed' ? 'Done' : 'In Progress';
  
  return <div>{iterationStatus}</div>;
}
```

### Expensive Calculations

**❌ Bad: Recalculating on every render**
```typescript
function TaskList({ tasks, filter }) {
  // This runs on every render, even when tasks/filter haven't changed
  const visibleTasks = tasks.filter(task => 
    task.status === filter || filter === 'all'
  );
  
  return <div>{/* ... */}</div>;
}
```

**✅ Good: Memoize expensive calculations**
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

**❌ Bad: Using effects to reset state**
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

**✅ Good: Use key prop to reset component**
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

**❌ Bad: Using effects for event logic**
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

**✅ Good: Handle in event directly**
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

**❌ Bad: Effects for state synchronization**
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

**✅ Good: Handle in the same event**
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

## When to Use Effects

Effects are still appropriate for:
- External subscriptions (WebSocket, DOM events)
- Side effects (logging, analytics)
- Synchronizing with external systems
- Cleanup operations

## Guidelines

1. **Start simple**: Calculate values directly during render
2. **Optimize when needed**: Add `useMemo` only for expensive operations (see [Performance Optimization](./performance-optimization.md))
3. **Use proper dependencies**: Ensure dependency arrays are complete
4. **Consider alternatives**: `key` prop, event handlers, or [lifting state](./state-management.md)

## References

- [You Might Not Need an Effect](https://react.dev/learn/you-might-not-need-an-effect)
- [Separating Events from Effects](https://react.dev/learn/separating-events-from-effects)
- [useMemo Hook](https://react.dev/reference/react/useMemo)
