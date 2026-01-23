# Browser Automation Script Examples

## Basic Usage

### Connect and Navigate

```bash
cat > /tmp/connect-navigate.ts <<'EOF'
import { connect, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

const client = await connect();
const page = await client.page("main");

// goto() already waits for domcontentloaded
await page.goto(getPreviewUrl());

console.log("Current URL:", page.url());

await client.disconnect();
EOF

bun run /tmp/connect-navigate.ts
```

### Get Page Content

```bash
cat > /tmp/get-content.ts <<'EOF'
import { connect, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

const client = await connect();
const page = await client.page("main");

await page.goto(getPreviewUrl());

// Get various page information via evaluate()
const title = await page.evaluate(() => document.title);
const heading = await page.evaluate(() => document.querySelector('h1')?.textContent);
const linkCount = await page.evaluate(() => document.querySelectorAll('a').length);

console.log({ title, heading, linkCount });

await client.disconnect();
EOF

bun run /tmp/get-content.ts
```

### Take Screenshot

```bash
cat > /tmp/take-screenshot.ts <<'EOF'
import { connect, takeScreenshot, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";
import { writeFileSync } from "fs";

const client = await connect();
const page = await client.page("main");

await page.goto(getPreviewUrl());

const base64 = await takeScreenshot(page);
writeFileSync("screenshot.png", Buffer.from(base64, "base64"));
console.log("Screenshot saved to screenshot.png");

await client.disconnect();
EOF

bun run /tmp/take-screenshot.ts
```

---

## Form Interactions

### Fill and Submit Form

```bash
cat > /tmp/fill-submit-form.ts <<'EOF'
import { connect, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

const client = await connect();
const page = await client.page("form");

await page.goto(getPreviewUrl("/contact"));

// Fill form fields via evaluate()
await page.evaluate(() => {
  const setInput = (selector: string, value: string) => {
    const el = document.querySelector(selector) as HTMLInputElement | HTMLTextAreaElement;
    if (el) {
      el.value = value;
      el.dispatchEvent(new Event('input', { bubbles: true }));
    }
  };
  setInput('input[name="name"]', "John Doe");
  setInput('input[name="email"]', "john@example.com");
  setInput('textarea[name="message"]', "Hello, this is a test message!");
});

// Submit form
await page.evaluate(() => document.querySelector('button[type="submit"]')?.click());

// Wait for form submission to process
await page.waitForTimeout(500);

console.log("Form submitted!");
const successMessage = await page.evaluate(() =>
  document.querySelector('.success-message')?.textContent
);
console.log("Result:", successMessage);

await client.disconnect();
EOF

bun run /tmp/fill-submit-form.ts
```

### Login Flow

```bash
cat > /tmp/login-flow.ts <<'EOF'
import { connect, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

const client = await connect();
const page = await client.page("login");

await page.goto(getPreviewUrl("/login"));

// Enter credentials via evaluate()
await page.evaluate(() => {
  const emailInput = document.querySelector('input[type="email"]') as HTMLInputElement;
  const passwordInput = document.querySelector('input[type="password"]') as HTMLInputElement;
  if (emailInput) emailInput.value = "user@example.com";
  if (passwordInput) passwordInput.value = "password123";
});

// Click login button
await page.evaluate(() => document.querySelector('button[type="submit"]')?.click());

// Wait for login to complete
await page.waitForTimeout(1000);

console.log("After login URL:", page.url());
const userInfo = await page.evaluate(() =>
  document.querySelector('.user-info')?.textContent
);
console.log("User info:", userInfo);

await client.disconnect();
EOF

bun run /tmp/login-flow.ts
```

---

## Navigation Testing

### Click Through Navigation

```bash
cat > /tmp/click-navigation.ts <<'EOF'
import { connect, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

const client = await connect();
const page = await client.page("nav");

await page.goto(getPreviewUrl());

// Get initial content
console.log("=== Home Page ===");
const homeHeading = await page.evaluate(() => document.querySelector('h1')?.textContent);
console.log("Heading:", homeHeading);

// Navigate to About using evaluate()
await page.evaluate(() => document.querySelector('a[href="/about"]')?.click());
await page.waitForTimeout(500);
console.log("\n=== About Page ===");
const aboutHeading = await page.evaluate(() => document.querySelector('h1')?.textContent);
console.log("Heading:", aboutHeading);

// Navigate to Contact
await page.evaluate(() => document.querySelector('a[href="/contact"]')?.click());
await page.waitForTimeout(500);
console.log("\n=== Contact Page ===");
const contactHeading = await page.evaluate(() => document.querySelector('h1')?.textContent);
console.log("Heading:", contactHeading);

await client.disconnect();
EOF

bun run /tmp/click-navigation.ts
```

