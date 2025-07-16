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
                  verificationResult.status === 'idle' &&
                    verificationResult.environment.status === 'ready',
                )}
                <span className="font-medium">Overall Status</span>
                {getStatusBadge(
                  verificationResult.status === 'idle' &&
                    verificationResult.environment.status === 'ready',
                )}
              </div>

              {/* Workspace Status */}
              <div className="border rounded-lg p-4">
                <div className="flex items-center gap-2 mb-2">
                  {getStatusIcon(verificationResult.status === 'idle')}
                  <h3 className="font-medium">Workspace Status</h3>
                  <Badge variant={verificationResult.status === 'idle' ? 'default' : 'destructive'}>
                    {verificationResult.status}
                  </Badge>
                </div>
                <p className="text-sm text-muted-foreground mb-2">
                  Background tasks: {verificationResult.backgroundTaskCount}
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
                    {verificationResult.environment.capabilities.fileOperations
                      ? 'Enabled'
                      : 'Disabled'}
                  </div>
                  <div>
                    Command Execution:{' '}
                    {verificationResult.environment.capabilities.commandExecution
                      ? 'Enabled'
                      : 'Disabled'}
                  </div>
                </div>
              </div>

              {/* Git Changes */}
              {verificationResult.gitChanges && (
                <div className="border rounded-lg p-4">
                  <div className="flex items-center gap-2 mb-2">
                    {getStatusIcon(verificationResult.gitChanges.totalChanges === 0)}
                    <h3 className="font-medium">Git Changes</h3>
                    <Badge
                      variant={
                        verificationResult.gitChanges.totalChanges === 0 ? 'default' : 'secondary'
                      }
                    >
                      {verificationResult.gitChanges.totalChanges} changes
                    </Badge>
                  </div>
                  <div className="text-sm text-muted-foreground space-y-1">
                    <div>Additions: {verificationResult.gitChanges.totalAdditions}</div>
                    <div>Deletions: {verificationResult.gitChanges.totalDeletions}</div>
                    <div>Files: {verificationResult.gitChanges.files.length}</div>
                    {verificationResult.gitChanges.files.length > 0 && (
                      <div className="mt-2">
                        <p className="text-xs mb-1">Changed files:</p>
                        <div className="text-xs font-mono space-y-1">
                          {verificationResult.gitChanges.files.map((file) => (
                            <div key={file}>{file}</div>
                          ))}
                        </div>
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
