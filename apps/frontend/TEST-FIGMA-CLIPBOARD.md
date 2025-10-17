# Figma Clipboard Paste Testing

This directory contains tools for testing the `useFigmaClipboardPaste` hook with the test fixture data.

## Quick Start

### Option 1: Using the Test Server (Recommended)

1. **Start the test server:**
   ```bash
   node test-figma-server.cjs
   ```

2. **Open your browser:**
   The server runs at `http://localhost:8765/` and will be automatically opened

3. **Copy test data:**
   Click the "📋 Copy Figma Test Data" button on the page

4. **Test in your app:**
   - Go to your running app at `http://localhost:3000`
   - Find any textarea with the Figma paste handler
   - Press `Cmd+V` (Mac) or `Ctrl+V` (Windows/Linux)

5. **Or test on the page:**
   Paste directly in the textarea on the test page to verify the clipboard data

### Option 2: Using the Shell Script

```bash
./test-figma-clipboard.sh
```

This opens the test page directly in your browser (may have CORS issues with fixture loading).

## Files

- **`test-figma-paste.html`** - Interactive test page with copy/paste functionality
- **`test-figma-server.cjs`** - Simple HTTP server for serving the test page and fixtures
- **`test-figma-clipboard.sh`** - Shell script to quickly open the test page

## How It Works

### The Hook (`use-figma-clipboard-paste.ts`)

The hook:
1. Listens for `onPaste` events on textarea elements
2. Checks for `text/html` MIME type in clipboard data
3. Looks for Figma-specific markers: `data-metadata` and `data-buffer`
4. If detected, calls `figmaApi.importClipboard(html)` with the HTML content

### The Test Fixture

Located at: `../backend/src/services/__tests__/fixtures/test.html`

This file contains real Figma clipboard data in HTML format with:
- `<span data-metadata>` - Contains Figma metadata (base64 encoded)
- `<span data-buffer>` - Contains the actual design data (fig-kiwie format, base64 encoded)

### The Test Page

The test page:
1. Loads the fixture HTML via fetch
2. Copies it to clipboard as `text/html` MIME type using the Clipboard API
3. Provides a test textarea to verify the paste event
4. Shows detection results (HTML type, Figma markers, etc.)

## Troubleshooting

### CORS Issues
If you see CORS errors when loading the fixture:
- Use the test server (`node test-figma-server.cjs`) instead of opening the HTML file directly
- The server properly handles cross-origin requests

### Clipboard API Issues
The Clipboard API requires:
- HTTPS or localhost (for security)
- User interaction (button click) to write to clipboard
- Modern browser support

### Browser Compatibility
Tested on:
- Chrome/Edge (Clipboard API fully supported)
- Firefox (Clipboard API supported)
- Safari (Clipboard API supported with limitations)

## Example Usage in Your App

```tsx
import { useFigmaClipboardPaste } from '@/features/figma/hooks/use-figma-clipboard-paste'

function MyComponent() {
  const handlePaste = useFigmaClipboardPaste({
    onSuccess: (result) => {
      console.log('Figma design imported:', result.root.name)
      // Handle the parsed design data
    },
    onError: (error) => {
      console.error('Failed to import:', error)
    }
  })

  return (
    <textarea
      onPaste={handlePaste}
      placeholder="Paste Figma design here..."
    />
  )
}
```

## API Response

The `figmaApi.importClipboard()` returns a `FigmaImportResponse` with:
- `root` - The root design node
- Other parsed design data from the Figma clipboard

Check `apps/frontend/src/features/figma/api/figma.ts` for the full response type.
