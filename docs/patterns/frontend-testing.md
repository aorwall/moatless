# Frontend Testing Guide

## Philosophy: Pragmatic Value-Focused Testing

This guide advocates for **pragmatic testing** that maximizes ROI by testing what matters. Not all components need the same level of testing coverage. Focus on business-critical paths and user-observable behavior.

**Key Principles:**
1. **Test user behavior, not implementation** - Focus on what users see and do
2. **Classify components by risk** - Not everything deserves equal test coverage
3. **Avoid test maintenance debt** - Brittle tests slow development
4. **Integration over isolation** - Real component integration catches more bugs

## Test Structure

Tests are organized in `__tests__` directories co-located with source code:

```
src/features/tasks/
├── components/
│   ├── __tests__/
│   │   ├── task-viewer.unit.test.tsx
│   │   └── task-header.unit.test.tsx
│   └── task-viewer.tsx
├── hooks/
│   ├── __tests__/
│   │   └── use-tasks.unit.test.ts
│   └── use-tasks.ts
```

## Component Classification System

Before writing tests, classify your component to determine appropriate test coverage:

### Tier 1: MUST TEST (Business-Critical) - Target: 80-90% Coverage

Components with complex logic that directly impact user workflows:

- **Complex state machines** - Components with multiple states and transitions
  - Example: `MessageHistory` with message grouping, expansion, auto-scroll
  - Why test: State bugs cause UX failures

- **Business logic in components** - Components that process or transform data
  - Example: `ChatForm` with file autocomplete, upload, paste handling
  - Why test: Logic errors lose user data or break workflows

- **Critical user flows** - Components in primary user paths
  - Example: Task submission, repository selection, error recovery
  - Why test: Breakage blocks core functionality

### Tier 2: SHOULD TEST (High-Value) - Target: 50-60% Coverage

Components with moderate complexity that enhance UX:

- **Conditional rendering** - Components that show/hide based on state
  - Example: `PopoverSelector` with loading/error/success states
  - Why test: Missing states confuse users

- **Interactive UI patterns** - Components with user interactions beyond simple clicks
  - Example: Drag-and-drop, multi-select, keyboard navigation
  - Why test: Interaction bugs frustrate users

- **Form validation** - Components that validate user input
  - Example: Input fields with validation rules
  - Why test: Validation bugs allow invalid data

### Tier 3: DON'T TEST (Low-Value) - Target: 0% Coverage

**DO NOT write tests for these components:**

- **Pure presentational wrappers** - Just apply styling (e.g., `ChatView`, `Card`)
- **Simple display components** - Just render props (e.g., `ThinkingIndicator`, `TodoList`)
- **Icon components** - SVG or icon wrappers

**Why skip:** No logic to break. Parent integration tests catch issues. Focus time on Tier 1/2.

### Classification Examples from Chat Components

| Component | Tier | Rationale |
|-----------|------|-----------|
| `ChatForm` | Tier 1 | Complex: file upload, autocomplete, paste, keyboard shortcuts |
| `MessageHistory` | Tier 1 | Complex: grouping, expansion, auto-scroll, SSE updates |
| `PopoverSelector` | Tier 2 | Moderate: loading/error states, keyboard navigation |
| `GenericSelector` | Tier 2 | Moderate: error handling, disabled states |
| `TodoList` | Tier 3 | Simple: maps props to UI, no logic - DON'T TEST |
| `ChatView` | Tier 3 | Trivial: styled wrapper - DON'T TEST |
| `ThinkingIndicator` | Tier 3 | Trivial: static animation - DON'T TEST |

## Test File Naming Conventions

**Unit Tests** - `*.unit.test.{ts,tsx}` - Fast, isolated tests
**Integration Tests** - `*.integration.test.{ts,tsx}` - Component integration tests
**E2E Tests** - `*.e2e.test.{ts,tsx}` - Full user journey tests

## Quick Reference: Should I Test This?

