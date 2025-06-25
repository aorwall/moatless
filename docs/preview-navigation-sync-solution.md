# Preview Navigation Sync Solution Design

## Problem Statement

The current bolt.diy preview system does not synchronize the address bar with navigation that occurs inside the preview iframe. When users click links or navigate within the preview, the address bar remains unchanged, leading to a disconnected user experience.

## Current State Analysis

### Existing Components

1. **Preview.tsx** (`app/components/workbench/Preview.tsx`)
   - Manages preview iframe and address bar UI
   - Handles manual URL entry via address bar
   - Already has message event listeners for inspector functionality
   - Uses `displayPath` state for address bar content
   - Uses `iframeUrl` state for iframe src attribute

2. **Inspector Script** (`public/inspector-script.js`)
   - Already injected into all preview iframes via WebContainer
   - Uses postMessage for parent-iframe communication
   - Provides a proven pattern for script injection

3. **WebContainer Integration** (`app/lib/webcontainer/index.ts`)
   - Provides `setPreviewScript()` method for script injection
   - Handles preview URL generation
   - Manages file watching and preview refreshes

4. **Message Handling Infrastructure**
   - Existing postMessage handlers in Preview.tsx
   - BroadcastChannel usage for cross-tab synchronization
   - Preview message types already defined

### Current Limitations

1. No tracking of navigation events inside iframe
2. Address bar doesn't update on iframe navigation
3. No browser history integration
4. No deep linking support
5. Cross-origin restrictions limit direct iframe access

## Proposed Solution

### Architecture Overview

```
┌─────────────────────┐
│   Preview.tsx       │
│  ┌─────────────┐    │
│  │ Address Bar │◄───┼──── User Input
│  └─────────────┘    │
│         ▲            │
│         │            │
│    displayPath       │
│         │            │
│  ┌──────┴────────┐  │
│  │ Message       │  │
│  │ Handler       │◄─┼──── postMessage
│  └───────────────┘  │
└─────────────────────┘
          ▲
          │
    postMessage
          │
┌─────────┴───────────┐
│   Preview iFrame    │
│  ┌───────────────┐  │
│  │ Navigation    │  │
│  │ Tracker       │  │
│  │ Script        │  │
│  └───────────────┘  │
│         │           │
│    Listen for:      │
│    - pushState     │
│    - replaceState  │
│    - popstate      │
│    - hashchange    │
│    - load          │
└─────────────────────┘
```

### Implementation Steps

#### Step 1: Create Navigation Tracking Script

Create a new file `public/navigation-tracker.js`:

```javascript
(function() {
  'use strict';
  
  // Store original methods
  const originalPushState = history.pushState;
  const originalReplaceState = history.replaceState;
  
  // Helper to extract path from URL
  function getPathFromUrl(url) {
    try {
      const urlObj = new URL(url, window.location.origin);
      return urlObj.pathname + urlObj.search + urlObj.hash;
    } catch {
      return url;
    }
  }
  
  // Send navigation update to parent
  function sendNavigationUpdate() {
    const path = window.location.pathname + window.location.search + window.location.hash;
    window.parent.postMessage({
      type: 'NAVIGATION_UPDATE',
      path: path,
      url: window.location.href,
      title: document.title
    }, '*');
  }
  
  // Override pushState
  history.pushState = function(state, title, url) {
    originalPushState.apply(history, arguments);
    sendNavigationUpdate();
  };
  
  // Override replaceState
  history.replaceState = function(state, title, url) {
    originalReplaceState.apply(history, arguments);
    sendNavigationUpdate();
  };
  
  // Listen for popstate (back/forward)
  window.addEventListener('popstate', sendNavigationUpdate);
  
  // Listen for hashchange
  window.addEventListener('hashchange', sendNavigationUpdate);
  
  // Send initial navigation info
  if (document.readyState === 'complete') {
    sendNavigationUpdate();
  } else {
    window.addEventListener('load', sendNavigationUpdate);
  }
  
  // Signal that navigation tracker is ready
  window.parent.postMessage({ type: 'NAVIGATION_TRACKER_READY' }, '*');
})();
```

#### Step 2: Inject Navigation Tracker Script

Update `app/lib/webcontainer/index.ts` to include navigation tracker:

```typescript
// In the startShell() method or initialization
await webcontainer.setPreviewScript(`
  ${await readFile('public/inspector-script.js', 'utf-8')}
  ${await readFile('public/navigation-tracker.js', 'utf-8')}
`);
```

#### Step 3: Update Preview.tsx Message Handler

Add navigation message handling to the existing message handler in Preview.tsx:

