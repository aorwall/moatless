# Preview Proxy Architecture

## Overview

The preview environment has been refactored to use a proxy approach instead of exposing preview servers on specific ports. This provides better security, easier configuration, and unified access through the backend API.

## Architecture Changes

### Before (Direct Port Access)
- Preview servers ran on dynamically allocated ports (e.g., 3000, 3001)
- Frontend connected directly to `http://localhost:PORT`
- Required port management and exposure in Docker environments
- Potential security concerns with exposing multiple ports

### After (Proxy Architecture)
- Preview servers still run on local ports internally
- All preview traffic is proxied through `/api/environments/:taskId/proxy/*`
- Frontend uses relative proxy URLs instead of direct localhost URLs
- No external port exposure required

## Implementation Details

### Backend Components

1. **Proxy Endpoint** (`/api/environments/:taskId/proxy/*`)
   - Handles all HTTP methods (GET, POST, etc.)
   - Forwards requests to the preview server
   - Filters hop-by-hop headers
   - Handles binary and text responses appropriately

2. **Preview Server Proxy Method** (`PreviewServer.proxyRequest()`)
   - Validates server is running
   - Forwards requests to localhost:PORT
   - Returns response with proper headers and body

3. **Environment Proxy Methods**
   - `LocalEnvironment.proxyPreviewRequest()` - Direct proxy to preview server
   - `DockerEnvironment.proxyPreviewRequest()` - Proxy through worker endpoint
   - Both implement the base environment interface

### Frontend Changes

1. **Preview Component**
   - Uses proxy URL: `/api/environments/${taskId}/proxy${path}`
   - Removed direct connection checking (server status determines connectivity)
   - Simplified error handling

2. **Removed Components**
   - `usePreviewConnection` hook no longer needed for proxy approach
   - Direct URL configuration removed from preview config

## Benefits

1. **Security**
   - No exposed ports on host machine
   - All traffic goes through authenticated API endpoints
   - Better control over allowed requests

2. **Simplicity**
   - No port allocation complexity for frontend
   - Works seamlessly across local and Docker environments
   - No CORS issues

3. **Flexibility**
   - Can add request/response transformations
   - Easy to add authentication headers
   - Can inject scripts or modify responses if needed

## Usage Example

```typescript
// Frontend iframe now uses proxy URL
<iframe src={`/api/environments/${taskId}/proxy/`} />

// Backend handles the proxy request
app.all('/api/environments/:taskId/proxy/*', async (c) => {
  const response = await workspace.proxyPreviewRequest({
    method: c.req.method,
    path: proxyPath,
    headers: c.req.headers,
    body: c.req.body
  })
  
  return new Response(response.body, {
    status: response.status,
    headers: response.headers
  })
})
```

## Migration Notes

- Server status no longer returns a URL (frontend uses proxy endpoint)
- Preview port allocation still happens internally but is not exposed
- All existing preview functionality remains the same from user perspective