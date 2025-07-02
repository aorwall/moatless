# Playwright E2E Testing Guide

## Overview

The Moatless Swarm UI uses Playwright for end-to-end testing, validating the complete user experience from UI interactions through backend processing to agent execution. The test suite includes infrastructure validation, smoke tests, and comprehensive workflow tests with real Claude Code agents.

## Test Categories & Running Tests

### 1. Smoke Tests (`smoke.spec.ts`)
Quick validation that the application is running correctly:
- Homepage loads successfully
- API endpoints respond (repositories, events)
- SSE connections can be established
- Event stream infrastructure is ready

**Run time**: ~5-10 seconds

```bash
# Run smoke tests only
pnpm test:e2e smoke.spec.ts
```

### 2. Infrastructure Tests (`infrastructure-test.spec.ts`)
Validates E2E testing framework components without a server:
- Event monitoring utilities
- API request/response validation
- Event filtering logic
- Connection resilience simulation
- TypeScript interface validation

**Run time**: ~10-15 seconds

```bash
# Run infrastructure tests only (fastest)
pnpm exec playwright test --config=playwright-infrastructure.config.ts
```

### 3. UI Task Workflow Tests (`ui-task-complete-workflow.spec.ts`)
Complete user journey through the UI:
- Creates a test repository
- Form inputs and validation
- Repository selection dropdown
- Task submission button
- Uses UI to create a new task:
  - Selects repository from dropdown
  - Enters task description
  - Clicks "Start Task" button
- Navigation to task detail page
- Real-time UI updates
- Monitors task execution until completion
- Claude Code agent startup
- Task processing
- File creation and git operations
- Verifies agent created the requested file
- Validates git diff shows changes

**Run time**: ~30-60 seconds

```bash
# Run UI workflow tests
pnpm test:e2e ui-task-complete-workflow.spec.ts

# Run specific test in UI mode
pnpm exec playwright test --ui tests/e2e/ui-task-complete-workflow.spec.ts --grep "should create task via UI"
```

### 4. Event Flow Tests (`event-flow-complete.spec.ts`)
Advanced multi-agent scenarios:
- Dual agent execution with cross-visibility
- SSE connection establishment and resilience
- Event delivery and task status updates
- Event ordering and integrity
- Global vs task-specific event streams

**Run time**: ~60-90 seconds

```bash
# Run event flow tests
pnpm test:e2e event-flow-complete.spec.ts
```

## Prerequisites & Setup

1. Install dependencies:
   ```bash
   pnpm install
   ```

2. Build the application:
   ```bash
   pnpm build
   ```

3. **Port Configuration**: Tests use port 3002 by default. You must ensure this port is available.

### Port Requirements

The E2E tests require port 3002 to be available. If the port is occupied:

```bash
# Check what's using port 3002
lsof -i :3002

# Kill process using port 3002 if needed
lsof -ti:3002 | xargs kill -9

# Then run tests
pnpm test:e2e
```

**Note**: Currently, E2E tests use a hardcoded port (3002). If you need dynamic port allocation, this would require updating the global setup configuration.

## Common Test Commands

```bash
# Run all E2E tests
pnpm test:e2e

# Run specific test file
pnpm test:e2e ui-task-complete-workflow.spec.ts

# Run tests matching a pattern
pnpm test:e2e --grep "should create task via UI"

# Run in UI mode (interactive debugging)
pnpm test:e2e:ui

# Run with browser visible
pnpm test:e2e:headed

# Debug mode (step through)
pnpm test:e2e:debug

# Generate and view HTML report
pnpm test:e2e --reporter=html
pnpm exec playwright show-report
```

### Running Specific Tests in UI Mode

To run a specific test in UI mode for interactive debugging:

```bash
# Run specific test file in UI mode
pnpm exec playwright test --ui tests/e2e/ui-task-complete-workflow.spec.ts

# Run specific test by name pattern in UI mode
pnpm exec playwright test --ui --grep "should create task via UI"

# Run specific test file and grep pattern in UI mode
pnpm exec playwright test --ui tests/e2e/ui-task-complete-workflow.spec.ts --grep "should create task"

# Alternative: Use the pnpm script with additional arguments
pnpm test:e2e:ui -- tests/e2e/ui-task-complete-workflow.spec.ts
```

In UI mode, you can:
- Click on individual tests to run them
- See the browser preview in real-time
- Step through each action with the timeline
- Inspect DOM elements
- View network requests
- Debug with breakpoints
- Re-run tests instantly

**Example: Debugging the UI task creation test**
```bash
# Open UI mode for the task creation test
pnpm exec playwright test --ui tests/e2e/ui-task-complete-workflow.spec.ts --grep "should create task via UI"
```

This opens the Playwright UI where you can:
1. Select the test in the left panel
2. Click "Run" to execute it
3. Use the timeline to step through each action
4. Hover over steps to see screenshots
5. Click "Pick locator" to test selectors
6. Use "Watch mode" for automatic re-runs on code changes

### Test Output

