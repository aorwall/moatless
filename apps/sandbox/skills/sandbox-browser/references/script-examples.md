# Browser Automation Script Examples

## Basic Usage

### Connect and Navigate

```bash
bun x tsx <<'EOF'
import { connect, waitForPageLoad, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client";

const client = await connect();
const page = await client.page("main");

await page.goto(getPreviewUrl());
await waitForPageLoad(page);

console.log("Page title:", await page.title());
console.log("Current URL:", page.url());

await client.disconnect();
EOF
```

### Get ARIA Snapshot

```bash
bun x tsx <<'EOF'
import { connect, waitForPageLoad, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client";

const client = await connect();
const page = await client.page("main");

await page.goto(getPreviewUrl());
await waitForPageLoad(page);

const snapshot = await client.getAISnapshot("main");
console.log("=== ARIA Snapshot ===");
console.log(snapshot);

await client.disconnect();
EOF
```

### Take Screenshot

```bash
bun x tsx <<'EOF'
import { connect, waitForPageLoad, takeScreenshot, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client";
import { writeFileSync } from "fs";

const client = await connect();
const page = await client.page("main");

await page.goto(getPreviewUrl());
await waitForPageLoad(page);

const base64 = await takeScreenshot(page);
writeFileSync("screenshot.png", Buffer.from(base64, "base64"));
console.log("Screenshot saved to screenshot.png");

await client.disconnect();
EOF
```

---

## Form Interactions

### Fill and Submit Form

```bash
bun x tsx <<'EOF'
import { connect, waitForPageLoad, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client";

const client = await connect();
const page = await client.page("form");

await page.goto(getPreviewUrl("/contact"));
await waitForPageLoad(page);

// Fill form fields using Playwright API
await page.fill('input[name="name"]', "John Doe");
await page.fill('input[name="email"]', "john@example.com");
await page.fill('textarea[name="message"]', "Hello, this is a test message!");

// Select dropdown
await page.selectOption('select[name="subject"]', "support");

// Check checkbox
await page.check('input[name="newsletter"]');

// Submit form
await page.click('button[type="submit"]');

// Wait for success message
await page.waitForSelector(".success-message", { timeout: 5000 });

console.log("Form submitted successfully!");
const snapshot = await client.getAISnapshot("form");
console.log(snapshot);

await client.disconnect();
EOF
```

### Login Flow

```bash
bun x tsx <<'EOF'
import { connect, waitForPageLoad, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client";

const client = await connect();
const page = await client.page("login");

await page.goto(getPreviewUrl("/login"));
await waitForPageLoad(page);

// Enter credentials
await page.fill('input[type="email"]', "user@example.com");
await page.fill('input[type="password"]', "password123");

// Click login button
await page.click('button:has-text("Login")');

// Wait for redirect to dashboard
await page.waitForURL("**/dashboard", { timeout: 10000 });

console.log("Login successful!");
console.log("Redirected to:", page.url());

await client.disconnect();
EOF
```

---

## Navigation Testing

### Click Through Navigation

```bash
bun x tsx <<'EOF'
import { connect, waitForPageLoad, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client";

const client = await connect();
const page = await client.page("nav");

await page.goto(getPreviewUrl());
await waitForPageLoad(page);

// Get initial snapshot
console.log("=== Home Page ===");
console.log(await client.getAISnapshot("nav"));

// Navigate to About
await page.click('a:has-text("About")');
await waitForPageLoad(page);
console.log("\n=== About Page ===");
console.log(await client.getAISnapshot("nav"));

// Navigate to Contact
await page.click('a:has-text("Contact")');
await waitForPageLoad(page);
console.log("\n=== Contact Page ===");
console.log(await client.getAISnapshot("nav"));

await client.disconnect();
EOF
```

### Test All Links

```bash
bun x tsx <<'EOF'
import { connect, waitForPageLoad, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client";

const client = await connect();
const page = await client.page("links");

await page.goto(getPreviewUrl());
await waitForPageLoad(page);

// Get all navigation links
const links = await page.$$eval('nav a', (anchors) =>
  anchors.map(a => ({ text: a.textContent, href: a.href }))
);

console.log("Found links:", links);

// Test each link
for (const link of links) {
  console.log(`\nTesting: ${link.text} -> ${link.href}`);
  await page.goto(link.href);
  await waitForPageLoad(page);
  console.log("Title:", await page.title());
}

await client.disconnect();
EOF
```

---

## Content Verification

### Check for Element