---

## Content Verification

### Verify Page Structure

```bash
cat > /tmp/verify-page-structure.ts <<'EOF'
import { connect, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

const client = await connect();
const page = await client.page("verify");

await page.goto(getPreviewUrl());

// Check if elements exist via evaluate()
const structure = await page.evaluate(() => ({
  hasHeading: !!document.querySelector('h1'),
  hasNav: !!document.querySelector('nav'),
  hasButton: !!document.querySelector('button'),
  hasFooter: !!document.querySelector('footer'),
}));

console.log("Element checks:");
console.log("  Has heading:", structure.hasHeading ? "PASS" : "FAIL");
console.log("  Has navigation:", structure.hasNav ? "PASS" : "FAIL");
console.log("  Has button:", structure.hasButton ? "PASS" : "FAIL");
console.log("  Has footer:", structure.hasFooter ? "PASS" : "FAIL");

await client.disconnect();
EOF

bun run /tmp/verify-page-structure.ts
```

### Verify Text Content

```bash
cat > /tmp/verify-text-content.ts <<'EOF'
import { connect, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

const client = await connect();
const page = await client.page("verify");

await page.goto(getPreviewUrl());

// Get all text content
const pageText = await page.evaluate(() => document.body.innerText);

console.log("Page URL:", page.url());

// Check for expected content
const expectedContent = ["Welcome", "Get Started", "Learn More"];
for (const content of expectedContent) {
  const found = pageText.includes(content);
  console.log(`Contains "${content}":`, found ? "PASS" : "FAIL");
}

await client.disconnect();
EOF

bun run /tmp/verify-text-content.ts
```

---

## Interactive Testing

### Button Click with Verification

```bash
cat > /tmp/button-click-verify.ts <<'EOF'
import { connect, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

const client = await connect();
const page = await client.page("button");

await page.goto(getPreviewUrl());

// Get state before click
console.log("=== Before Click ===");
const beforeState = await page.evaluate(() =>
  document.querySelector('.toggle-state')?.textContent
);
console.log("State:", beforeState);

// Click toggle button
await page.evaluate(() => document.querySelector('button')?.click());

// Wait for UI update
await page.waitForTimeout(500);

// Get state after click
console.log("\n=== After Click ===");
const afterState = await page.evaluate(() =>
  document.querySelector('.toggle-state')?.textContent
);
console.log("State:", afterState);

await client.disconnect();
EOF

bun run /tmp/button-click-verify.ts
```

### Modal Dialog

```bash
cat > /tmp/modal-dialog.ts <<'EOF'
import { connect, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

const client = await connect();
const page = await client.page("modal");

await page.goto(getPreviewUrl());

// Click to open modal
await page.evaluate(() => document.querySelector('button')?.click());
await page.waitForSelector('.modal', { state: 'visible' });

console.log("=== Modal Content ===");
const modalTitle = await page.evaluate(() =>
  document.querySelector('.modal-title')?.textContent
);
console.log("Title:", modalTitle);

// Close modal by clicking close button
await page.evaluate(() => document.querySelector('[aria-label="Close"]')?.click());
await page.waitForSelector('.modal', { state: 'hidden' });

console.log("\n=== Modal Closed ===");

await client.disconnect();
EOF

bun run /tmp/modal-dialog.ts
```

---

## Error Handling

### With Try/Catch

```bash
cat > /tmp/error-handling.ts <<'EOF'
import { connect, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

async function testPage() {
  const client = await connect();

  try {
    const page = await client.page("test");

    await page.goto(getPreviewUrl());

    // Try to click element that might not exist
    const buttonExists = await page.evaluate(() =>
      !!document.querySelector('button.submit')
    );

    if (buttonExists) {
      await page.evaluate(() => document.querySelector('button.submit')?.click());
      console.log("Button clicked");
    } else {
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

bun run /tmp/error-handling.ts
```

---

## Advanced Patterns

### Multiple Pages

