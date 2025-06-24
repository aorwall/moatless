import { Component, type ReactNode } from 'react'

import { ErrorAlert } from '@/lib/components/ui/error-alert'

interface ErrorBoundaryProps {
  children: ReactNode
  fallback?: (error: Error, reset: () => void) => ReactNode
  componentName?: string
}

interface ErrorBoundaryState {
  hasError: boolean
  error: Error | null
}

export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  constructor(props: ErrorBoundaryProps) {
    super(props)
    this.state = { hasError: false, error: null }
  }

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { hasError: true, error }
  }

  componentDidCatch(error: Error, errorInfo: { componentStack: string }) {
    console.error(
      `Error caught by ${this.props.componentName || 'ErrorBoundary'}:`,
      error,
      errorInfo,
    )
  }

  reset = () => {
    this.setState({ hasError: false, error: null })
  }

  render() {
    if (this.state.hasError && this.state.error) {
      if (this.props.fallback) {
        return this.props.fallback(this.state.error, this.reset)
      }

      return (
        <div className="h-full flex items-center justify-center p-6">
          <div className="text-center space-y-3">
            <ErrorAlert
              error={this.state.error}
              title={`${this.props.componentName || 'Component'} Error`}
              className="max-w-md w-full"
            />
            <button onClick={this.reset} className="text-sm text-primary hover:underline">
              Try Again
            </button>
          </div>
        </div>
      )
    }

    return this.props.children
  }
}
