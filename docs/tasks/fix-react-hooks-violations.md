# Task: Fix React Hooks Violations and Performance Issues

## Overview
This codebase has 19 React hooks violations that need to be fixed to prevent performance issues, infinite loops, and potential bugs. These were discovered after enabling strict React hooks linting rules in Biome.

## Current Issues Summary
- **17 useExhaustiveDependencies violations**: Missing or incorrect dependencies in hooks
- **2 useHookAtTopLevel violations**: Hooks called conditionally after early returns
- **Multiple performance anti-patterns**: Effects used for derived state

## Prerequisites
1. Read the derived state pattern documentation: `/docs/patterns/derived-state.md`
2. Understand React hooks rules: https://react.dev/learn/you-might-not-need-an-effect
3. Run `pnpm -w run biome:check` to see all current violations

## Tasks

### 1. Fix useHookAtTopLevel Violations (Critical)

**Location**: `apps/frontend/src/features/tasks/components/chat-view/chat-view.tsx`
- Lines 264 and 277: Hooks are called after an early return on line 193-210
- **Solution**: Move all hooks above any conditional returns
- **Pattern**:
  ```typescript
  // ❌ Bad
  if (loading) return <Loading />
  const data = useMemo(() => process(), [])
  
  // ✅ Good
  const data = useMemo(() => process(), [])
  if (loading) return <Loading />
  ```

### 2. Fix useExhaustiveDependencies Violations

Run this command to see all violations:
```bash
pnpm exec biome check apps/frontend/src --max-diagnostics=1000 2>&1 | grep -A5 "useExhaustiveDependencies"
```

**Common patterns to fix**:

#### Missing Dependencies
```typescript
// ❌ Bad - missing 'value' dependency
useEffect(() => {
  console.log(value)
}, [])

// ✅ Good
useEffect(() => {
  console.log(value)
}, [value])
```

#### Unstable Dependencies
```typescript
// ❌ Bad - handleClick changes every render
useEffect(() => {
  // ...
}, [handleClick])

// ✅ Good - wrap in useCallback
const handleClick = useCallback(() => {
  // ...
}, [/* stable deps */])
```

### 3. Refactor Derived State Patterns

**Files with derived state in effects** (found by our analyzer):
- `apps/frontend/src/features/preview/components/preview-toolbar.tsx:41`
- `apps/frontend/src/features/preview/components/preview.tsx:42`
- `apps/frontend/src/features/tasks/contexts/task-events-provider.tsx:118`

**Pattern to apply**:
```typescript
// ❌ Bad
const [derived, setDerived] = useState()
useEffect(() => {
  setDerived(calculateFrom(props))
}, [props])

// ✅ Good
const derived = useMemo(() => calculateFrom(props), [props])
```

### 4. Fix Specific High-Priority Components

#### GlobalEventStreamProvider
**File**: `apps/frontend/src/features/events/contexts/global-event-stream-provider.tsx`
- Missing `connect` and `disconnect` dependencies
- Review if these should be stable references or if the effect needs adjustment

#### ChatInput Component
**File**: `apps/frontend/src/features/tasks/components/chat-input/index.tsx`
- Line 193: Missing `templateSelector.value` dependency
- Check if this causes stale template selection

### 5. Automated Fixes

Some issues can be auto-fixed by Biome:
```bash
# Preview fixes first
pnpm exec biome check apps/frontend/src --write --unsafe

# Apply fixes after review
pnpm exec biome check apps/frontend/src --write
```

**WARNING**: Review each auto-fix carefully as they might change behavior!

## Testing Strategy

After fixing each component:
1. Test the specific functionality manually
2. Check for infinite render loops using React DevTools Profiler
3. Verify no regression in behavior
4. Run the test suite: `pnpm test`

## Success Criteria

1. `pnpm -w run biome:check` shows 0 React hooks violations
2. No infinite render loops or performance regressions
3. All tests pass
4. Key user flows work correctly:
   - Task creation and chat
   - File preview
   - Git diff viewing
   - Event streaming

## Additional Resources

- Performance analysis scripts: `/scripts/analyze-performance.ts`
- React performance debugging guide: `/docs/debugging/react-performance-issues.md`
- Example fixes already applied:
  - `MinifiedLastEventsDisplay` (commit: f355dd6)
  - `GitDiffViewer` (commit: 9c8e3b2)
  - `TaskUIProvider` (commit: 9a28c13)

## Priority Order

1. Fix `useHookAtTopLevel` violations first (breaks React rules)
2. Fix missing dependencies that could cause bugs
3. Refactor derived state patterns for performance
4. Fix unnecessary dependencies last

## Notes

- Some "violations" might be intentional (e.g., one-time mount effects). Document these with comments:
  ```typescript
  useEffect(() => {
    loadOnce()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []) // Intentionally run once on mount
  ```

- If you find more instances of the infinite loop pattern we fixed in `TaskUIProvider`, apply the same solution (memoize context values properly)

Good luck! The codebase will be much more robust after these fixes.