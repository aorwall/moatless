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

## Real Examples from Codebase

### Deriving State from Multiple Sources
```typescript
// From: apps/frontend/src/features/tasks/components/task-header.tsx
export function TaskHeader() {
  const { task, iterations } = useTaskData();
  const { workspaceHasChanges } = useWorkspaceStatus();
  
  // Derive complex state from multiple sources
  const currentIteration = iterations[iterations.length - 1];
  const isCompleted = task.status === 'completed';
  const needsCommit = isCompleted && workspaceHasChanges;
  
  // Only use useMemo for expensive operations
  const stats = useMemo(() => {
    return {
      totalTime: iterations.reduce((sum, iter) => sum + iter.duration, 0),
      avgTime: iterations.length > 0 ? totalTime / iterations.length : 0,
      successRate: iterations.filter(i => i.status === 'success').length / iterations.length
    };
  }, [iterations]);
  
  return <div>{/* ... */}</div>;
}
```

### Task Context Memoization
```typescript
// From: apps/frontend/src/features/tasks/contexts/task-ui-provider.tsx
export function TaskUIProvider({ taskId, children }) {
  const [taskStates, setTaskStates] = useState(new Map());
  
  // Derive current state from taskId
  const currentState = taskStates.get(taskId) || getDefaultTaskUIState();
  
  // Memoize callbacks to prevent re-renders
  const setRightPanelView = useCallback((view) => {
    setTaskStates(prev => {
      const newMap = new Map(prev);
      const state = newMap.get(taskId) || getDefaultTaskUIState();
      newMap.set(taskId, { ...state, rightPanelView: view });
      return newMap;
    });
  }, [taskId]);
  
  // Stable context value
  const contextValue = useMemo(
    () => ({
      currentState,
      setRightPanelView,
      // ... other methods
    }),
    [currentState, setRightPanelView]
  );
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
2. **Optimize when needed**: Add `useMemo` only for expensive operations
3. **Measure performance**: Use React DevTools Profiler to verify improvements
4. **Use proper dependencies**: Ensure dependency arrays are complete
5. **Consider alternatives**: `key` prop, event handlers, or lifting state up

## References

- [You Might Not Need an Effect](https://react.dev/learn/you-might-not-need-an-effect)
- [Separating Events from Effects](https://react.dev/learn/separating-events-from-effects)
- [useMemo Hook](https://react.dev/reference/react/useMemo)

## Specific Refactoring Opportunities in Task Components

### 1. ChatView Component (chat-view.tsx)

#### Scroll Position State Synchronization (Lines 51-59, 106-145)
**Current Pattern:**
```typescript
// Lines 51-54: State for scroll position
const [scrollTop, setScrollTop] = useState(0)
const [scrollHeight, setScrollHeight] = useState(0)
const [clientHeight, setClientHeight] = useState(0)

// Lines 56-58: Derive isAtBottom from state
const isAtBottom = useMemo(() => {
  return scrollHeight - scrollTop - clientHeight < 50
}, [scrollTop, scrollHeight, clientHeight])

// Lines 106-145: Effect to sync scroll state
useEffect(() => {
  const scrollArea = scrollRef.current
  if (!scrollArea) return

  const viewport = scrollArea.querySelector('[data-radix-scroll-area-viewport]') as HTMLElement
  // ... scroll event handler updates state
}, [scrollRef])
```

**Issue:** Using state to track scroll position and syncing it with useEffect causes unnecessary re-renders on every scroll event.

**Recommended Refactor:**
```typescript
// Use a ref for scroll values and only update state when isAtBottom changes
const scrollValuesRef = useRef({ scrollTop: 0, scrollHeight: 0, clientHeight: 0 })
const [isAtBottom, setIsAtBottom] = useState(true)

