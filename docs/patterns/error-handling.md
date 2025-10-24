# Error Handling Patterns

## Problem

Complex applications face numerous error scenarios that can degrade user experience:
- Component failures can cascade and crash entire applications
- Users see generic error messages that don't help them recover
- Developers struggle to debug issues without proper error context
- Network failures, API errors, and runtime exceptions need different handling strategies
- Error recovery mechanisms are inconsistent across the application

## Solution

Strategic Error Handling uses multiple layers of error boundaries and recovery mechanisms:
- **Error Boundaries**: Catch JavaScript errors in component trees
- **API Error Handling**: Consistent error responses and retry logic
- **Graceful Degradation**: Fallback UI when features fail
- **Error Recovery**: User-friendly options to recover from errors
- **Error Tracking**: Comprehensive logging and monitoring

## Implementation

### Typed Error Construction

- MUST construct errors with `new Error(message)` or domain subclasses; NEVER throw strings or plain objects.
- SHOULD capture context on the error instance (`this.cause`, `this.details`) instead of relying on console output.
- MUST NOT suppress type safety with `// @ts-ignore`; add proper types or guards.

```typescript
export class UiRenderError extends Error {
  constructor(componentName: string, cause: unknown) {
    super(`Failed to render ${componentName}`)
    this.name = 'UiRenderError'
    this.cause = cause
  }
}

function renderSection(componentName: string, render: () => ReactNode) {
  try {
    return render()
  } catch (error) {
    throw new UiRenderError(componentName, error)
  }
}
```

### React Error Boundaries

```typescript
// Base error boundary component
interface ErrorBoundaryState {
  hasError: boolean;
  error: Error | null;
  errorInfo: ErrorInfo | null;
  errorId: string | null;
}

interface ErrorBoundaryProps {
  children: React.ReactNode;
  componentName: string;
  fallback?: (error: Error, reset: () => void) => React.ReactNode;
  onError?: (error: Error, errorInfo: ErrorInfo) => void;
  resetKeys?: Array<string | number>;
  resetOnPropsChange?: boolean;
}

export class ErrorBoundary extends Component<
  ErrorBoundaryProps,
  ErrorBoundaryState
> {
  private resetTimeoutId: number | null = null;
  private prevResetKeys: Array<string | number> = [];
  
  constructor(props: ErrorBoundaryProps) {
    super(props);
    
    this.state = {
      hasError: false,
      error: null,
      errorInfo: null,
      errorId: null,
    };
    
    this.prevResetKeys = props.resetKeys || [];
  }
  
  static getDerivedStateFromError(error: Error): Partial<ErrorBoundaryState> {
    const errorId = generateErrorId();
    
    return {
      hasError: true,
      error,
      errorId,
    };
  }
  
  componentDidCatch(error: Error, errorInfo: ErrorInfo) {
    const { componentName, onError } = this.props;
    
    // Log error with context
    console.error(`Error in ${componentName}:`, error, errorInfo);
    
    // Track error for monitoring
    this.trackError(error, errorInfo);
    
    // Update state with error info
    this.setState({ errorInfo });
    
    // Call custom error handler
    onError?.(error, errorInfo);
  }
  
  componentDidUpdate(prevProps: ErrorBoundaryProps) {
    const { resetKeys = [], resetOnPropsChange = true } = this.props;
    const { hasError } = this.state;
    
    if (hasError && resetOnPropsChange) {
      // Check if reset keys changed
      if (resetKeys.length > 0) {
        const hasResetKeyChanged = resetKeys.some(
          (key, idx) => key !== this.prevResetKeys[idx]
        );
        
        if (hasResetKeyChanged) {
          this.reset();
        }
      }
    }
    
    this.prevResetKeys = resetKeys;
  }
  
  reset = () => {
    if (this.resetTimeoutId) {
      clearTimeout(this.resetTimeoutId);
    }
    
    this.resetTimeoutId = window.setTimeout(() => {
      this.setState({
        hasError: false,
        error: null,
        errorInfo: null,
        errorId: null,
      });
    }, 100);
  };
  
  private trackError(error: Error, errorInfo: ErrorInfo) {
    // Send to error tracking service
    if (typeof window !== 'undefined' && window.gtag) {
      window.gtag('event', 'exception', {
        description: error.message,
        fatal: false,
        component: this.props.componentName,
        stack: error.stack,
        componentStack: errorInfo.componentStack,
      });
    }
    
    // Log to console in development
    if (process.env.NODE_ENV === 'development') {
      console.group(`🚨 Error Boundary: ${this.props.componentName}`);
      console.error('Error:', error);
      console.error('Component Stack:', errorInfo.componentStack);
      console.groupEnd();
    }
  }
  
  render() {
    const { hasError, error } = this.state;
    const { children, fallback, componentName } = this.props;
    
    if (hasError && error) {
      if (fallback) {
        return fallback(error, this.reset);
      }
      
      return (
        <DefaultErrorFallback
          error={error}
          componentName={componentName}
          onReset={this.reset}
        />
      );
    }
    
    return children;
  }
}
```

