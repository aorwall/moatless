import type { VerificationResponse } from '@moatless/types'
import { AlertCircle, CheckCircle, Loader2, XCircle } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import { useVerifyRepository } from '@/features/repositories/hooks/use-repositories'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/lib/components/ui/dialog'
import { getErrorMessage } from '@/lib/utils/error'

interface VerificationDialogProps {
  repositoryId: string
}

export function VerificationDialog({ repositoryId }: VerificationDialogProps) {
  const [open, setOpen] = useState(false)
  const [verificationResult, setVerificationResult] = useState<VerificationResponse | null>(null)

  const verifyRepository = useVerifyRepository({
    onSuccess: (data) => {
      setVerificationResult(data)
      toast.success('Repository verification completed')
    },
    onError: (error) => {
      toast.error('Failed to verify repository', {
        description: getErrorMessage(error),
      })
    },
  })

  const handleVerify = () => {
    setVerificationResult(null)
    verifyRepository.mutate(repositoryId)
  }

  const getStatusIcon = (success: boolean) => {
    return success ? (
      <CheckCircle className="h-4 w-4 text-green-500" />
    ) : (
      <XCircle className="h-4 w-4 text-red-500" />
    )
  }

  const getStatusBadge = (success: boolean) => {
    return (
      <Badge variant={success ? 'default' : 'destructive'}>{success ? 'Success' : 'Failed'}</Badge>
    )
  }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button variant="outline" size="sm">
          <AlertCircle className="h-4 w-4 mr-2" />
          Verify Setup
        </Button>
      </DialogTrigger>
      <DialogContent className="max-w-2xl max-h-[80vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Repository Setup Verification</DialogTitle>
          <DialogDescription>
            Test repository configuration, workspace setup, and environment readiness.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="flex gap-2">
            <Button onClick={handleVerify} disabled={verifyRepository.isPending}>
              {verifyRepository.isPending ? (
                <Loader2 className="h-4 w-4 mr-2 animate-spin" />
              ) : (
                <AlertCircle className="h-4 w-4 mr-2" />
              )}
              {verifyRepository.isPending ? 'Verifying...' : 'Run Verification'}
            </Button>
          </div>

          {verificationResult && (
            <div className="space-y-4">
              {/* Overall Status */}
              <div className="flex items-center gap-2 p-3 border rounded-lg">
                {getStatusIcon(verificationResult.success)}
                <span className="font-medium">Overall Status</span>
                {getStatusBadge(verificationResult.success)}
              </div>

              {/* Workspace Setup */}
              <div className="border rounded-lg p-4">
                <div className="flex items-center gap-2 mb-2">
                  {getStatusIcon(verificationResult.workspaceSetup.success)}
                  <h3 className="font-medium">Workspace Setup</h3>
                  {getStatusBadge(verificationResult.workspaceSetup.success)}
                </div>
                <p className="text-sm text-muted-foreground mb-2">
                  {verificationResult.workspaceSetup.message}
                </p>
                {verificationResult.workspaceSetup.workspacePath && (
                  <p className="text-xs text-muted-foreground font-mono">
                    Path: {verificationResult.workspaceSetup.workspacePath}
                  </p>
                )}
              </div>

              {/* Environment */}
              <div className="border rounded-lg p-4">
                <div className="flex items-center gap-2 mb-2">
                  {getStatusIcon(verificationResult.environment.status === 'ready')}
                  <h3 className="font-medium">
                    Environment ({verificationResult.environment.type})
                  </h3>
                  <Badge
                    variant={
                      verificationResult.environment.status === 'ready' ? 'default' : 'destructive'
                    }
                  >
                    {verificationResult.environment.status}
                  </Badge>
                </div>
                {verificationResult.environment.error && (
                  <p className="text-sm text-muted-foreground">
                    {verificationResult.environment.error}
                  </p>
                )}
              </div>

              {/* Setup Commands */}
              <div className="border rounded-lg p-4">
                <div className="flex items-center gap-2 mb-3">
                  {getStatusIcon(verificationResult.setupCommands.success)}
                  <h3 className="font-medium">Setup Commands</h3>
                  {getStatusBadge(verificationResult.setupCommands.success)}
                </div>

                {verificationResult.setupCommands.commands.length === 0 ? (
                  <p className="text-sm text-muted-foreground">No setup commands configured</p>
                ) : (
                  <div className="space-y-3">
                    {verificationResult.setupCommands.commands.map((command, index) => (
                      <div
                        key={`${command.command}-${index}`}
                        className="border rounded p-3 bg-muted/30"
                      >
                        <div className="flex items-center gap-2 mb-2">
                          {getStatusIcon(command.success)}
                          <code className="text-sm font-mono">{command.command}</code>
                          <Badge variant={command.success ? 'default' : 'destructive'}>
                            Exit {command.exit_code}
                          </Badge>
                        </div>

                        {command.output && (
                          <div className="mt-2">
                            <p className="text-xs text-muted-foreground mb-1">Output:</p>
                            <pre className="text-xs bg-background p-2 rounded border overflow-x-auto">
                              {command.output}
                            </pre>
                          </div>
                        )}

                        {command.error && (
                          <div className="mt-2">
                            <p className="text-xs text-muted-foreground mb-1">Error:</p>
                            <pre className="text-xs bg-destructive/10 text-destructive p-2 rounded border overflow-x-auto">
                              {command.error}
                            </pre>
                          </div>
                        )}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}
