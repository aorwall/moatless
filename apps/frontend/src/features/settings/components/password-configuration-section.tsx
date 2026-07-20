import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { changePasswordHandler } from '@moatless/api-client/sandbox/auth/auth'
import { useMutation } from '@tanstack/react-query'
import { Loader2 } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import {
  ConfigurationGroup,
  ConfigurationItem,
  ConfigurationSectionWrapper,
} from '@/lib/components/configuration'
import { Alert, AlertDescription } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'
import { getErrorMessage } from '@/lib/utils/error'

/** Mirrors the server's minimum in `validate_password`. */
const MIN_PASSWORD_LENGTH = 12

export function PasswordConfigurationSection() {
  const [currentPassword, setCurrentPassword] = useState('')
  const [newPassword, setNewPassword] = useState('')
  const [confirmPassword, setConfirmPassword] = useState('')

  const changePassword = useMutation({
    mutationFn: (request: { currentPassword: string; newPassword: string }) =>
      unwrapResponse(changePasswordHandler(request)),
    // A wrong current password answers 401; without this the global handler
    // would bounce the user to /login mid-form.
    meta: { skipAuthRedirect: true },
    onSuccess: () => {
      // The server drops every session on success, so the next request 401s
      // and the user re-authenticates with the new password.
      toast.success('Password changed. Please sign in again.')
      window.location.replace('/login')
    },
    onError: (error) => {
      toast.error(`Failed to change password: ${getErrorMessage(error)}`)
    },
  })

  const tooShort = newPassword.length > 0 && newPassword.length < MIN_PASSWORD_LENGTH
  const mismatch = confirmPassword.length > 0 && newPassword !== confirmPassword
  const canSubmit =
    currentPassword.length > 0 &&
    newPassword.length >= MIN_PASSWORD_LENGTH &&
    newPassword === confirmPassword

  const handleSubmit = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    changePassword.mutate({ currentPassword, newPassword })
  }

  return (
    <ConfigurationSectionWrapper>
      <form onSubmit={handleSubmit}>
        <ConfigurationGroup title="Password" showSeparator={false}>
          {tooShort && (
            <Alert variant="destructive" role="alert">
              <AlertDescription>
                New password must be at least {MIN_PASSWORD_LENGTH} characters.
              </AlertDescription>
            </Alert>
          )}
          {mismatch && (
            <Alert variant="destructive" role="alert">
              <AlertDescription>Passwords do not match.</AlertDescription>
            </Alert>
          )}

          <ConfigurationItem label="Current password" description="Your existing password">
            <Input
              id="current-password"
              type="password"
              autoComplete="current-password"
              value={currentPassword}
              onChange={(e) => setCurrentPassword(e.target.value)}
            />
          </ConfigurationItem>
          <ConfigurationItem
            label="New password"
            description={`At least ${MIN_PASSWORD_LENGTH} characters`}
          >
            <Input
              id="new-password"
              type="password"
              autoComplete="new-password"
              value={newPassword}
              onChange={(e) => setNewPassword(e.target.value)}
            />
          </ConfigurationItem>
          <ConfigurationItem label="Confirm new password" description="Type it again">
            <Input
              id="confirm-password"
              type="password"
              autoComplete="new-password"
              value={confirmPassword}
              onChange={(e) => setConfirmPassword(e.target.value)}
            />
          </ConfigurationItem>

          <div className="flex justify-end pt-2">
            <Button type="submit" disabled={!canSubmit || changePassword.isPending}>
              {changePassword.isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              Change Password
            </Button>
          </div>
        </ConfigurationGroup>
      </form>
    </ConfigurationSectionWrapper>
  )
}
