import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { Settings } from 'lucide-react'
import { useState } from 'react'
import { SettingsDialog, SystemStatusCard } from '@/features/settings'
import { Button } from '@/lib/components/ui/button'

export const Route = createFileRoute('/settings')({
  component: SettingsPage,
})

function SettingsPage() {
  const navigate = useNavigate()
  const [showDialog, setShowDialog] = useState(true) // Auto-open dialog on route load

  const handleDialogChange = (open: boolean) => {
    setShowDialog(open)
    if (!open) {
      navigate({ to: '/' })
    }
  }

  return (
    <div className="flex min-h-screen flex-col">
      <div className="flex-1 space-y-6 p-6">
        <div className="mx-auto max-w-6xl">
          <div className="mb-6">
            <SystemStatusCard />
          </div>

          <div className="flex justify-center">
            <Button onClick={() => setShowDialog(true)} size="lg">
              <Settings className="mr-2 h-4 w-4" />
              Open Settings
            </Button>
          </div>

          <SettingsDialog open={showDialog} onOpenChange={handleDialogChange} />
        </div>
      </div>
    </div>
  )
}