### Default Error Fallback UI

```typescript
interface DefaultErrorFallbackProps {
  error: Error;
  componentName: string;
  onReset: () => void;
}

function DefaultErrorFallback({
  error,
  componentName,
  onReset,
}: DefaultErrorFallbackProps) {
  const [showDetails, setShowDetails] = useState(false);
  
  return (
    <Alert variant="destructive" className="error-boundary">
      <AlertTriangle className="h-4 w-4" />
      <AlertTitle>Something went wrong in {componentName}</AlertTitle>
      <AlertDescription>
        <p className="mb-3">
          An unexpected error occurred. You can try reloading this section or
          refreshing the page.
        </p>
        
        <div className="flex gap-2 mb-3">
          <Button
            variant="outline"
            size="sm"
            onClick={onReset}
          >
            <RefreshCw className="h-3 w-3 mr-1" />
            Try Again
          </Button>
          
          <Button
            variant="outline"
            size="sm"
            onClick={() => window.location.reload()}
          >
            <RotateCcw className="h-3 w-3 mr-1" />
            Refresh Page
          </Button>
          
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setShowDetails(!showDetails)}
          >
            {showDetails ? 'Hide' : 'Show'} Details
          </Button>
        </div>
        
        {showDetails && (
          <details className="mt-3">
            <summary className="cursor-pointer text-sm font-medium">
              Error Details
            </summary>
            <pre className="mt-2 text-xs bg-muted p-2 rounded overflow-auto">
              {error.message}
              {error.stack && `\n\n${error.stack}`}
            </pre>
          </details>
        )}
      </AlertDescription>
    </Alert>
  );
}
```

### Safe Boundary with Suspense

```typescript
// Combined error boundary and suspense for async components
interface SafeBoundaryProps {
  children: React.ReactNode;
  componentName: string;
  loadingFallback?: React.ReactNode;
  errorFallback?: (error: Error, reset: () => void) => React.ReactNode;
  resetKeys?: Array<string | number>;
}

export function SafeBoundary({
  children,
  componentName,
  loadingFallback = <ComponentSkeleton />,
  errorFallback,
  resetKeys,
}: SafeBoundaryProps) {
  return (
    <ErrorBoundary
      componentName={componentName}
      fallback={errorFallback}
      resetKeys={resetKeys}
    >
      <Suspense fallback={loadingFallback}>
        {children}
      </Suspense>
    </ErrorBoundary>
  );
}
```

### API Error Handling