| Component Type | Test It? | Example |
|----------------|----------|---------|
| Complex forms with validation/upload | ✅ YES - Tier 1 | ChatForm, TaskForm |
| State machines with multiple states | ✅ YES - Tier 1 | MessageHistory, TaskViewer |
| Conditional UI with error/loading | ⚠️ MAYBE - Tier 2 | PopoverSelector, GenericSelector |
| Simple display components | ❌ NO - Tier 3 | TodoList, ThinkingIndicator |
| Styled wrappers (div with className) | ❌ NO - Tier 3 | ChatView, Card |

**Golden Rule**: Business logic or critical user flow → Test it. Just renders props → Skip it.

## Test Commands

```bash
bun test           # All tests
bun test:unit      # Unit tests only (fastest)
bun test:integration # Integration tests only
bun test:fast      # Alias for test:unit (git hooks)
bun test:coverage  # With coverage report
```

## React 19 Compatibility

```typescript
// Required in src/test-utils/setup.ts
if (!globalThis.IS_REACT_ACT_ENVIRONMENT) {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true
}
```

## Mock Utilities Quick Reference

```typescript
// 1. Data Mocks (from @moatless/types/test-utils)
import { createMockTask, createMockRepository } from '@moatless/types/test-utils'
const task = createMockTask({ status: 'completed' })

// 2. API Hook States (from @/test-utils/mocks)
import { createMockQuery, createMockLoadingQuery, createMockErrorQuery } from '@/test-utils/mocks'
vi.mocked(useTask).mockReturnValue(createMockQuery(task))
vi.mocked(useTask).mockReturnValue(createMockLoadingQuery())
vi.mocked(useTask).mockReturnValue(createMockErrorQuery(new Error('Failed')))

// 3. SSE/WebSocket
import { createMockSSEConnection } from '@/test-utils/mocks'
const { mockEventSource } = createMockSSEConnection()
mockEventSource.simulateData({ type: 'update', data: newData })

// 4. UI Components (only if they use ResizeObserver/IntersectionObserver)
import { mockResizableComponents } from '@/test-utils/mocks'
vi.mock('@/lib/components/ui/resizable', () => mockResizableComponents())

// 5. Provider Wrappers
import { createQueryWrapper } from '@/test-utils/mocks'
const { wrapper } = createQueryWrapper()
render(<Component />, { wrapper })
```

## Key Testing Principles

### 1. Mock Placement and Import Order

```typescript
// ✅ CORRECT: vi.mock() calls MUST come before all imports
vi.mock('@/features/tasks/hooks/use-tasks')
vi.mock('@/features/repositories/hooks/use-repositories')

import { render, screen } from '@testing-library/react'
import { useTask } from '@/features/tasks/hooks/use-tasks'
import { TaskViewer } from '../task-viewer'

// ❌ INCORRECT: vi.mock() after imports causes hoisting issues
import { TaskViewer } from '../task-viewer'
vi.mock('@/features/tasks/hooks/use-tasks') // This will fail!
```

### 2. UI Component Testing Decision Tree

```
START: I need to test a component
│
├─ Does it have business logic? (data transformation, validation, calculations)
│  ├─ YES → Tier 1: Write comprehensive tests
│  │        Test: logic correctness, edge cases, error handling
│  │
│  └─ NO → Does it have multiple states/interactions?
│           ├─ YES → Tier 2: Test state transitions
│           │        Test: loading/error states, keyboard interactions
│           │
│           └─ NO → Is it just rendering props?
│                    ├─ YES → Tier 3: DON'T TEST
│                    │
│                    └─ UNSURE → Ask: "If this breaks, does it block users?"
│                                 ├─ YES → Tier 1 or 2
│                                 └─ NO → Tier 3 (skip testing)
```

### 3. What to Test vs What NOT to Test

**✅ DO TEST (User-Observable Behavior):**
- User interactions produce expected results (form submits, modal opens)
- Error messages display when operations fail
- Loading states appear during async operations
- Keyboard shortcuts work (Enter, Escape, Tab)
- Accessibility attributes present (ARIA labels, roles)
- Edge cases handled (empty state, max length, network errors)

**❌ DON'T TEST (Implementation Details):**
- CSS classes applied correctly → Use visual regression tests instead
- Props passed to child components → Integration test at parent level
- Component internal state values → Test observable effects instead
- React hook call counts → Only test if implementing retry logic
- Exact HTML structure → Test semantic elements (buttons, inputs)
- Styling variations → Component libraries already tested

