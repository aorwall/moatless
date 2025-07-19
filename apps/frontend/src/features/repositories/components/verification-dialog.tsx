import type { WorkspaceStatusResponse } from '@moatless/types'
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
  const [verificationResult, setVerificationResult] = useState<WorkspaceStatusResponse | null>(null)

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
                {getStatusIcon(
                  verificationResult.agent.state === 'idle' &&
                    verificationResult.environment.status === 'ready',
                )}
                <span className="font-medium">Overall Status</span>
                {getStatusBadge(
                  verificationResult.agent.state === 'idle' &&
                    verificationResult.environment.status === 'ready',
                )}
              </div>

              {/* Workspace Status */}
              <div className="border rounded-lg p-4">
                <div className="flex items-center gap-2 mb-2">
                  {getStatusIcon(verificationResult.agent.state === 'idle')}
                  <h3 className="font-medium">Workspace Status</h3>
                  <Badge
                    variant={verificationResult.agent.state === 'idle' ? 'default' : 'destructive'}
                  >
                    {verificationResult.agent.state}
                  </Badge>
                </div>
                <p className="text-sm text-muted-foreground mb-2">
                  Workspace state: {verificationResult.workspace.state}
                </p>
                {verificationResult.gitStatus && (
                  <div className="mt-2">
                    <p className="text-xs text-muted-foreground mb-1">Git Status:</p>
                    <div className="text-xs space-y-1">
                      <div>Branch: {verificationResult.gitStatus.currentBranch}</div>
                      <div>Base: {verificationResult.gitStatus.baseBranch}</div>
                      <div>
                        Uncommitted changes:{' '}
                        {verificationResult.gitStatus.hasUncommittedChanges ? 'Yes' : 'No'}
                      </div>
                    </div>
                  </div>
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
                  <p className="text-sm text-red-500 mb-2">
                    {verificationResult.environment.error}
                  </p>
                )}
                <div className="text-xs text-muted-foreground space-y-1">
                  <div>
                    Created: {new Date(verificationResult.environment.createdAt).toLocaleString()}
                  </div>
                  <div>
                    File Operations:{' '}
                    {verificationResult.capabilities.fileOperations ? 'Enabled' : 'Disabled'}
                  </div>
                  <div>
                    Command Execution:{' '}
                    {verificationResult.capabilities.commandExecution ? 'Enabled' : 'Disabled'}
                  </div>
                </div>
              </div>

              {/* Git Status */}
              {verificationResult.gitStatus && (
                <div className="border rounded-lg p-4">
                  <div className="flex items-center gap-2 mb-2">
                    {getStatusIcon(!verificationResult.gitStatus.hasUncommittedChanges)}
                    <h3 className="font-medium">Git Status</h3>
                    <Badge
                      variant={
                        !verificationResult.gitStatus.hasUncommittedChanges
                          ? 'default'
                          : 'secondary'
                      }
                    >
                      {verificationResult.gitStatus.hasUncommittedChanges
                        ? 'Changes present'
                        : 'Clean'}
                    </Badge>
                  </div>
                  <div className="text-sm text-muted-foreground space-y-1">
                    <div>Current branch: {verificationResult.gitStatus.currentBranch}</div>
                    <div>Base branch: {verificationResult.gitStatus.baseBranch}</div>
                    <div>Status: {verificationResult.gitStatus.status}</div>
                    {verificationResult.gitStatus.hasUncommittedChanges && (
                      <div className="mt-2 text-xs text-yellow-600">
                        There are uncommitted changes in the workspace
                      </div>
                    )}
                  </div>
                </div>
              )}
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}
