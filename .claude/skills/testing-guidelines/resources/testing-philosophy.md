# Testing Philosophy

## Core Principles

### 1. Test Behavior, Not Implementation
Focus on what users see and experience, not internal implementation details.

**❌ Wrong - Testing Implementation**:
```tsx
test('useState is called correctly', () => {
  const spy = vi.spyOn(React, 'useState')
  render(<Counter />)
  expect(spy).toHaveBeenCalled()  // Implementation detail!
})

test('has correct CSS class', () => {
  render(<Button />)
  expect(screen.getByRole('button')).toHaveClass('px-4 py-2')  // Styling detail!
})
```

**✅ Correct - Testing Behavior**:
```tsx
test('increments counter when button clicked', async () => {
  const user = userEvent.setup()
  render(<Counter />)

  await user.click(screen.getByRole('button', { name: 'Increment' }))

  expect(screen.getByText('Count: 1')).toBeInTheDocument()  // User-visible!
})

test('button is disabled when loading', () => {
  render(<Button loading />)
  expect(screen.getByRole('button')).toBeDisabled()  // User experience!
})
```

### 2. Test the Contract, Not the Implementation
Test the public API of components/functions, not how they work internally.

**What to Test**:
- Props interface
- Rendered output
- User interactions
- Side effects (API calls, state updates)

**What NOT to Test**:
- Internal state variables
- Private methods
- Helper functions (test through public API)
- Implementation details

### 3. Write Tests Users Would Write
If a user can't see or interact with it, don't test it.

```tsx
// ❌ User can't see internal state
test('sets loading state to true', () => {
  const { result } = renderHook(() => useTask())
  expect(result.current.isLoading).toBe(true)
})

// ✅ User can see loading indicator
test('shows loading spinner while fetching', () => {
  render(<TaskList />)
  expect(screen.getByRole('status')).toBeInTheDocument()
})
```

### 4. Prefer Integration Over Isolation
Test components together as users would use them.

**❌ Over-Isolated**:
```tsx
// Testing TaskCard in complete isolation
test('TaskCard renders', () => {
  render(<TaskCard task={mockTask} />)
  expect(screen.getByText(mockTask.title)).toBeInTheDocument()
})

// Testing TaskList in complete isolation
test('TaskList renders', () => {
  render(<TaskList tasks={[]} />)
  expect(screen.getByText('No tasks')).toBeInTheDocument()
})
```

**✅ Integrated**:
```tsx
// Testing TaskList with real TaskCard components
test('displays list of tasks', () => {
  const tasks = [
    { id: '1', title: 'Task 1' },
    { id: '2', title: 'Task 2' }
  ]

  render(<TaskList tasks={tasks} />)

  expect(screen.getByText('Task 1')).toBeInTheDocument()
  expect(screen.getByText('Task 2')).toBeInTheDocument()
})
```

### 5. Test Happy Path and Edge Cases
Cover both success scenarios and failure scenarios.

```tsx
describe('TaskForm', () => {
  // Happy path
  test('creates task with valid data', async () => {
    // ...
  })

  // Edge cases
  test('shows error when title is empty', async () => {
    // ...
  })

  test('disables submit button while loading', async () => {
    // ...
  })

  test('handles server error gracefully', async () => {
    // ...
  })
})
```

### 6. Keep Tests Simple and Focused
One test should verify one behavior.

**❌ Too Much in One Test**:
```tsx
test('form works', async () => {
  // Tests multiple behaviors
  render(<TaskForm />)
  await user.type(screen.getByLabelText('Title'), 'Task')
  expect(screen.getByLabelText('Title')).toHaveValue('Task')  // Value update
  await user.click(screen.getByRole('button'))
  expect(createTask).toHaveBeenCalled()  // Submission
  expect(screen.getByText('Success')).toBeInTheDocument()  // Success message
})
```

**✅ Focused Tests**:
```tsx
test('updates input value when user types', async () => {
  const user = userEvent.setup()
  render(<TaskForm />)

  await user.type(screen.getByLabelText('Title'), 'Task')

  expect(screen.getByLabelText('Title')).toHaveValue('Task')
})

test('calls createTask when form submitted', async () => {
  const user = userEvent.setup()
  render(<TaskForm />)

  await user.type(screen.getByLabelText('Title'), 'Task')
  await user.click(screen.getByRole('button', { name: 'Create' }))

  expect(createTask).toHaveBeenCalled()
})
```

## Testing Philosophy Summary

1. **Behavior over Implementation** - Test what users see
2. **Contract over Internals** - Test public API
3. **User Perspective** - Test as users would use it
4. **Integration over Isolation** - Test components together
5. **Happy + Edge Cases** - Cover success and failures
6. **Simple + Focused** - One behavior per test
