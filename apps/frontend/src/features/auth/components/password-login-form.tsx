import { useState } from 'react'

import { Alert, AlertDescription } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'
import { Label } from '@/lib/components/ui/label'

import { usePasswordLogin } from '../hooks/use-password-login'

/**
 * Username/password sign-in form.
 *
 * The error message is deliberately identical for an unknown user and a wrong
 * password — the server answers both the same way, and repeating that here
 * keeps the UI from becoming an account-existence oracle.
 */
export function PasswordLoginForm() {
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const login = usePasswordLogin()

  const handleSubmit = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    login.mutate({ username, password })
  }

  return (
    <form onSubmit={handleSubmit} className="space-y-4 text-left">
      {login.isError && (
        <Alert variant="destructive" role="alert">
          <AlertDescription>Invalid username or password.</AlertDescription>
        </Alert>
      )}

      <div className="space-y-2">
        <Label htmlFor="username">Username</Label>
        <Input
          id="username"
          name="username"
          autoComplete="username"
          autoFocus
          required
          value={username}
          onChange={(e) => setUsername(e.target.value)}
        />
      </div>

      <div className="space-y-2">
        <Label htmlFor="password">Password</Label>
        <Input
          id="password"
          name="password"
          type="password"
          autoComplete="current-password"
          required
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
      </div>

      <Button type="submit" className="w-full" disabled={login.isPending}>
        {login.isPending ? 'Signing in…' : 'Sign in'}
      </Button>
    </form>
  )
}