```bash
cat > /tmp/multiple-pages.ts <<'EOF'
import { connect, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

const client = await connect();

// Create multiple named pages
const homePage = await client.page("home");
const aboutPage = await client.page("about");

// Navigate each to different URLs
await homePage.goto(getPreviewUrl());
await aboutPage.goto(getPreviewUrl("/about"));

// Get content from both
console.log("=== Home Page ===");
const homeTitle = await homePage.evaluate(() => document.title);
console.log("Title:", homeTitle);

console.log("\n=== About Page ===");
const aboutTitle = await aboutPage.evaluate(() => document.title);
console.log("Title:", aboutTitle);

// Clean up specific page
await client.close("about");
console.log("\nClosed about page");

// List remaining pages
const pages = await client.list();
console.log("Remaining pages:", pages);

await client.disconnect();
EOF

bun run /tmp/multiple-pages.ts
```

### Mobile Viewport Testing

```bash
cat > /tmp/mobile-viewport.ts <<'EOF'
import { connect, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

const client = await connect();

// Create mobile-sized page
const page = await client.page("mobile", {
  viewport: { width: 375, height: 812 }
});

await page.goto(getPreviewUrl());

console.log("Mobile viewport:", page.viewportSize());
const isMobileLayout = await page.evaluate(() =>
  !!document.querySelector('.mobile-nav') ||
  !!document.querySelector('.hamburger-menu')
);
console.log("Has mobile layout:", isMobileLayout);

// Switch to tablet
await page.setViewportSize({ width: 768, height: 1024 });
console.log("\nTablet viewport:", page.viewportSize());

await client.disconnect();
EOF

bun run /tmp/mobile-viewport.ts
```

### Page with URL Option

```bash
cat > /tmp/page-url-option.ts <<'EOF'
import { connect, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

const client = await connect();

// Create page and navigate in one call
const page = await client.page("quick", {
  url: getPreviewUrl("/dashboard")
});

console.log("URL:", page.url());
const dashboardTitle = await page.evaluate(() =>
  document.querySelector('h1')?.textContent
);
console.log("Title:", dashboardTitle);

await client.disconnect();
EOF

bun run /tmp/page-url-option.ts
```

---

## DOM Manipulation

### Get Element Content

```bash
cat > /tmp/get-element-content.ts <<'EOF'
import { connect, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

const client = await connect();
const page = await client.page("main");

await page.goto(getPreviewUrl());

// Get document title
const title = await page.evaluate(() => document.title);
console.log("Title:", title);

// Get text content of an element
const heading = await page.evaluate(
  (selector) => document.querySelector(selector)?.textContent,
  'h1'
);
console.log("Heading:", heading);

// Get an attribute
const logoHref = await page.evaluate(
  (selector, attr) => document.querySelector(selector)?.getAttribute(attr),
  'a.logo', 'href'
);
console.log("Logo href:", logoHref);

await client.disconnect();
EOF

bun run /tmp/get-element-content.ts
```

### Check Element State

```bash
cat > /tmp/check-element-state.ts <<'EOF'
import { connect, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

const client = await connect();
const page = await client.page("main");

await page.goto(getPreviewUrl());

// Check if element is visible
const isModalVisible = await page.evaluate((selector) => {
  const el = document.querySelector(selector);
  if (!el) return false;
  const style = getComputedStyle(el);
  return style.display !== 'none' && style.visibility !== 'hidden';
}, '.modal');
console.log("Modal visible:", isModalVisible);

// Check if button is disabled
const isButtonDisabled = await page.evaluate(
  (selector) => (document.querySelector(selector) as HTMLButtonElement)?.disabled,
  'button[type="submit"]'
);
console.log("Button disabled:", isButtonDisabled);

// Check if checkbox is checked
const isChecked = await page.evaluate(
  (selector) => (document.querySelector(selector) as HTMLInputElement)?.checked,
  'input[type="checkbox"]'
);
console.log("Checkbox checked:", isChecked);

await client.disconnect();
EOF

bun run /tmp/check-element-state.ts
```

### Select Dropdown Option

