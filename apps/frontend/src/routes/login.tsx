import { useQuery } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'

import { PasswordLoginForm } from '@/features/auth/components/password-login-form'
import { authModeQueryOptions } from '@/features/auth/hooks/use-auth-mode'

const API_BASE_URL = import.meta.env.PROD ? '' : (import.meta.env.VITE_API_URL ?? '')

export const Route = createFileRoute('/login')({
  component: LoginPage,
})

function OAuthSignInLink() {
  return (
    <a
      href={`${API_BASE_URL}/api/v1/auth/login`}
      className="inline-flex items-center justify-center rounded-md bg-primary px-6 py-2.5 text-sm font-medium text-primary-foreground shadow-sm hover:bg-primary/90 transition-colors"
    >
      Sign in
    </a>
  )
}

function LoginPage() {
  const { data, isLoading, isError } = useQuery(authModeQueryOptions())

  const showPasswordForm = !isError && data?.mode === 'password'

  return (
    <div className="flex min-h-screen items-center justify-center bg-background">
      <div className="text-center max-w-sm w-full mx-auto p-8">
        <h1 className="text-3xl font-bold tracking-tight mb-2">Moatless Vibe</h1>
        <p className="text-muted-foreground mb-8">Sign in to continue</p>
        {/* Fall back to the OAuth link on error as well — an unreachable mode
            endpoint must not brick sign-in on OAuth deployments. */}
        {isLoading ? null : showPasswordForm ? <PasswordLoginForm /> : <OAuthSignInLink />}
      </div>
    </div>
  )
}