### 4. Strategic Mocking - Decision Tree

```
What should I mock in my test?
│
├─ Is it an external dependency? (network, browser API, third-party service)
│  ├─ YES → Mock it
│  │        • HTTP API → createMockQuery() / createMockMutation()
│  │        • SSE/WebSocket → createMockSSEConnection()
│  │        • localStorage/sessionStorage → vi.stubGlobal()
│  │        • File uploads → Mock FileReader, FormData
│  │
│  └─ NO → Is it a UI library component?
│           ├─ Does it use browser measurements? (ResizeObserver, IntersectionObserver)
│           │  ├─ YES → Mock it (e.g., react-resizable-panels)
│           │  └─ NO → Is it simple and stable? (Button, Input, Card)
│           │           ├─ YES → Don't mock, test real integration
│           │           └─ NO → Does it have complex state? (Dialog, Dropdown)
│           │                    ├─ YES → Consider mocking for test speed
│           │                    └─ NO → Don't mock
│           │
│           └─ Is it your own component from the same codebase?
│                    → DON'T mock (test real integration)
```

### Concrete Mocking Rules

**ALWAYS Mock These (External Dependencies)**:
- `fetch()`, `axios`, or any HTTP client
- `EventSource` for SSE connections
- `WebSocket` connections
- `window.localStorage`, `window.sessionStorage`
- External authentication services
- File upload/download APIs
- Browser notifications API

**ALWAYS Mock These UI Libraries**:
- `@/lib/components/ui/resizable` - Uses ResizeObserver
- `react-virtualized` or similar - Complex viewport calculations
- `react-beautiful-dnd` - Drag state management
- Chart/graph libraries - Canvas/SVG rendering
- Rich text editors - Complex DOM manipulation

**NEVER Mock These**:
- Components in the same feature folder
- React hooks from the same codebase
- Simple UI components (Button, Input, Card)
- Utility functions
- Type definitions or constants

### 5. Pragmatic Test Examples from Chat Components

#### Example 1: Tier 1 Component (ChatForm) - Comprehensive Testing

```typescript
// ChatForm.test.tsx - ~100 lines for comprehensive coverage
import { render, screen, waitFor } from '@testing-library/react'
import { userEvent } from '@testing-library/user-event'
import { vi } from 'vitest'
import { ChatForm } from '../chat-form'

// Mock only external dependencies
vi.mock('@/features/uploads/hooks/use-upload')
vi.mock('@/lib/hooks/use-universal-file-autocomplete')

describe('ChatForm', () => {
  const defaultProps = {
    message: '',
    onMessageChange: vi.fn(),
    onSubmit: vi.fn(),
    files: [],
    onFilesChange: vi.fn(),
  }

  // Test critical user flow: message submission
  it('submits message on Enter key', async () => {
    const onSubmit = vi.fn()
    render(<ChatForm {...defaultProps} message="hello" onSubmit={onSubmit} />)

    const textarea = screen.getByRole('textbox')
    await userEvent.click(textarea)
    await userEvent.keyboard('{Enter}')

    expect(onSubmit).toHaveBeenCalled()
  })

  // Test edge case: empty message doesn't submit
  it('prevents submission when message is empty', async () => {
    const onSubmit = vi.fn()
    render(<ChatForm {...defaultProps} message="" onSubmit={onSubmit} />)

    await userEvent.click(screen.getByRole('button', { name: /send/i }))

    expect(onSubmit).not.toHaveBeenCalled()
  })

  // Test keyboard interaction: Shift+Enter adds newline
  it('adds newline on Shift+Enter', async () => {
    const onMessageChange = vi.fn()
    render(<ChatForm {...defaultProps} onMessageChange={onMessageChange} />)

    const textarea = screen.getByRole('textbox')
    await userEvent.type(textarea, 'line1{Shift>}{Enter}{/Shift}line2')

    // Verify newline was added (not submitted)
    expect(onMessageChange).toHaveBeenCalledWith(expect.stringContaining('\n'))
  })

  // Test file upload flow
  it('uploads file on paste', async () => {
    const onFilesChange = vi.fn()
    const mockUploadMutation = vi.fn()
    vi.mocked(useUploadFile).mockReturnValue({
      mutate: mockUploadMutation,
      isPending: false,
    })

    render(<ChatForm {...defaultProps} onFilesChange={onFilesChange} />)

    const file = new File(['image'], 'test.png', { type: 'image/png' })
    const pasteEvent = createPasteEvent([file])

    await userEvent.click(screen.getByRole('textbox'))
    await userEvent.paste(pasteEvent)

    expect(mockUploadMutation).toHaveBeenCalledWith(file)
  })

  // Test error state
  it('shows error toast when file upload fails', async () => {
    vi.mocked(useUploadFile).mockReturnValue({
      mutate: vi.fn((_, { onError }) => onError(new Error('Upload failed'))),
      isPending: false,
    })

    render(<ChatForm {...defaultProps} />)

    // Trigger upload and verify error shown
    await waitFor(() => {
      expect(screen.getByText(/failed to upload/i)).toBeInTheDocument()
    })
  })
})
```

