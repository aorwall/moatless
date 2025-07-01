# Playwright E2E Testing Guide

## Overview

The Moatless Swarm UI uses Playwright for end-to-end testing, focusing on validating the event-driven architecture, real-time SSE connections, and cross-agent communication. The test suite includes infrastructure validation tests and comprehensive integration tests with real Claude Code agents.

## Test Categories

### 1. Infrastructure Tests
Quick validation tests that verify the E2E framework components work correctly without requiring a full server setup.

### 2. Smoke Tests
Basic tests to ensure the application loads and API endpoints are accessible.

### 3. Complete E2E Tests
Full integration tests that run real Claude Code agents and validate the entire event flow architecture.

## Running the Tests

### Prerequisites
- Node.js and pnpm installed
- Server dependencies installed: `pnpm install`
- Build the application: `pnpm build`

### Test Commands

```bash
# Run all Playwright E2E tests
pnpm test:e2e

# Run tests with Playwright UI (interactive mode)
pnpm test:e2e:ui

# Run tests in headed mode (see browser)
pnpm test:e2e:headed

# Debug tests step-by-step
pnpm test:e2e:debug

# Run infrastructure tests only (faster)
pnpm exec playwright test --config=playwright-infrastructure.config.ts

# Run specific test file
pnpm test:e2e tests/e2e/smoke.spec.ts
```

## Test Suite Details

### Smoke Tests (`smoke.spec.ts`)
Validates basic infrastructure functionality:
- **Homepage Loading**: Ensures the application loads correctly
- **API Accessibility**: Tests that API endpoints respond with correct status codes
- **SSE Connection**: Verifies Server-Sent Events endpoints are accessible
- **Dependencies Check**: Confirms event stream infrastructure is ready

### Infrastructure Tests (`infrastructure-test.spec.ts`)
Tests E2E framework components without requiring a server:
- **Event Monitoring Utilities**: Validates event processing and filtering logic
- **API Request Helpers**: Tests request/response data structure validation
- **Event Filtering Logic**: Verifies task-specific and type-based filtering
- **Cross-Agent Visibility**: Tests event isolation and global aggregation logic
- **Connection Resilience**: Simulates disconnection and reconnection scenarios
- **TypeScript Validation**: Ensures all interfaces compile correctly

### Event Flow Complete Tests (`event-flow-complete.spec.ts`)
Comprehensive tests with real Claude Code agents:

#### Dual Agent Execution Test
- Creates two separate environments with git repositories
- Launches two Claude Code agents with different tasks:
  - Agent A: Creates a Python calculator file
  - Agent B: Creates a markdown features document
- Monitors events through multiple channels:
  - API polling monitor
  - Global SSE connection
  - Task-specific SSE connection
- Validates:
  - File creation by both agents
  - Cross-visibility of events between agents
  - Event ordering and integrity
  - Frontend UI updates
  - Connection statistics

#### Connection Resilience Test
- Tests SSE connection recovery after interruption
- Verifies event continuity after reconnection
- Ensures task completion despite network issues

## Test Architecture

### Event Monitors
The tests use specialized monitor classes to track events:

1. **APIEventMonitor**: Polls the REST API for events
2. **SSEEventMonitor**: Establishes SSE connections via Playwright
3. **Both monitors track**:
   - Event content and metadata
   - Connection status
   - Error conditions

### Test Helpers
Located in `tests/e2e/helpers/`:
- `test-server.ts`: Environment and task creation utilities
- `event-monitors.ts`: Event monitoring implementations
- `global-setup.ts`: Server startup for tests
- `global-teardown.ts`: Cleanup after tests

## What the Tests Validate

### Backend Event Flow
- ✅ Event emission from agents
- ✅ Dual channel emission (task-specific + global)
- ✅ Event persistence and storage
- ✅ SSE connection management
- ✅ Event filtering and routing

### Frontend Integration
- ✅ Global event stream provider functionality
- ✅ Task-specific event subscriptions
- ✅ React Query cache invalidation from events
- ✅ UI component updates from real-time events
- ✅ Connection status monitoring

### Cross-Agent Architecture
- ✅ Event visibility across different agents
- ✅ Event isolation for task-specific streams
- ✅ Global stream aggregation
- ✅ Event ordering and timing validation
- ✅ Connection resilience and recovery

### Performance & Reliability
- ✅ Connection limit management
- ✅ Event deduplication
- ✅ Memory leak prevention
- ✅ Error handling and recovery
- ✅ Reconnection with exponential backoff

## Configuration

### Main Configuration (`playwright.config.ts`)
- **Test timeout**: 90 seconds (for tests with real agents)
- **Assertion timeout**: 10 seconds
- **Base URL**: http://127.0.0.1:3001
- **Browser**: Chromium only
- **Features**: Screenshots, videos, and traces on failure

### Infrastructure Configuration (`playwright-infrastructure.config.ts`)
- **Test timeout**: 30 seconds (faster tests)
- **Assertion timeout**: 5 seconds
- **Test match**: Only infrastructure and smoke tests
- **Simplified setup**: No full server required

## Debugging Failed Tests

### View Test Reports
```bash
# After running tests, open the HTML report
npx playwright show-report
```

### Debug Strategies
1. **Use UI Mode**: `pnpm test:e2e:ui` for interactive debugging
2. **Check Screenshots**: Located in `test-results/` directory
3. **Review Videos**: Automatically recorded on failure
4. **Inspect Traces**: Use Playwright trace viewer for detailed execution logs
5. **Console Logs**: Tests include extensive logging for troubleshooting

### Common Issues
- **Server not started**: Ensure the global setup completes successfully
- **Port conflicts**: Check if port 3001 is available
- **Timeout errors**: Real agent tests may take longer; adjust timeouts if needed
- **Connection failures**: Verify SSE endpoints are accessible

## CI/CD Integration

The tests are configured for CI environments:
- Retries enabled (2 attempts on CI)
- Single worker to avoid resource conflicts
- Fail on `test.only` to prevent accidental test skipping
- HTML reports generated for artifact storage

## Best Practices

1. **Run infrastructure tests first** to quickly validate setup
2. **Use specific test files** when debugging individual features
3. **Monitor test logs** for detailed event flow information
4. **Clean up resources** - tests automatically clean up environments and tasks
5. **Check event counts** in test summaries to understand system behavior
