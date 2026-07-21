import { useQueryClient } from '@tanstack/react-query'
import { Copy, ExternalLink, Loader2, RefreshCw } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { toast } from 'sonner'
import {
  useCodexDevicePoll,
  useStartCodexDeviceLogin,
} from '@/features/settings/hooks/use-codex-connection'
import { queryKeys } from '@/lib/api/query-keys'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/lib/components/ui/dialog'
import { getErrorMessage } from '@/lib/utils/error'

interface CodexDeviceLoginDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  onSuccess?: () => void
}

export function CodexDeviceLoginDialog({
  open,
  onOpenChange,
  onSuccess,
}: CodexDeviceLoginDialogProps) {
  const queryClient = useQueryClient()
  const startLogin = useStartCodexDeviceLogin()
  const [start, setStart] = useState<{
    deviceAuthId: string
    userCode: string
    verificationUrl: string
    intervalSecs: number
    expiresAt: number
  } | null>(null)
  const [now, setNow] = useState(() => Date.now())
  const startedFor = useRef<boolean>(false)
  const openedExternalFor = useRef<string | null>(null)

  // Kick off the device-code request once when the dialog opens.
  useEffect(() => {
    if (!open) {
      startedFor.current = false
      return
    }
    if (startedFor.current) {
      return
    }
    startedFor.current = true
    startLogin.mutate(undefined, {
      onSuccess: (payload) => {
        setStart({
          deviceAuthId: payload.deviceAuthId,
          userCode: payload.userCode,
          verificationUrl: payload.verificationUrl,
          intervalSecs: Math.max(1, payload.intervalSecs),
          expiresAt: Date.now() + payload.expiresInSecs * 1000,
        })
      },
      onError: (error) => {
        toast.error('Failed to start ChatGPT sign-in', {
          description: getErrorMessage(error),
        })
      },
    })
  }, [open, startLogin])

  // Auto-open the verification URL once per device-code session.
  useEffect(() => {
    if (!start) {
      return
    }
    if (openedExternalFor.current === start.deviceAuthId) {
      return
    }
    openedExternalFor.current = start.deviceAuthId
    window.open(start.verificationUrl, '_blank', 'noopener,noreferrer')
  }, [start])

  const clockExpired = !!start && now >= start.expiresAt
  const remainingMs = start ? Math.max(0, start.expiresAt - now) : 0
  const remainingSecs = Math.ceil(remainingMs / 1000)
  const mins = Math.floor(remainingSecs / 60)
  const secs = remainingSecs % 60

  const poll = useCodexDevicePoll(
    start ? { deviceAuthId: start.deviceAuthId, userCode: start.userCode } : null,
    {
      enabled: open && !!start && !clockExpired,
      refetchInterval: start ? start.intervalSecs * 1000 : 5000,
    },
  )

  const done = poll.data?.status === 'success' || poll.data?.status === 'expired'
  const expired = clockExpired || poll.data?.status === 'expired'

  // Tick the countdown clock once per second while the dialog is open.
  useEffect(() => {
    if (!(open && start && !done && !expired)) {
      return
    }
    const id = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(id)
  }, [open, start, done, expired])

  // Handle terminal poll outcomes.
  useEffect(() => {
    if (!(open && poll.data)) {
      return
    }
    if (poll.data.status === 'success') {
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.codex() })
      toast.success('Signed in to ChatGPT')
      onSuccess?.()
      onOpenChange(false)
    }
  }, [open, poll.data, queryClient, onSuccess, onOpenChange])

  const handleRetry = () => {
    setStart(null)
    startedFor.current = false
    openedExternalFor.current = null
    // Re-trigger by toggling: the open-effect will fire again because
    // startedFor.current is reset and the dependency `open` is still true.
    // Use a microtask to make sure setState has flushed.
    queueMicrotask(() => {
      startedFor.current = true
      startLogin.mutate(undefined, {
        onSuccess: (payload) => {
          setStart({
            deviceAuthId: payload.deviceAuthId,
            userCode: payload.userCode,
            verificationUrl: payload.verificationUrl,
            intervalSecs: Math.max(1, payload.intervalSecs),
            expiresAt: Date.now() + payload.expiresInSecs * 1000,
          })
        },
        onError: (error) => {
          toast.error('Failed to start ChatGPT sign-in', {
            description: getErrorMessage(error),
          })
        },
      })
    })
  }

  const handleCopyCode = () => {
    if (!start) {
      return
    }
    navigator.clipboard.writeText(start.userCode).then(
      () => toast.success('Code copied to clipboard'),
      () => toast.error('Could not copy code to clipboard'),
    )
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Sign in to ChatGPT</DialogTitle>
          <DialogDescription>
            Enter the code below in the ChatGPT sign-in page that just opened.
          </DialogDescription>
        </DialogHeader>

        {startLogin.isPending && !start && (
          <div className="flex items-center justify-center py-8">
            <Loader2 className="h-6 w-6 animate-spin" />
          </div>
        )}

        {start && !expired && (
          <div className="space-y-4">
            <button
              type="button"
              onClick={handleCopyCode}
              className="group w-full rounded-md border border-dashed border-input bg-muted/40 px-4 py-6 text-center transition-colors hover:bg-muted"
            >
              <code className="font-mono text-3xl tracking-[0.4em] sm:text-4xl">
                {start.userCode}
              </code>
              <span className="mt-2 flex items-center justify-center gap-1 text-xs text-muted-foreground group-hover:text-foreground">
                <Copy className="h-3 w-3" />
                Click to copy
              </span>
            </button>

            <div className="flex flex-col items-center gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={() => window.open(start.verificationUrl, '_blank', 'noopener,noreferrer')}
              >
                <ExternalLink className="mr-2 h-4 w-4" />
                Open openai.com to enter the code
              </Button>
              <p className="text-xs text-muted-foreground">
                Code expires in {mins}:{secs.toString().padStart(2, '0')}
              </p>
            </div>

            <div className="flex items-center justify-center gap-2 text-sm text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin" />
              Waiting for sign-in…
            </div>
          </div>
        )}

        {expired && (
          <Alert variant="destructive">
            <AlertTitle>Code expired</AlertTitle>
            <AlertDescription>
              The sign-in code expired before it was approved. Try again to get a fresh code.
            </AlertDescription>
          </Alert>
        )}

        <DialogFooter>
          {expired ? (
            <Button onClick={handleRetry} disabled={startLogin.isPending}>
              <RefreshCw className="mr-2 h-4 w-4" />
              Try again
            </Button>
          ) : null}
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
