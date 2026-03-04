import { createFileRoute } from '@tanstack/react-router'

const API_BASE_URL = import.meta.env.PROD ? '' : (import.meta.env.VITE_API_URL ?? '')

export const Route = createFileRoute('/login')({
  component: LoginPage,
})

function LoginPage() {
  return (
    <div className="flex min-h-screen items-center justify-center bg-background">
      <div className="text-center max-w-sm mx-auto p-8">
        <h1 className="text-3xl font-bold tracking-tight mb-2">Moatless Vibe</h1>
        <p className="text-muted-foreground mb-8">Sign in to continue</p>
        <a
          href={`${API_BASE_URL}/api/v1/auth/login`}
          className="inline-flex items-center justify-center rounded-md bg-primary px-6 py-2.5 text-sm font-medium text-primary-foreground shadow-sm hover:bg-primary/90 transition-colors"
        >
          Sign in
        </a>
      </div>
    </div>
  )
}