```bash
cat > /tmp/select-dropdown.ts <<'EOF'
import { connect, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

const client = await connect();
const page = await client.page("main");

await page.goto(getPreviewUrl("/form"));

// Select an option and dispatch change event
await page.evaluate((selector, value) => {
  const select = document.querySelector(selector) as HTMLSelectElement;
  if (select) {
    select.value = value;
    select.dispatchEvent(new Event('change', { bubbles: true }));
  }
}, 'select#country', 'US');

console.log("Selected country: US");

// Verify selection
const selectedValue = await page.evaluate(
  (selector) => (document.querySelector(selector) as HTMLSelectElement)?.value,
  'select#country'
);
console.log("Selected value:", selectedValue);

await client.disconnect();
EOF

bun run /tmp/select-dropdown.ts
```

### Extract Multiple Elements

```bash
cat > /tmp/extract-elements.ts <<'EOF'
import { connect, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

const client = await connect();
const page = await client.page("main");

await page.goto(getPreviewUrl());

// Get all link URLs
const links = await page.evaluate(() =>
  Array.from(document.querySelectorAll('a')).map(a => ({
    text: a.textContent?.trim(),
    href: a.href
  }))
);
console.log("Links:", JSON.stringify(links, null, 2));

// Get all button texts
const buttons = await page.evaluate(() =>
  Array.from(document.querySelectorAll('button')).map(b => b.textContent?.trim())
);
console.log("Buttons:", buttons);

await client.disconnect();
EOF

bun run /tmp/extract-elements.ts
```

---

## Waiting for Elements

### Wait for Dynamic Content

```bash
cat > /tmp/wait-dynamic-content.ts <<'EOF'
import { connect, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

const client = await connect();
const page = await client.page("main");

await page.goto(getPreviewUrl());

// Click button that loads content
await page.evaluate(() => document.querySelector('button#load-data')?.click());

// Wait for results to appear
await page.waitForSelector('.results', { state: 'visible' });
console.log("Results loaded!");

// Wait for loading spinner to disappear
await page.waitForSelector('.spinner', { state: 'hidden' });
console.log("Spinner hidden!");

// Get the results
const results = await page.evaluate(() =>
  document.querySelector('.results')?.textContent
);
console.log("Results:", results);

await client.disconnect();
EOF

bun run /tmp/wait-dynamic-content.ts
```

### Wait for Modal

```bash
cat > /tmp/wait-modal.ts <<'EOF'
import { connect, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

const client = await connect();
const page = await client.page("main");

await page.goto(getPreviewUrl());

// Click to open modal
await page.evaluate(() => document.querySelector('button#open-modal')?.click());

// Wait for modal to be visible
await page.waitForSelector('.modal', { state: 'visible' });
console.log("Modal opened!");

const modalContent = await page.evaluate(() =>
  document.querySelector('.modal-body')?.textContent
);
console.log("Modal content:", modalContent);

// Close modal
await page.evaluate(() => document.querySelector('button.modal-close')?.click());

// Wait for modal to be hidden
await page.waitForSelector('.modal', { state: 'hidden' });
console.log("Modal closed!");

await client.disconnect();
EOF

bun run /tmp/wait-modal.ts
```

---

## Raw CDP Access

### Get Cookies

```bash
cat > /tmp/get-cookies.ts <<'EOF'
import { connect, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

const client = await connect();
const page = await client.page("main");

await page.goto(getPreviewUrl());

const cdp = page.getCdpSession();

// Enable network domain
await cdp.send('Network.enable', {});

// Get all cookies
const { cookies } = await cdp.send<{ cookies: Array<{ name: string; value: string }> }>(
  'Network.getCookies', {}
);
console.log("Cookies:", cookies);

await client.disconnect();
EOF

bun run /tmp/get-cookies.ts
```

### Emulate Dark Mode

```bash
cat > /tmp/emulate-dark-mode.ts <<'EOF'
import { connect, getPreviewUrl } from "/opt/moatless/.claude/skills/sandbox-browser/scripts/browser-client.ts";

const client = await connect();
const page = await client.page("main");

const cdp = page.getCdpSession();

// Emulate dark mode preference
await cdp.send('Emulation.setEmulatedMedia', {
  features: [{ name: 'prefers-color-scheme', value: 'dark' }]
});

await page.goto(getPreviewUrl());

console.log("Page rendered in dark mode");

// Check if dark mode styles are applied
const isDarkMode = await page.evaluate(() =>
  window.matchMedia('(prefers-color-scheme: dark)').matches
);
console.log("Dark mode active:", isDarkMode);

await client.captureScreenshot("main", "manual");

await client.disconnect();
EOF

bun run /tmp/emulate-dark-mode.ts
```
