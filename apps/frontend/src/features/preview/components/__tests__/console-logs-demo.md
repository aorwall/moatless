# Console Logs Component Test Demonstration

This document demonstrates how the console logs component works and how it filters logs.

## Key Features Tested

### 1. Console logs from the preview iframe ARE captured
When the preview iframe sends console messages via postMessage with:
- `type: 'console'`
- `source: 'preview-iframe'`
- `level: 'log' | 'error' | 'warn' | 'info' | 'debug'`
- `message: string`

These messages are displayed in the ConsoleLogs component.

### 2. Console logs from the main app are NOT captured
Regular console.log/error/warn/info calls from the React app itself are NOT displayed in the ConsoleLogs component. This ensures that only logs from the preview iframe are shown.

### 3. Implementation Details

#### Preview Component (preview.tsx)
When the iframe loads, it injects a script that:
1. Overrides console methods (log, error, warn, info, debug)
2. Sends console messages to the parent window via postMessage
3. Captures uncaught errors and unhandled promise rejections

```javascript
// Injected into iframe
console.log = function(...args) {
  originalConsole.log.apply(console, args);
  window.parent.postMessage({
    type: 'console',
    source: 'preview-iframe',
    level: 'log',
    message: formatArgs(args)
  }, '*');
};
```

#### Console Logs Component (console-logs.tsx)
1. Listens for postMessage events
2. Filters messages by type='console' and source='preview-iframe'
3. Displays filtered logs with appropriate styling
4. Clears logs when preview URL changes
5. Auto-scrolls to bottom when new logs arrive

## Test Scenarios

### Scenario 1: Preview App Logs ARE Shown
```javascript
// In preview iframe
console.log('Server started on port 3000');
console.error('Failed to connect to database');
console.warn('Deprecated API usage');

// Result: All three messages appear in ConsoleLogs component
```

### Scenario 2: Main App Logs are NOT Shown
```javascript
// In main React app
console.log('React component rendered');
console.error('API request failed');

// Result: These messages do NOT appear in ConsoleLogs component
```

### Scenario 3: Different Log Levels Have Different Styles
- `log` → text-slate-100
- `error` → text-red-400
- `warn` → text-yellow-400
- `info` → text-blue-400
- `debug` → text-gray-500

### Scenario 4: Logs Clear on URL Change
When the preview URL changes (e.g., navigating to a different page), all previous logs are cleared to provide a clean slate for the new page.

## Security Considerations

The implementation uses postMessage with a specific protocol:
- Only messages with `type: 'console'` are processed
- Only messages with `source: 'preview-iframe'` are accepted
- This prevents other windows/iframes from polluting the console logs