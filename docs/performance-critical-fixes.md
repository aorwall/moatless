# 🚨 Critical Performance Fixes - Quick Implementation Guide

**⚠️ IMPORTANT: This document describes performance improvements. Fix #1 (IsolatedTimer) has been implemented in commit 0cdb481. Fixes #2-4 are still TO BE IMPLEMENTED.**

## The Problem: 54 Renders in 17 Seconds

Your performance data shows:
- **3.2 renders per second** (should be < 0.1)
- **Render times degrading** from 23ms to 74ms
- **Timer-based re-renders** every ~1 second

## ✅ Fix #1: Stop Timer Re-renders (COMPLETED - Commit 0cdb481)

### What Was Fixed

1. **Created IsolatedTimer Component**
   - Location: `/client/src/lib/components/ui/isolated-timer.tsx`
   - A memoized component that encapsulates timer state
   - Updates internally without re-rendering parent components

2. **Updated ChatView**
   - Removed `currentTime` state and `setInterval` effect
   - Replaced with `<IsolatedTimer startTime={timing.startTime} />`
   - Timer updates no longer cause ChatView re-renders

3. **Updated MinifiedLastThreeEventsDisplay**
   - Removed animation frame timer logic
   - Replaced with `<IsolatedTimer startTime={runningStartTime} />`
   - Eliminated requestAnimationFrame-based re-renders

### Expected Impact
- **-90% reduction in renders**
- Timer updates isolated to IsolatedTimer component only
- Parent components remain stable while timer ticks

## 🔥 Fix #2: Add React.memo (1 hour, -50% renders)

### Wrap Heavy Components

```typescript
// TaskPage.tsx - at the bottom
export const TaskPage = React.memo(TaskPageComponent)

// EventViewer.tsx - wrap the export
export const EventViewer = React.memo(function EventViewer() {
  // ... existing code
})

// ChatView.tsx - wrap the export  
export const ChatView = React.memo(function ChatView() {
  // ... existing code
})
```

## 🔥 Fix #3: Optimize Context Usage (2 hours, -40% renders)

### TaskHeader Optimization

**File:** `/client/src/features/tasks/components/task-header.tsx`

**Current State:**
```typescript
// Currently uses full task context
const { data: task, isLoading, error } = useTask(taskId)
const { status, isRunning } = task || {}
```

**TO BE IMPLEMENTED - Replace with:**
```typescript
import { useStreamingStatus } from '@/features/tasks/hooks/optimized-hooks'
const { status, isRunning, canExecute, isComplete } = useStreamingStatus(taskId)
```

### EventViewer Optimization  

**File:** `/client/src/features/tasks/components/event-viewer/event-viewer.tsx`

**Current State:**
```typescript
// Currently subscribes to full task data
const { isLoading, error } = useTaskData()
```

**TO BE IMPLEMENTED - Replace with:**
```typescript
import { useTaskLoadingState } from '@/features/tasks/hooks/optimized-hooks'
const { isLoading, error } = useTaskLoadingState()
```

### GitDiffViewer Optimization

**File:** `/client/src/features/tasks/components/git-diff-viewer/git-diff-viewer.tsx`

**Current:**
```typescript
const { isLoading, error } = useTaskData()
```

**Fix:**
```typescript
import { useTaskLoadingState } from '@/features/tasks/hooks/optimized-hooks'
const { isLoading, error } = useTaskLoadingState()
```

## 🔥 Fix #4: Memoize Event Merging (30 mins, -30% CPU)

**File:** `/client/src/features/tasks/components/event-viewer/event-viewer.tsx`

**Current State:**
The file may have some memoization, but needs verification and potential optimization:

```typescript
// TO BE VERIFIED - Check if mergedEvents is properly memoized:
const mergedEvents = useMemo(() => 
  mergeToolEvents(events), 
  [events] // Ensure dependency array is correct
)
```

**TO BE IMPLEMENTED if missing:**
- Ensure event merging is wrapped in useMemo
- Verify dependency arrays are minimal
- Consider memoizing heavy computations in utils/merge-tool-events.ts

## 🎯 Immediate Testing

After implementing these fixes:

1. **Start the app:** `pnpm dev`
2. **Open performance monitor:** Press `Ctrl+Shift+P`
3. **Start recording**
4. **Navigate to a task page**
5. **Wait 20 seconds**
6. **Stop recording**

### Expected Results:
- **Before:** 54 renders in 17 seconds
- **After:** 5-10 renders in 17 seconds
- **Render time:** <16ms average

## 🚀 Quick Copy-Paste Implementations

### IsolatedTimer Component (Already Implemented)

The IsolatedTimer component has been implemented at:
```
/client/src/lib/components/ui/isolated-timer.tsx
```

Usage:
```typescript
import { IsolatedTimer } from '@/lib/components/ui/isolated-timer'

// In your component:
<IsolatedTimer startTime={startTime} className="text-sm" />
```

## 📊 Verification Checklist

### Fix #1 (IsolatedTimer) - COMPLETED ✅
- [x] Timer components no longer update parent components
- [x] React DevTools shows IsolatedTimer re-rendering, not parent
- [x] No more ~1 second interval renders in ChatView and MinifiedLastThreeEventsDisplay

### Remaining Optimizations - TO BE VERIFIED
- [ ] Performance monitor shows <16ms average render time
- [ ] TaskPage renders <10 times in 20 seconds
- [ ] React.memo applied to heavy components
- [ ] Context usage optimized
- [ ] Event merging properly memoized

## 🆘 If Issues Persist

1. **Check for other timers:** Search for `setInterval` in the codebase
2. **Look for polling:** Check for `refetchInterval` in React Query
3. **Verify memo is working:** Add console.log to component renders
4. **Check SSE updates:** May need to batch/debounce event updates

---

**These 4 fixes, once implemented, should reduce renders by 90% and bring render times below 16ms. Start with these before moving to more complex optimizations.**

**Remember: This is a guide for improvements to be made - none of these optimizations have been applied to the main branch yet.**