```bash
bun x tsx <<'EOF'
import { connect, waitForPageLoad, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client";

const client = await connect();
const page = await client.page("verify");

await page.goto(getPreviewUrl());
await waitForPageLoad(page);

// Check if elements exist
const hasHeader = await page.$('h1') !== null;
const hasNav = await page.$('nav') !== null;
const hasFooter = await page.$('footer') !== null;

console.log("Element checks:");
console.log("  Header (h1):", hasHeader ? "PASS" : "FAIL");
console.log("  Navigation:", hasNav ? "PASS" : "FAIL");
console.log("  Footer:", hasFooter ? "PASS" : "FAIL");

await client.disconnect();
EOF
```

### Verify Text Content

```bash
bun x tsx <<'EOF'
import { connect, waitForPageLoad, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client";

const client = await connect();
const page = await client.page("verify");

await page.goto(getPreviewUrl());
await waitForPageLoad(page);

// Get page content
const title = await page.title();
const h1Text = await page.$eval('h1', el => el.textContent).catch(() => null);
const bodyText = await page.textContent('body');

console.log("Page title:", title);
console.log("H1 text:", h1Text);

// Check for expected content
const expectedContent = ["Welcome", "Get Started", "Learn More"];
for (const content of expectedContent) {
  const found = bodyText?.includes(content);
  console.log(`Contains "${content}":`, found ? "PASS" : "FAIL");
}

await client.disconnect();
EOF
```

---

## Interactive Testing

### Button Click with Verification

```bash
bun x tsx <<'EOF'
import { connect, waitForPageLoad, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client";

const client = await connect();
const page = await client.page("button");

await page.goto(getPreviewUrl());
await waitForPageLoad(page);

// Get snapshot before click
console.log("=== Before Click ===");
console.log(await client.getAISnapshot("button"));

// Click button
await page.click('button:has-text("Toggle")');

// Wait for change
await page.waitForTimeout(500);

// Get snapshot after click
console.log("\n=== After Click ===");
console.log(await client.getAISnapshot("button"));

await client.disconnect();
EOF
```

### Modal Dialog

```bash
bun x tsx <<'EOF'
import { connect, waitForPageLoad, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client";

const client = await connect();
const page = await client.page("modal");

await page.goto(getPreviewUrl());
await waitForPageLoad(page);

// Click to open modal
await page.click('button:has-text("Open Modal")');

// Wait for modal to appear
await page.waitForSelector('[role="dialog"]', { timeout: 5000 });

console.log("=== Modal Content ===");
console.log(await client.getAISnapshot("modal"));

// Close modal
await page.click('button:has-text("Close")');

// Verify modal is closed
await page.waitForSelector('[role="dialog"]', { state: 'hidden', timeout: 5000 });
console.log("Modal closed successfully");

await client.disconnect();
EOF
```

---

## Error Handling

### With Try/Catch

```bash
bun x tsx <<'EOF'
import { connect, waitForPageLoad, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client";

async function testPage() {
  const client = await connect();

  try {
    const page = await client.page("test");

    await page.goto(getPreviewUrl());
    await waitForPageLoad(page);

    // Try to click element that might not exist
    try {
      await page.click('button.submit', { timeout: 2000 });
      console.log("Button clicked");
    } catch (e) {
      console.log("Button not found, taking screenshot for debugging");
      const screenshot = await page.screenshot();
      require('fs').writeFileSync('debug.png', screenshot);
    }

    console.log("Page URL:", page.url());
  } finally {
    await client.disconnect();
  }
}

testPage().catch(console.error);
EOF
```

---

## Advanced Patterns

### Multiple Pages

```bash
bun x tsx <<'EOF'
import { connect, waitForPageLoad, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client";

const client = await connect();

// Create multiple named pages
const homePage = await client.page("home");
const aboutPage = await client.page("about");

// Navigate each to different URLs
await homePage.goto(getPreviewUrl());
await aboutPage.goto(getPreviewUrl("/about"));

await waitForPageLoad(homePage);
await waitForPageLoad(aboutPage);

// Get snapshots of both
console.log("=== Home Page ===");
console.log(await client.getAISnapshot("home"));

console.log("\n=== About Page ===");
console.log(await client.getAISnapshot("about"));

// Clean up specific page
await client.close("about");
console.log("\nClosed about page");

// List remaining pages
const pages = await client.list();
console.log("Remaining pages:", pages);

await client.disconnect();
EOF
```

### Wait for Network

```bash
bun x tsx <<'EOF'
import { connect, waitForPageLoad, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client";

const client = await connect();
const page = await client.page("api");

await page.goto(getPreviewUrl("/dashboard"));

// Wait for API call to complete
await page.waitForResponse(
  response => response.url().includes('/api/data') && response.status() === 200,
  { timeout: 10000 }
);

console.log("API call completed");
console.log(await client.getAISnapshot("api"));

await client.disconnect();
EOF
```