const handleScroll = useCallback(() => {
  const viewport = scrollRef.current?.querySelector('[data-radix-scroll-area-viewport]')
  if (!viewport) return
  
  const newIsAtBottom = viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight < 50
  if (newIsAtBottom !== isAtBottom) {
    setIsAtBottom(newIsAtBottom)
  }
}, [isAtBottom])
```

### 3. ChatForm Component (chat-form.tsx)

#### Initial Textarea Resize (Lines 123-133)
**Current Pattern:**
```typescript
// Initial resize on mount
useEffect(() => {
  const textarea = inputRef.current
  if (!textarea) return

  // Set initial height
  textarea.style.height = 'auto'
  const scrollHeight = textarea.scrollHeight
  const maxHeightPx = parseInt(maxHeight)
  const newHeight = Math.min(scrollHeight, maxHeightPx)
  textarea.style.height = `${newHeight}px`
}, [maxHeight, inputRef.current]) // Problematic dependency on ref.current
```

**Issue:** Using effect to set initial DOM properties. The dependency on `inputRef.current` is problematic.

**Recommended Refactor:**
```typescript
// Use a callback ref
const textareaRef = useCallback((node: HTMLTextAreaElement | null) => {
  if (node) {
    // Set initial height when element is attached
    node.style.height = 'auto'
    const scrollHeight = node.scrollHeight
    const maxHeightPx = parseInt(maxHeight)
    const newHeight = Math.min(scrollHeight, maxHeightPx)
    node.style.height = `${newHeight}px`
  }
  // Store ref if needed elsewhere
  inputRef.current = node
}, [maxHeight])
```

#### Dropdown Position Detection (Lines 136-149)
**Current Pattern:**
```typescript
// Detect if dropdown should appear above or below
useEffect(() => {
  if (isAutocompleteOpen && containerRef.current) {
    const rect = containerRef.current.getBoundingClientRect()
    const spaceBelow = window.innerHeight - rect.bottom
    const dropdownHeight = 240

    if (spaceBelow < dropdownHeight && isTaskContext) {
      setDropdownPosition('above')
    } else {
      setDropdownPosition('below')
    }
  }
}, [isAutocompleteOpen, isTaskContext])
```

**Issue:** Using effect to calculate UI position based on props.

**Recommended Refactor:**
```typescript
// Calculate position directly when rendering
const dropdownPosition = useMemo(() => {
  if (!isAutocompleteOpen || !containerRef.current) return 'below'
  
  const rect = containerRef.current.getBoundingClientRect()
  const spaceBelow = window.innerHeight - rect.bottom
  const dropdownHeight = 240
  
  return spaceBelow < dropdownHeight && isTaskContext ? 'above' : 'below'
}, [isAutocompleteOpen, isTaskContext])
```

### 4. MinifiedLastEventsDisplay Component (minified-last-events-display.tsx)

#### Event Filtering (Lines 43-51)
**Current Pattern:**
```typescript
const [displayedEvents, setDisplayedEvents] = useState<Array<Event>>([])

// Filter and update displayed events
useEffect(() => {
  // Filter to only include message and tool events
  const filteredEvents = events.filter(isMessageOrToolEvent)
  // Take the last 3 events
  const lastThree = filteredEvents.slice(-3)
  setDisplayedEvents(lastThree)
}, [events])
```

**Issue:** Classic anti-pattern of syncing props to state.

**Recommended Refactor:**
```typescript
// Derive directly from props
const displayedEvents = useMemo(() => {
  const filteredEvents = events.filter(isMessageOrToolEvent)
  return filteredEvents.slice(-3)
}, [events])
```

### 5. GitDiffFileTree Component (git-diff-file-tree.tsx)

#### Auto-expand Directories (Lines 77-93)
**Current Pattern:**
```typescript
// Auto-expand all directories initially
React.useEffect(() => {
  if (expandedPaths.size === 0 && tree.length > 0) {
    const allDirectories = new Set<string>()
    const collectDirectories = (nodes: FileTreeNode[]) => {
      // ... collect directories
    }
    collectDirectories(tree)
    updateGitDiffState({ expandedPaths: allDirectories })
  }
}, [tree, expandedPaths.size, updateGitDiffState])
```

**Issue:** Using effect to set initial UI state based on data.

**Recommended Refactor:**
```typescript
// Initialize expanded paths in the context provider or use a custom hook
const useInitialExpandedPaths = (tree: FileTreeNode[]) => {
  return useMemo(() => {
    const allDirectories = new Set<string>()
    const collectDirectories = (nodes: FileTreeNode[]) => {
      nodes.forEach((node) => {
        if (node.type === 'directory') {
          allDirectories.add(node.path)
          if (node.children) {
            collectDirectories(node.children)
          }
        }
      })
    }
    collectDirectories(tree)
    return allDirectories
  }, [tree])
}
```
