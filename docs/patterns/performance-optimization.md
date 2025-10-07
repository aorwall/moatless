# Performance Optimization

AI-focused rules for React performance optimization. Use these when you observe performance issues, not by default.

## Premature Optimization

**Rule:** Start simple. Only optimize when performance issues are observed.

**Optimize when:**
- Component re-renders >10 times/second
- Render time >16ms
- Expensive calculation (array ops >100 items, recursive ops) repeated unnecessarily

**Don't optimize on first implementation:**
```typescript
// ❌ Premature optimization
const title = useMemo(() => task.title, [task.title]);
const handleClick = useCallback(() => console.log('clicked'), []);

// ✅ Start simple
const title = task.title;
const handleClick = () => console.log('clicked');
```

## useMemo

**✅ Use when:**
- Array operations on >100 items: `useMemo(() => tasks.sort(...), [tasks])`
- Expensive object with multiple computed properties (>5 operations)
- Recursive operations: `useMemo(() => buildFileTree(files), [files])`

**❌ Don't use when:**
- Simple calculations (<3 operations): `a + b` not `useMemo(() => a + b, [a, b])`
- Single property access: `task.title.toUpperCase()` not `useMemo(() => task.title.toUpperCase(), [task.title])`
- Value changes every render: `new Date()` not `useMemo(() => new Date(), [])`

**See also:** [Derived State](./derived-state.md) for more useMemo examples in context.

## useCallback

**✅ Use when:**
- Passed to `React.memo` component: `const onClick = useCallback(() => {...}, [])` → `<MemoizedChild onClick={onClick} />`
- In another hook's dependency array: `useCallback` → `useEffect(..., [callback])`
- Passed to expensive virtualized components

**❌ Don't use when:**
- "Just in case": `() => console.log('click')` not `useCallback(() => console.log('click'), [])`
- Callback dependencies change frequently anyway

## React.memo

**✅ Use when:**
- Component re-renders frequently (>10/sec) but props rarely change
- Expensive render logic (syntax highlighting, complex calculations)
- Leaf component in frequently updating tree

**❌ Don't use when:**
- Props change every render (e.g., `timestamp` prop)
- Component is cheap to render (simple JSX)
- As default practice (don't wrap every component)

## Common Memoization Pitfalls

**1. Object/Array props break memoization**
```typescript
// ❌ New object every render
<TaskCard config={{ showIcon: true }} />

// ✅ Stable reference
const DEFAULT_CONFIG = { showIcon: true };
<TaskCard config={DEFAULT_CONFIG} />
```

**2. Inline functions break memoization**
```typescript
// ❌ New function every render
<TaskCard onDelete={() => deleteTask(task.id)} />

// ✅ Stable callback
const handleDelete = useCallback((taskId: string) => deleteTask(taskId), []);
<TaskCard taskId={task.id} onDelete={handleDelete} />
```

**3. Missing dependencies**
```typescript
// ❌ Uses 'filter' but not in deps
useMemo(() => tasks.filter(t => t.status === filter), [tasks])

// ✅ Complete dependencies
useMemo(() => tasks.filter(t => t.status === filter), [tasks, filter])
```

## Decision Flow

```
Is there a performance issue?
├─ No → Don't optimize. Keep code simple.
└─ Yes → What's the issue?
    ├─ Expensive calculation repeated → useMemo
    ├─ Callback causing child re-renders → useCallback
    ├─ Component re-renders with same props → React.memo
    └─ Other → Profile to find actual bottleneck
```

## References

- [React useMemo Hook](https://react.dev/reference/react/useMemo)
- [React useCallback Hook](https://react.dev/reference/react/useCallback)
- [React.memo](https://react.dev/reference/react/memo)
- [Derived State Pattern](./derived-state.md) - When to calculate during render