```typescript
// In the handleMessage function (around line 623)
const handleMessage = (event: MessageEvent) => {
  if (event.data.type === 'INSPECTOR_READY') {
    // ... existing inspector code ...
  } else if (event.data.type === 'INSPECTOR_CLICK') {
    // ... existing inspector code ...
  } else if (event.data.type === 'NAVIGATION_UPDATE') {
    // Update the displayed path when navigation occurs in iframe
    const { path } = event.data;
    setDisplayPath(path);
    
    // Optionally broadcast to other tabs
    if (broadcastChannel) {
      broadcastChannel.postMessage({
        type: 'navigation',
        previewId: activePreview?.id,
        path: path
      });
    }
  } else if (event.data.type === 'NAVIGATION_TRACKER_READY') {
    console.log('[Preview] Navigation tracker initialized');
  }
};
```

#### Step 4: Handle Edge Cases

1. **Initial Load Synchronization**
   - When iframe loads initially, wait for NAVIGATION_UPDATE to get actual path
   - Update displayPath accordingly

2. **Manual URL Entry**
   - Keep existing functionality for manual address bar input
   - Ensure manual navigation still works as expected

3. **Cross-Origin Restrictions**
   - The solution works because scripts are injected into the same origin
   - No CORS issues with postMessage communication

4. **Performance Considerations**
   - Debounce rapid navigation updates if needed
   - Minimize message payload size

### Optional Enhancements

#### 1. Browser History Integration

```typescript
// In Preview.tsx
useEffect(() => {
  const handleBrowserNavigation = (e: PopStateEvent) => {
    if (e.state?.previewPath) {
      setDisplayPath(e.state.previewPath);
      setIframeUrl(activePreview.baseUrl + e.state.previewPath);
    }
  };
  
  window.addEventListener('popstate', handleBrowserNavigation);
  return () => window.removeEventListener('popstate', handleBrowserNavigation);
}, [activePreview]);

// When navigation occurs
const updateBrowserHistory = (path: string) => {
  window.history.pushState(
    { previewPath: path },
    '',
    `#preview${path}`
  );
};
```

#### 2. Multi-Tab Synchronization

```typescript
// Listen for navigation broadcasts from other tabs
useEffect(() => {
  const channel = new BroadcastChannel('preview-navigation');
  
  channel.onmessage = (event) => {
    if (event.data.type === 'navigation' && 
        event.data.previewId === activePreview?.id) {
      setDisplayPath(event.data.path);
    }
  };
  
  return () => channel.close();
}, [activePreview]);
```

#### 3. Loading State Indicator

```typescript
// Add loading state when navigation is in progress
const [isNavigating, setIsNavigating] = useState(false);

// In message handler
} else if (event.data.type === 'NAVIGATION_START') {
  setIsNavigating(true);
} else if (event.data.type === 'NAVIGATION_UPDATE') {
  setIsNavigating(false);
  setDisplayPath(event.data.path);
}
```

## Testing Strategy

### Unit Tests
1. Test navigation tracker script functions
2. Test message handling in Preview component
3. Test path parsing and URL construction

### Integration Tests
1. Test navigation via clicking links
2. Test programmatic navigation (pushState/replaceState)
3. Test browser back/forward buttons
4. Test hash navigation
5. Test manual address bar input

### Edge Case Tests
1. Test rapid navigation
2. Test navigation during iframe reload
3. Test invalid URLs
4. Test navigation with query parameters and hashes
5. Test navigation in different preview ports

## Implementation Priority

1. **Phase 1 (Core Functionality)**
   - Navigation tracking script
   - Basic message handling
   - Address bar synchronization

2. **Phase 2 (Enhancements)**
   - Browser history integration
   - Loading states
   - Error handling

3. **Phase 3 (Advanced Features)**
   - Multi-tab synchronization
   - Navigation breadcrumbs
   - URL sharing/copying

## Security Considerations

1. **Message Validation**
   - Validate message origin if needed
   - Sanitize paths before display
   - Prevent XSS through URL manipulation

2. **Script Injection**
   - Scripts run in iframe context
   - No access to parent window directly
   - Communication only via postMessage

3. **URL Handling**
   - Validate URLs before navigation
   - Prevent navigation to external domains
   - Handle malformed URLs gracefully

## Performance Impact

- Minimal overhead from navigation tracking
- PostMessage is asynchronous and non-blocking
- No polling or continuous monitoring needed
- Event-driven architecture ensures efficiency

## Conclusion

This solution provides seamless navigation synchronization between the preview iframe and the address bar with minimal changes to the existing codebase. It leverages existing patterns (inspector script) and infrastructure (postMessage, WebContainer preview scripts) to deliver a robust navigation tracking system that enhances the user experience significantly.