```typescript
// Custom error types
export class ApiError extends Error {
  constructor(
    message: string,
    public statusCode: number,
    public code?: string,
    public details?: any
  ) {
    super(message);
    this.name = 'ApiError';
  }
}

export class NetworkError extends Error {
  constructor(message: string, public originalError?: Error) {
    super(message);
    this.name = 'NetworkError';
  }
}

export class ValidationError extends Error {
  constructor(
    message: string,
    public field: string,
    public value: any
  ) {
    super(message);
    this.name = 'ValidationError';
  }
}

// HTTP client with error handling
async function apiRequest<T>(
  url: string,
  options: RequestInit = {}
): Promise<T> {
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), 30000);
  
  try {
    const response = await fetch(url, {
      ...options,
      signal: controller.signal,
      headers: {
        'Content-Type': 'application/json',
        ...options.headers,
      },
    });
    
    clearTimeout(timeoutId);
    
    if (!response.ok) {
      const errorData = await response.json().catch(() => ({}));
      
      throw new ApiError(
        errorData.message || `HTTP ${response.status}`,
        response.status,
        errorData.code,
        errorData.details
      );
    }
    
    return await response.json();
  } catch (error) {
    clearTimeout(timeoutId);
    
    if (error instanceof ApiError) {
      throw error;
    }
    
    if (error.name === 'AbortError') {
      throw new NetworkError('Request timeout');
    }
    
    throw new NetworkError(
      'Network request failed',
      error as Error
    );
  }
}

// React Query error handling
export function useApiWithErrorHandling<T>(
  queryKey: QueryKey,
  queryFn: () => Promise<T>,
  options?: UseQueryOptions<T>
) {
  return useQuery({
    queryKey,
    queryFn,
    retry: (failureCount, error) => {
      // Don't retry client errors (4xx)
      if (error instanceof ApiError && error.statusCode < 500) {
        return false;
      }
      
      // Retry up to 3 times for server errors
      return failureCount < 3;
    },
    retryDelay: (attemptIndex) => Math.min(1000 * 2 ** attemptIndex, 30000),
    ...options,
  });
}
```

### Error Display Components

```typescript
// Generic error alert component
interface ErrorAlertProps {
  error: Error | null;
  title?: string;
  onRetry?: () => void;
  onDismiss?: () => void;
  showDetails?: boolean;
}

export function ErrorAlert({
  error,
  title = 'An error occurred',
  onRetry,
  onDismiss,
  showDetails = false,
}: ErrorAlertProps) {
  const [detailsOpen, setDetailsOpen] = useState(false);
  
  if (!error) return null;
  
  const errorMessage = getErrorMessage(error);
  const isRetryable = isRetryableError(error);
  
  return (
    <Alert variant="destructive">
      <AlertTriangle className="h-4 w-4" />
      <AlertTitle>{title}</AlertTitle>
      <AlertDescription>
        <p className="mb-3">{errorMessage}</p>
        
        <div className="flex gap-2">
          {isRetryable && onRetry && (
            <Button variant="outline" size="sm" onClick={onRetry}>
              <RefreshCw className="h-3 w-3 mr-1" />
              Try Again
            </Button>
          )}
          
          {onDismiss && (
            <Button variant="ghost" size="sm" onClick={onDismiss}>
              Dismiss
            </Button>
          )}
          
          {showDetails && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setDetailsOpen(!detailsOpen)}
            >
              {detailsOpen ? 'Hide' : 'Show'} Details
            </Button>
          )}
        </div>
        
        {showDetails && detailsOpen && (
          <Collapsible open={detailsOpen}>
            <CollapsibleContent>
              <pre className="mt-3 text-xs bg-muted p-2 rounded overflow-auto">
                {error.stack || error.toString()}
              </pre>
            </CollapsibleContent>
          </Collapsible>
        )}
      </AlertDescription>
    </Alert>
  );
}

// Hook for error state management
export function useErrorHandler() {
  const [error, setError] = useState<Error | null>(null);
  
  const handleError = useCallback((error: Error) => {
    console.error('Error handled:', error);
    setError(error);
    
    // Track error
    if (window.gtag) {
      window.gtag('event', 'exception', {
        description: error.message,
        fatal: false,
      });
    }
  }, []);
  
  const clearError = useCallback(() => {
    setError(null);
  }, []);
  
  const retry = useCallback((fn: () => void | Promise<void>) => {
    clearError();
    try {
      const result = fn();
      if (result instanceof Promise) {
        result.catch(handleError);
      }
    } catch (error) {
      handleError(error as Error);
    }
  }, [clearError, handleError]);
  
  return {
    error,
    hasError: error !== null,
    handleError,
    clearError,
    retry,
  };
}
```

### Error Utility Functions

