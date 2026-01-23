---
name: sandbox-browser
description: Browser automation for sandbox preview servers using CDP. Use when automating browser interactions, taking screenshots, or testing web UI in sandboxed environments. Agent scripts interactions directly via evaluate().
---

# Sandbox Browser Automation

Low-level browser automation for sandbox preview servers. The agent scripts interactions directly via `evaluate()` - no high-level abstractions.

## Quick Start

```bash
cat > /tmp/browser-test.ts <<'EOF'
import { connect, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

const client = await connect();
const page = await client.page("main");

await page.goto(getPreviewUrl());

// Script interactions via evaluate()
const title = await page.evaluate(() => document.title);
console.log("Title:", title);

await client.disconnect();
EOF

bun run /tmp/browser-test.ts
```

## Browser Client API

### Connecting

```typescript
import { connect, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

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
// goto() already waits for domcontentloaded - no additional wait needed
await page.goto(getPreviewUrl());              // http://localhost:3000
await page.goto(getPreviewUrl("/about"));      // http://localhost:3000/about
await page.goto(getPreviewUrl("contact"));     // http://localhost:3000/contact
```

### Interactions via evaluate()

All DOM interactions are done via `evaluate()` - the agent scripts directly:

```typescript
// Click elements
await page.evaluate(() => document.querySelector('button')?.click());
await page.evaluate(() => document.querySelector('a[href="/about"]')?.click());

// Fill inputs
await page.evaluate((sel, val) => {
  const el = document.querySelector(sel) as HTMLInputElement;
  el.value = val;
  el.dispatchEvent(new Event('input', { bubbles: true }));
}, 'input[name="email"]', 'test@example.com');

// Get text content
const heading = await page.evaluate(
  (selector) => document.querySelector(selector)?.textContent,
  'h1'
);

// Get attribute
const href = await page.evaluate(
  (selector, attr) => document.querySelector(selector)?.getAttribute(attr),
  'a.logo', 'href'
);

// Check visibility
const isVisible = await page.evaluate((selector) => {
  const el = document.querySelector(selector);
  if (!el) return false;
  const style = getComputedStyle(el);
  return style.display !== 'none' && style.visibility !== 'hidden';
}, '.modal');

// Select dropdown option
await page.evaluate((selector, value) => {
  const select = document.querySelector(selector) as HTMLSelectElement;
  select.value = value;
  select.dispatchEvent(new Event('change', { bubbles: true }));
}, 'select#country', 'US');

// Get all link URLs
const links = await page.evaluate(() =>
  Array.from(document.querySelectorAll('a')).map(a => a.href)
);
```

### Screenshots

```typescript
// Manual capture with metadata
await client.captureScreenshot("main", "manual");

// Take screenshot as base64
import { takeScreenshot } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";
const base64 = await takeScreenshot(page);

// Direct screenshot to buffer
const buffer = await page.screenshot({ type: 'png' });
```

Screenshots are saved to `/data/screenshots/` with metadata JSON files.

### Waiting

```typescript
// Wait for a fixed timeout (useful for animations/transitions)
await page.waitForTimeout(500);

// Wait for page load state (usually not needed after goto())
await page.waitForLoadState('domcontentloaded');
await page.waitForLoadState('load');
await page.waitForLoadState('networkidle');
```

### Waiting for Elements

Wait for elements to appear, disappear, or change visibility state:

```typescript
// Wait for element to be visible (default)
await page.waitForSelector('.modal');

// Wait for element to be hidden (e.g., loading spinner)
await page.waitForSelector('.spinner', { state: 'hidden' });

// Wait for element to exist in DOM
await page.waitForSelector('.item', { state: 'attached' });

// Wait for element to be removed from DOM
await page.waitForSelector('.notification', { state: 'detached' });

// Custom timeout
await page.waitForSelector('.results', { state: 'visible', timeout: 5000 });
```

### Advanced: Raw CDP Access

For edge cases not covered by the high-level API, use raw CDP commands:

```typescript
// Get CDP session
const cdp = page.getCdpSession();

// Enable network domain
await cdp.send('Network.enable', {});

// Get all cookies
const { cookies } = await cdp.send<{ cookies: unknown[] }>('Network.getCookies', {});

// Set extra HTTP headers
await cdp.send('Network.setExtraHTTPHeaders', {
  headers: { 'X-Custom-Header': 'value' }
});

// Emulate media features (e.g., dark mode)
await cdp.send('Emulation.setEmulatedMedia', {
  features: [{ name: 'prefers-color-scheme', value: 'dark' }]
});
```

## Common Patterns

### Form Submission

```typescript
const client = await connect();
const page = await client.page("form");

await page.goto(getPreviewUrl("/contact"));

// Fill form via evaluate()
await page.evaluate(() => {
  const form = document.querySelector('form');
  (form?.querySelector('input[name="name"]') as HTMLInputElement).value = "John Doe";
  (form?.querySelector('input[name="email"]') as HTMLInputElement).value = "john@example.com";
  (form?.querySelector('textarea[name="message"]') as HTMLTextAreaElement).value = "Hello!";
});

// Submit
await page.evaluate(() => document.querySelector('button[type="submit"]')?.click());

// Wait for any transitions
await page.waitForTimeout(500);

await client.disconnect();
```

### Content Verification

```typescript
const client = await connect();
const page = await client.page("verify");

await page.goto(getPreviewUrl());

const title = await page.evaluate(() => document.title);
const hasNav = await page.evaluate(() => !!document.querySelector('nav'));
const buttonText = await page.evaluate(() =>
  document.querySelector('button')?.textContent
);

console.log({ title, hasNav, buttonText });

await client.disconnect();
```

### Navigation Testing

```typescript
const client = await connect();
const page = await client.page("nav");

await page.goto(getPreviewUrl());

// Click navigation link
await page.evaluate(() => document.querySelector('a[href="/about"]')?.click());
await page.waitForTimeout(500);  // Wait for client-side routing

console.log("URL:", page.url());
const heading = await page.evaluate(() => document.querySelector('h1')?.textContent);
console.log("Heading:", heading);

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

// Check mobile-specific elements
const hasHamburgerMenu = await page.evaluate(() =>
  !!document.querySelector('.hamburger-menu')
);
console.log("Has hamburger menu:", hasHamburgerMenu);

await client.disconnect();
```

## Troubleshooting

### Import errors with heredoc execution

If you see errors like "does not provide an export named 'connect'" when using inline heredoc syntax with `bun x tsx`, use the file-based approach instead:

```bash
# Write script to file first, then run
cat > /tmp/script.ts <<'EOF'
// your code here
EOF

bun run /tmp/script.ts
```

This ensures proper TypeScript module resolution. The heredoc pattern fails because tsx's stdin evaluation creates a synthetic file path with incomplete module resolution.

## References

- [Script Examples](references/script-examples.md) - More usage patterns
