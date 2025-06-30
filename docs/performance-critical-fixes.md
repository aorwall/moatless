# 🚨 Critical Performance Fixes - Quick Implementation Guide

**⚠️ IMPORTANT: This document describes performance improvements TO BE IMPLEMENTED. These optimizations have NOT been applied to the main codebase yet.**

## The Problem: 54 Renders in 17 Seconds

Your performance data shows:
- **3.2 renders per second** (should be < 0.1)
- **Render times degrading** from 23ms to 74ms
- **Timer-based re-renders** every ~1 second

## 🔥 Fix #1: Stop Timer Re-renders (2 hours, -90% renders)

### ChatView Timer Fix

**File:** `/client/src/features/tasks/components/chat-view/chat-view.tsx`

**Current Problem (lines 82-87):**
```typescript
const [currentTime, setCurrentTime] = useState(Date.now())

useEffect(() => {
  if (isRunning && runningStartTime) {
    const interval = setInterval(() => {
      setCurrentTime(Date.now()) // ❌ Re-renders entire ChatView
    }, 1000)
    return () => clearInterval(interval)
  }
}, [isRunning, runningStartTime])
```

**Fix - Replace with:**
```typescript
// Remove the currentTime state and useEffect

// In the render, replace the duration calculation with:
{isRunning && runningStartTime && (
  <IsolatedTimer 
    startTime={runningStartTime} 
    format="duration"
    className="text-sm text-muted-foreground"
  />
)}
```

### MinifiedLastThreeEventsDisplay Timer Fix

**File:** `/client/src/features/tasks/components/event-viewer/minified/minified-last-three-events-display.tsx`

**Current Problem (lines 44-48):**
```typescript
useEffect(() => {
  if (isRunning && runningStartTime) {
    const animate = () => {
      forceUpdate()
      rafRef.current = requestAnimationFrame(() => {
        setTimeout(animate, 1000) // ❌ Re-renders every second
      })
    }
    animate()
  }
}, [isRunning, runningStartTime, forceUpdate])
```

**Fix - Replace entire duration display with:**
```typescript
// Remove the forceUpdate and animation logic

// In the render:
{isRunning && runningStartTime && (
  <span className="text-xs text-muted-foreground">
    Running for <IsolatedTimer startTime={runningStartTime} format="duration" />
  </span>
)}
```

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

### If IsolatedTimer doesn't exist, create it:

```typescript
// client/src/features/tasks/components/common/isolated-timer.tsx
import React, { useEffect, useState } from 'react'

export const IsolatedTimer = React.memo(function IsolatedTimer({ 
  startTime, 
  format = 'duration',
  className = ''
}: { 
  startTime: Date | string | number
  format?: 'duration' | 'elapsed'
  className?: string
}) {
  const [now, setNow] = useState(Date.now())
  
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [])
  
  const start = new Date(startTime).getTime()
  const elapsed = Math.max(0, now - start)
  const seconds = Math.floor(elapsed / 1000)
  const minutes = Math.floor(seconds / 60)
  const hours = Math.floor(minutes / 60)
  
  const formatted = hours > 0 
    ? `${hours}h ${minutes % 60}m`
    : minutes > 0
    ? `${minutes}m ${seconds % 60}s`
    : `${seconds}s`
  
  return <span className={className}>{formatted}</span>
})
```

## 📊 Verification Checklist

- [ ] Timer components no longer update parent components
- [ ] React DevTools shows IsolatedTimer re-rendering, not parent
- [ ] Performance monitor shows <16ms average render time
- [ ] No more ~1 second interval renders
- [ ] TaskPage renders <10 times in 20 seconds

## 🆘 If Issues Persist

1. **Check for other timers:** Search for `setInterval` in the codebase
2. **Look for polling:** Check for `refetchInterval` in React Query
3. **Verify memo is working:** Add console.log to component renders
4. **Check SSE updates:** May need to batch/debounce event updates

---

**These 4 fixes, once implemented, should reduce renders by 90% and bring render times below 16ms. Start with these before moving to more complex optimizations.**

**Remember: This is a guide for improvements to be made - none of these optimizations have been applied to the main branch yet.**