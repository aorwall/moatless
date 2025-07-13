/**
 * Centralized mock utilities for consistent testing
 * 
 * This module exports all mock utilities to provide a single
 * source of truth for test mocking patterns.
 */

// Provider mocks
export {
  createQueryWrapper,
  createAllProvidersWrapper,
  defaultWrapper,
} from './providers'

// API hook mocks
export {
  createMockQuery,
  createMockLoadingQuery,
  createMockErrorQuery,
  createMockMutation,
  createMockPendingMutation,
  createMockSuccessMutation,
  mockApiHooks,
} from './api-hooks'

// SSE/WebSocket mocks
export {
  MockEventSource,
  MockWebSocket,
  createMockSSEConnection,
  createMockWebSocketConnection,
  createMockSSEHook,
} from './sse-connections'

// UI library mocks
export {
  mockResizableComponents,
  mockDialogComponents,
  mockTooltipComponents,
  mockSelectComponents,
  mockTabsComponents,
  mockScrollAreaComponents,
  mockCodeEditorComponents,
  createUILibraryMocks,
  mockAllUILibraries,
} from './ui-libraries'