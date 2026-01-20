---
name: sandbox-browser
description: Browser automation for sandbox preview servers using Playwright. Use when automating browser interactions, taking screenshots, getting ARIA snapshots, or testing web UI in sandboxed environments.
---

# Sandbox Browser Automation

Browser automation for sandbox preview servers using the browser-client TypeScript library.

## Quick Start

```bash
bun x tsx <<'EOF'
import { connect, waitForPageLoad, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

const client = await connect();
const page = await client.page("main");

await page.goto(getPreviewUrl());
await waitForPageLoad(page);

const snapshot = await client.getAISnapshot("main");
console.log(snapshot);

await client.disconnect();
EOF
```

## Browser Client API

### Connecting

```typescript
import { connect, waitForPageLoad, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

// Default viewport (1280x720)
const client = await connect();

// Custom default viewport
const client = await connect({ viewport: { width: 1920, height: 1080 } });
```

### Page Management

```typescript
// Get or create a named page
const page = await client.page("main");

// With options
const page = await client.page("test", {
  url: getPreviewUrl(),           // Navigate on creation
  disableAutoCapture: true        // Disable auto screenshots
});

// With custom viewport for this page
const mobilePage = await client.page("mobile", {
  viewport: { width: 375, height: 812 }  // iPhone X
});

// Change viewport later
await mobilePage.setViewportSize({ width: 768, height: 1024 });  // iPad

// List all pages
const pages = await client.list();

// Close a page
await client.close("main");

// Disconnect (browser stays alive for next script)
await client.disconnect();
```

### Navigation

```typescript
// Use getPreviewUrl() helper for preview server URLs
await page.goto(getPreviewUrl());              // http://localhost:3000
await page.goto(getPreviewUrl("/about"));      // http://localhost:3000/about
await page.goto(getPreviewUrl("contact"));     // http://localhost:3000/contact

await waitForPageLoad(page);
```

### ARIA Snapshots

ARIA snapshots are the preferred way to understand page structure:

```typescript
const snapshot = await client.getAISnapshot("main");
console.log(snapshot);
```

Output example:
```yaml
- document "My App"
  - navigation
    - link "Home"
    - link "About"
  - main
    - heading "Welcome" (level=1)
    - button "Click me"
    - textbox "Email"
```

### Interacting with Elements

Use standard Playwright selectors:

```typescript
// Click
await page.click('button[type="submit"]');
await page.click('a:has-text("About")');

// Fill inputs
await page.fill('input[name="email"]', "test@example.com");
await page.fill('textarea', "Message content");

// Select dropdown
await page.selectOption('select[name="country"]', "US");

// Check/uncheck
await page.check('input[name="agree"]');
await page.uncheck('input[name="newsletter"]');
```

### Screenshots

Auto-capture is enabled by default after `goto()`, `click()`, `fill()`.

```typescript
// Manual capture
await client.captureScreenshot("main", "manual");

// Take screenshot as base64
import { takeScreenshot } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";
const base64 = await takeScreenshot(page);
```

Screenshots are saved to `/data/screenshots/` with metadata JSON files.

### Waiting

```typescript
// Wait for page load
await waitForPageLoad(page);
await waitForPageLoad(page, { waitUntil: 'networkidle', timeout: 10000 });

// Wait for element
await page.waitForSelector(".success-message");
await page.waitForSelector('[role="dialog"]', { state: 'hidden' });

// Wait for URL
await page.waitForURL("**/dashboard");

// Wait for network
await page.waitForResponse(r => r.url().includes('/api/data'));
```

## Common Patterns

### Form Submission

```typescript
const client = await connect();
const page = await client.page("form");

await page.goto(getPreviewUrl("/contact"));
await waitForPageLoad(page);

await page.fill('input[name="name"]', "John Doe");
await page.fill('input[name="email"]', "john@example.com");
await page.fill('textarea[name="message"]', "Hello!");
await page.click('button[type="submit"]');

await page.waitForSelector(".success-message");
console.log(await client.getAISnapshot("form"));

await client.disconnect();
```

### Content Verification

```typescript
const client = await connect();
const page = await client.page("verify");

await page.goto(getPreviewUrl());
await waitForPageLoad(page);

const snapshot = await client.getAISnapshot("verify");
if (snapshot.includes('heading "Welcome"')) {
  console.log("Page loaded correctly");
}

await client.disconnect();
```

### Navigation Testing

```typescript
const client = await connect();
const page = await client.page("nav");

await page.goto(getPreviewUrl());
await waitForPageLoad(page);

await page.click('a:has-text("About")');
await waitForPageLoad(page);

console.log("URL:", page.url());
console.log(await client.getAISnapshot("nav"));

await client.disconnect();
```

### Mobile Testing

```typescript
const client = await connect();

// Create mobile-sized page
const page = await client.page("mobile", {
  viewport: { width: 375, height: 812 }
});

await page.goto(getPreviewUrl());
await waitForPageLoad(page);

// Check mobile layout
const snapshot = await client.getAISnapshot("mobile");
console.log(snapshot);

await client.disconnect();
```

## References

- [Script Examples](references/script-examples.md) - More usage patterns
