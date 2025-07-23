# Playwright E2E Testing Guide

## Overview

The Moatless Vibe uses Playwright for end-to-end testing, validating the complete user experience from UI interactions through backend processing to agent execution. The test suite includes infrastructure validation, smoke tests, and comprehensive workflow tests with real Claude Code agents.

## Event Schema

The application uses a consolidated event system with 5 main event types. For complete details about event types, subtypes, and state transitions, see [Task Lifecycle Pattern](./task-lifecycle.md#backend-event-emission).

### Key Changes for E2E Testing

When testing event flows, note these important changes:
- **Agent messages**: Previously `type: 'message'`, now `type: 'agent', subType: 'message'`
- **Task completion**: Previously `type: 'result'`, now `type: 'agent', subType: 'completed'` 
- **Error events**: All error events now use the `error` subType (not `failed`)
- **Tool calls**: No longer separate events - they're part of agent messages

## Test Categories & Running Tests

### 1. Essential Smoke Tests (`smoke.spec.ts`)
Quick validation that the application is running correctly:
- React application loads and renders
- API endpoints respond (repositories, settings, events)
- SSE event stream connections work
- Temporary data directory is used correctly

**Run time**: ~5-10 seconds

```bash
# Run smoke tests only
bun test:e2e smoke.spec.ts
```

### 2. UI Task Workflow Tests (`ui-task-complete-workflow.spec.ts`)
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
bun test:e2e ui-task-complete-workflow.spec.ts

# Run specific test in UI mode
bun exec playwright test --ui tests/e2e/ui-task-complete-workflow.spec.ts --grep "should create task via UI"
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
bun test:e2e event-flow-complete.spec.ts
```

## Prerequisites & Setup

1. Install dependencies:
   ```bash
   bun install
   ```

2. Build the application:
   ```bash
   bun build
   ```

3. **Set Claude OAuth Token** (required for tests using Claude Code agent):
   ```bash
   # Add to your .env file:
   CLAUDE_CODE_OAUTH_TOKEN=your-oauth-token-here
   ```
   Without this token, E2E tests that create tasks with Claude Code agents will fail.

4. **Port Configuration**: Tests automatically find available ports starting from 3002.

### Automatic Port Detection

The E2E tests automatically detect and use available ports:

```bash
# Tests automatically find an available port
bun test:e2e

# Use a custom starting port
E2E_PORT=4000 bun test:e2e

# Check which port was selected (shown in test output)
# Example output: "Starting server for E2E tests on port 3003..."
```

**How it works**: The setup automatically scans ports starting from 3002 (or your custom port) and selects the first available one. The selected port is displayed in the terminal output.

### Temporary Data Directory

E2E tests use a temporary data directory to ensure test isolation:
- A new temporary directory is created for each test run
- The directory is automatically cleaned up after tests complete
- This ensures tests don't interfere with your local development data
- Repository lists will be empty in tests unless explicitly created

The temporary directory path is shown in the test output:
```
Created temp directory: /var/folders/.../moatless-e2e-test-xxxxx
```

## Common Test Commands

```bash
# Run all E2E tests
bun test:e2e

# Run specific test file
bun test:e2e ui-task-complete-workflow.spec.ts

# Run tests matching a pattern
bun test:e2e --grep "should create task via UI"

# Run in UI mode (interactive debugging)
bun test:e2e:ui

# Run with browser visible
bun test:e2e:headed

# Debug mode (step through)
bun test:e2e:debug

# Generate and view HTML report
bun test:e2e --reporter=html
bun exec playwright show-report
```

### Running Specific Tests in UI Mode

To run a specific test in UI mode for interactive debugging:

```bash
# Run specific test file in UI mode
bun exec playwright test --ui tests/e2e/ui-task-complete-workflow.spec.ts

# Run specific test by name pattern in UI mode
bun exec playwright test --ui --grep "should create task via UI"

# Run specific test file and grep pattern in UI mode
bun exec playwright test --ui tests/e2e/ui-task-complete-workflow.spec.ts --grep "should create task"

# Alternative: Use the bun script with additional arguments
bun test:e2e:ui -- tests/e2e/ui-task-complete-workflow.spec.ts
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
bun exec playwright test --ui tests/e2e/ui-task-complete-workflow.spec.ts --grep "should create task via UI"
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
2. Find an available port (starting from 3002)
3. Build the frontend and backend
4. Start the server on the selected port
5. Set up test data (repositories, tasks)
6. Clean up after completion

### Configuration Files

**Main Configuration** (`playwright.config.ts`):
- Test timeout: 90 seconds
- Retries: 2 on CI, 0 locally
- Base URL: Dynamically set based on available port
- Browser: Chromium
- Screenshots/videos on failure

**Infrastructure Config** (`playwright-infrastructure.config.ts`):
- Test timeout: 30 seconds
- No server startup required
- Faster execution

## Debugging Failed Tests

### 1. Use UI Mode
```bash
bun test:e2e:ui
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
bun exec playwright show-report
```
Shows detailed test results with:
- Step-by-step execution
- Timing information
- Error messages
- Attached artifacts

### 4. Common Issues

**All Ports in Range Occupied**
If you encounter issues with port detection:

```bash
# Check which ports are in use in the range
lsof -i :3002-3102

# Use a different starting port range
E2E_PORT=4000 bun test:e2e

# The system will automatically find the first available port
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
- Verify all dependencies installed: `bun install`
- Ensure some ports are available in the 3002+ range
- Try a different port range: `E2E_PORT=4000 bun test:e2e`
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