Tests provide detailed console output:
```
✅ Test repository created: dc448762-a0ac-4663-b2fe-aa730c744eb2
✅ Homepage loaded, chat input visible
✅ Repository selected: UI E2E Test Repository
✅ Task message entered
✅ Task created with ID: 3e0a8c55-7947-45e7-84cd-494b75ec7b14
✅ Task detail page loaded
⏳ Waiting for agent to complete task...
✅ Agent completed the task
✅ Task completed successfully
✅ File hello_world.py was created (shown in git diff)
✅ UI-based task creation and monitoring test completed successfully!
```

## Test Environment

### Automatic Setup
Tests automatically:
1. Create temporary directories for git repositories
2. Build the frontend and backend
3. Start the server on port 3002
4. Set up test data (repositories, tasks)
5. Clean up after completion

### Configuration Files

**Main Configuration** (`playwright.config.ts`):
- Test timeout: 90 seconds
- Retries: 2 on CI, 0 locally
- Base URL: http://127.0.0.1:3002 (hardcoded)
- Browser: Chromium
- Screenshots/videos on failure

**Infrastructure Config** (`playwright-infrastructure.config.ts`):
- Test timeout: 30 seconds
- No server startup required
- Faster execution

## Debugging Failed Tests

### 1. Use UI Mode
```bash
pnpm test:e2e:ui
```
- Step through tests interactively
- See browser state at each step
- Time-travel debugging

### 2. Check Artifacts
Failed tests generate:
- Screenshots: `test-results/*/test-failed-1.png`
- Videos: `test-results/*/video.webm`
- Traces: Can be viewed in trace viewer

### 3. View HTML Report
```bash
pnpm exec playwright show-report
```
Shows detailed test results with:
- Step-by-step execution
- Timing information
- Error messages
- Attached artifacts

### 4. Common Issues

**Port 3002 Already in Use**
The tests require port 3002 to be available:

```bash
# Check what's using port 3002
lsof -i :3002

# Kill the process using port 3002
lsof -ti:3002 | xargs kill -9

# Then run tests
pnpm test:e2e
```

**Test Timeouts**
- Increase timeout in test: `test('name', async ({ page }) => { ... }, 120000)`
- Or globally in config: `timeout: 120000`

**Repository Creation Fails**
- Check git is installed
- Verify temp directory permissions
- Ensure path in test matches schema

**Agent Doesn't Complete**
- Check Claude Code API key is set
- Verify agent service is running
- Look for errors in server logs

## Writing New Tests

### Basic Structure
```typescript
test('should perform user action', async ({ page }) => {
  // Navigate to page
  await page.goto('/')
  
  // Interact with UI
  await page.locator('button').click()
  
  // Wait for result
  await expect(page.locator('.result')).toBeVisible()
  
  // Verify via API
  const response = await page.request.get('/api/endpoint')
  expect(response.ok()).toBeTruthy()
})
```

### Best Practices

1. **Use data-testid attributes** for reliable element selection
2. **Wait for specific conditions** rather than arbitrary timeouts
3. **Verify through multiple channels** (UI + API)
4. **Clean up resources** in afterEach hooks
5. **Log progress** for easier debugging
6. **Make tests independent** - each test should work in isolation

### Helper Functions

The test suite provides utilities in `tests/e2e/helpers/`:

```typescript
// Create a test repository
const repo = await createTestRepository(baseURL, name, tempDir)

// Create a task via API
const task = await createTestTask(baseURL, repoId, message)

// Wait for task completion
const result = await waitForTaskCompletion(baseURL, taskId)

// Clean up
await deleteTestTask(baseURL, taskId)
await deleteTestRepository(baseURL, repoId)
```

## Performance Tips

1. **Run smoke tests first** - Quick validation before longer tests
2. **Use grep for focused testing** - Run only relevant tests during development
3. **Parallelize carefully** - Agent tests may conflict if run simultaneously
4. **Cache builds** - Use turbo cache for faster subsequent runs
5. **Monitor resource usage** - Agent tests can be CPU intensive

## Troubleshooting

### Server Won't Start
- Check node version matches requirements
- Verify all dependencies installed: `pnpm install`
- Ensure port 3002 is available (kill any conflicting processes)
- Check server logs in test output

### Flaky Tests
- Add explicit waits for conditions
- Increase assertion timeouts
- Check for race conditions
- Use retry mechanisms for network requests

### Memory Issues
- Ensure proper cleanup in afterEach
- Check for SSE connection leaks
- Monitor node process memory
- Use `--max-old-space-size` if needed

## Test Suite Structure

```
tests/e2e/
├── helpers/                    # Shared test utilities
│   ├── test-server.ts         # Repository & task creation helpers
│   ├── event-monitors.ts      # SSE & API event monitoring
│   ├── global-setup.ts        # Server startup
│   └── global-teardown.ts     # Cleanup
├── smoke.spec.ts              # Basic health checks
├── infrastructure-test.spec.ts # Framework validation
├── event-flow-complete.spec.ts # Multi-agent event tests
└── ui-task-complete-workflow.spec.ts # UI-driven task creation
```