#### Example 2: Tier 2 Component (PopoverSelector) - Focused Testing

```typescript
// PopoverSelector.test.tsx - ~50 lines for key states
import { render, screen } from '@testing-library/react'
import { userEvent } from '@testing-library/user-event'
import { FileIcon } from 'lucide-react'
import { PopoverSelector } from '../popover-selector'

describe('PopoverSelector', () => {
  // Test loading state
  it('shows loading text when loading', () => {
    render(
      <PopoverSelector icon={FileIcon} loading placeholder="Select">
        <div>Options</div>
      </PopoverSelector>
    )

    expect(screen.getByText('Loading...')).toBeInTheDocument()
  })

  // Test error state
  it('disables trigger when disabled', () => {
    render(
      <PopoverSelector icon={FileIcon} disabled placeholder="Select">
        <div>Options</div>
      </PopoverSelector>
    )

    const trigger = screen.getByRole('button')
    expect(trigger).toBeDisabled()
  })

  // Test value display
  it('displays selected value', () => {
    render(
      <PopoverSelector
        icon={FileIcon}
        value="file1"
        displayValue="README.md"
        placeholder="Select"
      >
        <div>Options</div>
      </PopoverSelector>
    )

    expect(screen.getByText('README.md')).toBeInTheDocument()
  })

  // Test keyboard interaction
  it('opens popover on click', async () => {
    render(
      <PopoverSelector icon={FileIcon} placeholder="Select">
        <div>File options</div>
      </PopoverSelector>
    )

    await userEvent.click(screen.getByRole('button'))

    expect(screen.getByText('File options')).toBeInTheDocument()
  })
})
```

## Common Testing Patterns

Mock external dependencies only, test real component integration:

```typescript
// Mock API hooks
vi.mock('@/features/tasks/hooks/use-tasks')

it('displays task data through component hierarchy', () => {
  vi.mocked(useTask).mockReturnValue(createMockQuery(taskData))
  render(<TaskDetail />)
  expect(screen.getByText(taskData.title)).toBeInTheDocument()
})

// Test loading state
it('shows loading skeleton', () => {
  vi.mocked(useTask).mockReturnValue(createMockLoadingQuery())
  render(<TaskViewer />)
  expect(screen.getByTestId('task-skeleton')).toBeInTheDocument()
})

// Test error state
it('displays error message', () => {
  vi.mocked(useTask).mockReturnValue(createMockErrorQuery(new Error('Failed')))
  render(<TaskViewer />)
  expect(screen.getByText(/Failed/)).toBeInTheDocument()
})
```

## Test Quality Checklist

**Before committing tests:**
- [ ] Classified component using Tier system (1/2/3)
- [ ] Tests match coverage target for tier
- [ ] vi.mock() calls placed before all imports
- [ ] Used centralized mock utilities (@moatless/types, @/test-utils/mocks)
- [ ] No `.skip()` or `.only()` in committed code
- [ ] Tests complete in <1 second (unit) or <5 seconds (integration)
- [ ] Clean up (timers, subscriptions, EventSource)

## Related Patterns

- [Strategic Mocking Pattern](docs/patterns/strategic-mocking.md) - Detailed mocking frameworks