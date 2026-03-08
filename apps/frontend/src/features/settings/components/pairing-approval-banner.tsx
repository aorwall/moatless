import { CheckCircle, Loader2, UserPlus } from 'lucide-react'
import { toast } from 'sonner'
import { useApprovePairing, useMyPairingRequests } from '@/features/settings/hooks/use-channels'
import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { getErrorMessage } from '@/lib/utils/error'

export function PairingApprovalBanner() {
  const { data: requests } = useMyPairingRequests()
  const approve = useApprovePairing()

  if (!requests || requests.length === 0) {
    return null
  }

  return (
    <div className="px-4 pt-3">
      <Alert className="border-amber-500/50 bg-amber-50 dark:bg-amber-950/20">
        <UserPlus className="h-4 w-4 text-amber-600" />
        <AlertTitle className="text-amber-800 dark:text-amber-300">
          Pending pairing requests ({requests.length})
        </AlertTitle>
        <AlertDescription>
          <div className="mt-2 space-y-2">
            {requests.map((req) => {
              const meta = req.meta as Record<string, string> | null
              const displayName = meta?.first_name
                ? `${meta.first_name}${meta.last_name ? ` ${meta.last_name}` : ''}`
                : undefined
              const username = meta?.username

              return (
                <div
                  key={req.id}
                  className="flex items-center justify-between gap-3 rounded-md border border-amber-200 bg-white p-2 dark:border-amber-800 dark:bg-amber-950/30"
                >
                  <div className="min-w-0 flex-1 text-sm">
                    <span className="font-medium text-foreground">
                      {displayName ?? `User ${req.providerUserId}`}
                    </span>
                    {username && <span className="ml-1.5 text-muted-foreground">@{username}</span>}
                    <span className="ml-2 font-mono text-xs text-muted-foreground">{req.code}</span>
                  </div>
                  <Button
                    size="sm"
                    variant="outline"
                    className="shrink-0 border-green-600 text-green-700 hover:bg-green-50 dark:text-green-400 dark:hover:bg-green-950/30"
                    disabled={approve.isPending}
                    onClick={() =>
                      approve.mutate(
                        { configId: req.configId, code: req.code },
                        {
                          onSuccess: () => {
                            toast.success(`Approved ${displayName ?? req.providerUserId}`)
                          },
                          onError: (err) => {
                            toast.error(getErrorMessage(err))
                          },
                        },
                      )
                    }
                  >
                    {approve.isPending ? (
                      <Loader2 className="mr-1 h-3 w-3 animate-spin" />
                    ) : (
                      <CheckCircle className="mr-1 h-3 w-3" />
                    )}
                    Approve
                  </Button>
                </div>
              )
            })}
          </div>
        </AlertDescription>
      </Alert>
    </div>
  )
}