```typescript
// Extract user-friendly error messages
export function getErrorMessage(error: Error): string {
  if (error instanceof ApiError) {
    switch (error.statusCode) {
      case 400:
        return 'Invalid request. Please check your input and try again.';
      case 401:
        return 'You need to log in to perform this action.';
      case 403:
        return 'You don\'t have permission to perform this action.';
      case 404:
        return 'The requested resource was not found.';
      case 409:
        return 'This action conflicts with the current state. Please refresh and try again.';
      case 429:
        return 'Too many requests. Please wait a moment and try again.';
      case 500:
        return 'Server error. Please try again later.';
      default:
        return error.message || 'An unexpected error occurred.';
    }
  }
  
  if (error instanceof NetworkError) {
    return 'Network connection failed. Please check your internet connection.';
  }
  
  if (error instanceof ValidationError) {
    return `Invalid ${error.field}: ${error.message}`;
  }
  
  return error.message || 'An unexpected error occurred.';
}

// Check if error is retryable
export function isRetryableError(error: Error): boolean {
  if (error instanceof ApiError) {
    // Retry server errors and some client errors
    return error.statusCode >= 500 || error.statusCode === 408 || error.statusCode === 429;
  }
  
  if (error instanceof NetworkError) {
    return true;
  }
  
  return false;
}

// Generate unique error IDs for tracking
export function generateErrorId(): string {
  return `error_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;
}
```

### Error Context Provider

```typescript
// Global error context for app-wide error handling
interface ErrorContextValue {
  reportError: (error: Error, context?: string) => void;
  errors: Array<{ id: string; error: Error; context?: string; timestamp: Date }>;
  dismissError: (id: string) => void;
  clearAllErrors: () => void;
}

const ErrorContext = React.createContext<ErrorContextValue | null>(null);

export function ErrorProvider({ children }: { children: React.ReactNode }) {
  const [errors, setErrors] = useState<ErrorContextValue['errors']>([]);
  
  const reportError = useCallback((error: Error, context?: string) => {
    const errorEntry = {
      id: generateErrorId(),
      error,
      context,
      timestamp: new Date(),
    };
    
    setErrors(prev => [...prev, errorEntry]);
    
    // Auto-dismiss after 10 seconds
    setTimeout(() => {
      setErrors(prev => prev.filter(e => e.id !== errorEntry.id));
    }, 10000);
  }, []);
  
  const dismissError = useCallback((id: string) => {
    setErrors(prev => prev.filter(e => e.id !== id));
  }, []);
  
  const clearAllErrors = useCallback(() => {
    setErrors([]);
  }, []);
  
  const value = useMemo(() => ({
    reportError,
    errors,
    dismissError,
    clearAllErrors,
  }), [reportError, errors, dismissError, clearAllErrors]);
  
  return (
    <ErrorContext.Provider value={value}>
      {children}
      <ErrorToaster errors={errors} onDismiss={dismissError} />
    </ErrorContext.Provider>
  );
}

export function useErrorContext() {
  const context = useContext(ErrorContext);
  if (!context) {
    throw new Error('useErrorContext must be used within ErrorProvider');
  }
  return context;
}
```

## Usage Patterns

### Strategic Error Boundary Placement

```typescript
// App-level boundary
function App() {
  return (
    <ErrorProvider>
      <ErrorBoundary componentName="App Root">
        <Router>
          <Routes>
            <Route path="/tasks" element={<TaskRoutes />} />
            <Route path="/settings" element={<SettingsRoutes />} />
          </Routes>
        </Router>
      </ErrorBoundary>
    </ErrorProvider>
  );
}

// Feature-level boundaries
function TaskRoutes() {
  return (
    <SafeBoundary componentName="Task Feature">
      <Routes>
        <Route path="/" element={<TaskList />} />
        <Route path="/:id" element={<TaskDetail />} />
      </Routes>
    </SafeBoundary>
  );
}

// Component-level boundaries for critical sections
function TaskDetail({ taskId }: { taskId: string }) {
  return (
    <div>
      <SafeBoundary
        componentName="Task Header"
        resetKeys={[taskId]}
      >
        <TaskHeader taskId={taskId} />
      </SafeBoundary>
      
      <SafeBoundary
        componentName="Task Content"
        resetKeys={[taskId]}
      >
        <TaskContent taskId={taskId} />
      </SafeBoundary>
    </div>
  );
}
```

## Best Practices

### Do's
- **Place boundaries strategically** around feature modules
- **Provide meaningful fallback UI**
- **Include retry mechanisms** for recoverable errors
- **Log errors with sufficient context**
- **Test error scenarios** regularly
- **Use typed errors** for better handling

### Don'ts
- Don't catch errors that should crash the app (security issues)
- Don't show technical stack traces to end users
- Don't ignore error boundaries in testing
- Don't create too many nested error boundaries
- Don't forget to clean up resources in error states
