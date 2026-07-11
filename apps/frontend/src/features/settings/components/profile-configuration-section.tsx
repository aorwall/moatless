import { Loader2 } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import { useAuth } from '@/features/auth/hooks/use-auth'
import {
  ConfigurationGroup,
  ConfigurationItem,
  ConfigurationSectionWrapper,
} from '@/lib/components/configuration'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'
import { getErrorMessage } from '@/lib/utils/error'
import { useUpdateProfile } from '../hooks/use-profile'

type AuthUser = NonNullable<ReturnType<typeof useAuth>['user']>

export function ProfileConfigurationSection() {
  const { user, isLoading } = useAuth()

  if (isLoading) {
    return (
      <ConfigurationSectionWrapper>
        <div className="flex items-center justify-center py-8">
          <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
        </div>
      </ConfigurationSectionWrapper>
    )
  }

  if (!user) {
    return (
      <ConfigurationSectionWrapper>
        <Alert variant="destructive">
          <AlertTitle>Not authenticated</AlertTitle>
          <AlertDescription>Please log in to view your profile settings.</AlertDescription>
        </Alert>
      </ConfigurationSectionWrapper>
    )
  }

  // Remount on user change so the form seeds fresh state from the loaded
  // session — no effect mirroring query data into local state.
  return (
    <ConfigurationSectionWrapper>
      <ProfileForm key={user.login} user={user} />
    </ConfigurationSectionWrapper>
  )
}

function ProfileForm({ user }: { user: AuthUser }) {
  const updateProfile = useUpdateProfile()

  const [name, setName] = useState(user.name ?? '')
  const [email, setEmail] = useState(user.email ?? '')

  const hasChanges = name !== (user.name ?? '') || email !== (user.email ?? '')

  const handleSave = () => {
    updateProfile.mutate(
      {
        name: name || undefined,
        email: email || undefined,
      },
      {
        onSuccess: () => {
          toast.success('Profile updated successfully')
        },
        onError: (error) => {
          toast.error(`Failed to update profile: ${getErrorMessage(error)}`)
        },
      },
    )
  }

  return (
    <ConfigurationGroup title="Profile" showSeparator={false}>
      <ConfigurationItem label="Name" description="Your display name">
        <Input
          id="config-name"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="Enter your name"
        />
      </ConfigurationItem>
      <ConfigurationItem label="Email" description="Your email address">
        <Input
          id="config-email"
          type="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          placeholder="Enter your email"
        />
      </ConfigurationItem>
      <ConfigurationItem label="Login" description="Your login username">
        <Input id="config-login" value={user.login} disabled />
      </ConfigurationItem>
      <ConfigurationItem label="Provider" description="Authentication provider">
        <Input id="config-provider" value={user.provider} disabled />
      </ConfigurationItem>
      <ConfigurationItem label="Role" description="Your account role">
        <Input id="config-role" value={user.role ?? ''} disabled />
      </ConfigurationItem>
      <div className="flex justify-end pt-2">
        <Button onClick={handleSave} disabled={!hasChanges || updateProfile.isPending}>
          {updateProfile.isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
          Save Changes
        </Button>
      </div>
    </ConfigurationGroup>
  )
}
