/**
 * Centralized mock utilities for consistent testing
 *
 * This module exports all mock utilities to provide a single
 * source of truth for test mocking patterns.
 */

// API hook mocks
export {
  createMockErrorQuery,
  createMockLoadingQuery,
  createMockMutation,
  createMockPendingMutation,
  createMockQuery,
  createMockSuccessMutation,
  mockApiHooks,
} from './api-hooks'
// Provider mocks
export {
  createAllProvidersWrapper,
  createQueryWrapper,
  defaultWrapper,
} from './providers'

// SSE/WebSocket mocks
export {
  createMockSSEConnection,
  createMockSSEHook,
  createMockWebSocketConnection,
  MockEventSource,
  MockWebSocket,
} from './sse-connections'

// UI library mocks
export {
  createUILibraryMocks,
  mockAllUILibraries,
  mockCodeEditorComponents,
  mockDialogComponents,
  mockResizableComponents,
  mockScrollAreaComponents,
  mockSelectComponents,
  mockTabsComponents,
  mockTooltipComponents,
} from './ui-libraries'
