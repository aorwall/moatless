# SSE Streaming Verification Guide

## How to Verify SSE Streaming is Working

### 1. Browser Console Logging

When you open a task detail page, you should see console logs like:

```
[SSE] Starting streaming for task <taskId> {taskStatus: "open", agentStatus: "idle", workspaceStatus: "ready"}
[NDJSON-SSE] Connecting to task <taskId>
[NDJSON-SSE] Making request to http://localhost:8000/api/v1/tasks/<taskId>/stream
[NDJSON-SSE] Response status: 200
[SSE] Connected for task <taskId>
```

### 2. Network Tab Monitoring

1. Open Chrome DevTools (F12)
2. Go to the Network tab
3. Filter by "stream" or look for requests to `/api/v1/tasks/{taskId}/stream`
4. You should see:
   - Request with status 200
   - Type: `x-ndjson` or `eventsource`
   - The connection should stay open (not immediately close)
   - In the Response tab, you should see streaming data coming in

### 3. Server Logs

Check server logs for:
```
INFO: SSE connection sse_XX established for task <taskId>
```

If connection closes immediately, you'll see:
```
INFO: SSE connection sse_XX closed for task <taskId>
```

### 4. Manual Testing Steps

1. **Start the server**:
   ```bash
   pnpm dev
   ```

2. **Create a new task**:
   - Go to the UI
   - Create a new task
   - Note the task ID from the URL

3. **Monitor the connection**:
   - Open browser console
   - Open Network tab
   - Navigate to the task detail page
   - Watch for SSE connection logs

4. **Trigger events**:
   - Send a message to the task
   - Watch for real-time events appearing

### 5. Automated Test

Run this in the browser console while on a task page:

```javascript
// Get the current task ID from URL
const taskId = window.location.pathname.split('/').pop();

// Create a test SSE connection
const eventSource = new EventSource(`http://localhost:8000/api/v1/tasks/${taskId}/stream`);

eventSource.onopen = () => {
  console.log('✅ SSE Connection opened successfully');
};

eventSource.onmessage = (event) => {
  console.log('📨 Received event:', JSON.parse(event.data));
};

eventSource.onerror = (error) => {
  console.error('❌ SSE Error:', error);
  if (eventSource.readyState === EventSource.CLOSED) {
    console.log('Connection closed');
  }
};

// Keep connection open for 30 seconds then close
setTimeout(() => {
  eventSource.close();
  console.log('Test connection closed after 30 seconds');
}, 30000);
```

### 6. Expected Behavior

- ✅ Connection stays open
- ✅ Ping events received every 30 seconds
- ✅ Real-time events when task is running
- ✅ Connection closes gracefully when navigating away
- ✅ Reconnects automatically on connection loss

### 7. Common Issues to Check

1. **Connection closes immediately**:
   - Check task status conditions
   - Verify agent/workspace status
   - Check server error logs

2. **No events received**:
   - Verify task has events
   - Check event subscription in server
   - Ensure proper event formatting

3. **CORS errors**:
   - Check server CORS configuration
   - Verify API URL matches

4. **Network errors**:
   - Check if server is running
   - Verify port numbers
   - Check for proxy issues