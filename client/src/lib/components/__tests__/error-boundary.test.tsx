import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ErrorBoundary } from '../error-boundary';

// Component that throws an error
function ThrowError({ shouldThrow }: { shouldThrow: boolean }) {
  if (shouldThrow) {
    throw new Error('Test error');
  }
  return <div>No error</div>;
}

describe('ErrorBoundary', () => {
  // Suppress console.error for these tests
  const originalError = console.error;
  beforeEach(() => {
    console.error = vi.fn();
  });
  afterEach(() => {
    console.error = originalError;
  });

  it('renders children when there is no error', () => {
    render(
      <ErrorBoundary>
        <ThrowError shouldThrow={false} />
      </ErrorBoundary>,
    );

    expect(screen.getByText('No error')).toBeInTheDocument();
  });

  it('renders error UI when child component throws', () => {
    render(
      <ErrorBoundary componentName="Test Component">
        <ThrowError shouldThrow={true} />
      </ErrorBoundary>,
    );

    expect(screen.getByText('Test Component Error')).toBeInTheDocument();
    expect(screen.getByText('Test error')).toBeInTheDocument();
    expect(screen.getByText('Try Again')).toBeInTheDocument();
  });

  it('renders custom fallback when provided', () => {
    const customFallback = (error: Error, reset: () => void) => (
      <div>
        <p>Custom error: {error.message}</p>
        <button onClick={reset}>Custom Reset</button>
      </div>
    );

    render(
      <ErrorBoundary fallback={customFallback}>
        <ThrowError shouldThrow={true} />
      </ErrorBoundary>,
    );

    expect(screen.getByText('Custom error: Test error')).toBeInTheDocument();
    expect(screen.getByText('Custom Reset')).toBeInTheDocument();
  });

  it('calls reset function when reset is clicked', async () => {
    const user = userEvent.setup();
    const resetMock = vi.fn();

    const customFallback = (_error: Error, reset: () => void) => (
      <div>
        <p>Error occurred</p>
        <button
          onClick={() => {
            resetMock();
            reset();
          }}
        >
          Reset
        </button>
      </div>
    );

    render(
      <ErrorBoundary fallback={customFallback}>
        <ThrowError shouldThrow={true} />
      </ErrorBoundary>,
    );

    expect(screen.getByText('Error occurred')).toBeInTheDocument();

    // Click reset
    await user.click(screen.getByText('Reset'));

    // Verify reset was called
    expect(resetMock).toHaveBeenCalledTimes(1);
  });

  it('logs error to console', () => {
    render(
      <ErrorBoundary componentName="Test Component">
        <ThrowError shouldThrow={true} />
      </ErrorBoundary>,
    );

    expect(console.error).toHaveBeenCalledWith(
      'Error caught by Test Component:',
      expect.any(Error),
      expect.objectContaining({ componentStack: expect.any(String) }),
    );
  });
});
