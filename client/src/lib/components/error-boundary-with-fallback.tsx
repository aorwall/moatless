import { Component, type ReactNode, Suspense } from 'react';

import { ErrorAlert } from '@/lib/components/ui/error-alert';
import { Skeleton } from '@/lib/components/ui/skeleton';

interface ErrorBoundaryProps {
  children: ReactNode;
  fallback?: (error: Error, reset: () => void) => ReactNode;
  componentName?: string;
  onError?: (error: Error, errorInfo: { componentStack: string }) => void;
  resetKeys?: Array<string | number>;
  resetOnPropsChange?: boolean;
  isolate?: boolean;
}

interface ErrorBoundaryState {
  hasError: boolean;
  error: Error | null;
  resetKeys?: Array<string | number>;
}

export class ErrorBoundary extends Component<
  ErrorBoundaryProps,
  ErrorBoundaryState
> {
  constructor(props: ErrorBoundaryProps) {
    super(props);
    this.state = {
      hasError: false,
      error: null,
      resetKeys: props.resetKeys,
    };
  }

  static getDerivedStateFromError(error: Error): Partial<ErrorBoundaryState> {
    return { hasError: true, error };
  }

  static getDerivedStateFromProps(
    props: ErrorBoundaryProps,
    state: ErrorBoundaryState,
  ): Partial<ErrorBoundaryState> | null {
    // Reset error boundary when resetKeys change
    if (props.resetKeys && state.resetKeys) {
      const hasResetKeyChanged = props.resetKeys.some(
        (key, index) => key !== state.resetKeys?.[index],
      );
      if (hasResetKeyChanged) {
        return { hasError: false, error: null, resetKeys: props.resetKeys };
      }
    }

    // Reset on any props change if requested
    if (props.resetOnPropsChange && state.hasError) {
      return { hasError: false, error: null };
    }

    return null;
  }

  componentDidCatch(error: Error, errorInfo: { componentStack: string }) {
    // Log to error reporting service
    console.error(
      `Error caught by ${this.props.componentName || 'ErrorBoundary'}:`,
      error,
      errorInfo,
    );

    // Call custom error handler if provided
    this.props.onError?.(error, errorInfo);

    // In production, you might want to send this to an error tracking service
    if (
      typeof window !== 'undefined' &&
      window.location.hostname !== 'localhost'
    ) {
      // Example: sendToErrorTrackingService(error, errorInfo);
    }
  }

  reset = () => {
    this.setState({ hasError: false, error: null });
  };

  render() {
    if (this.state.hasError && this.state.error) {
      // Isolate mode - render nothing to prevent cascading failures
      if (this.props.isolate) {
        return null;
      }

      // Custom fallback
      if (this.props.fallback) {
        return this.props.fallback(this.state.error, this.reset);
      }

      // Default fallback
      return (
        <div className="h-full flex items-center justify-center p-6">
          <div className="text-center space-y-3">
            <ErrorAlert
              error={this.state.error}
              title={`${this.props.componentName || 'Component'} Error`}
              className="max-w-md w-full"
            />
            <button
              onClick={this.reset}
              className="text-sm text-primary hover:underline focus:outline-none focus:ring-2 focus:ring-primary focus:ring-offset-2 rounded"
              aria-label="Try again"
            >
              Try Again
            </button>
          </div>
        </div>
      );
    }

    return this.props.children;
  }
}

// Wrapper that combines error boundary with Suspense
export function SafeBoundary({
  children,
  componentName,
  fallback,
  loadingFallback,
  ...errorBoundaryProps
}: ErrorBoundaryProps & { loadingFallback?: ReactNode }) {
  return (
    <ErrorBoundary
      componentName={componentName}
      fallback={fallback}
      {...errorBoundaryProps}
    >
      <Suspense
        fallback={
          loadingFallback || (
            <div className="h-full flex items-center justify-center p-6">
              <Skeleton className="h-32 w-full max-w-md" />
            </div>
          )
        }
      >
        {children}
      </Suspense>
    </ErrorBoundary>
  );
}
