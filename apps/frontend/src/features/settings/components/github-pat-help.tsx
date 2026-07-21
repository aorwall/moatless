import { ExternalLink } from 'lucide-react'

import { Alert, AlertDescription, AlertTitle } from '@/lib/components/ui/alert'

interface GitHubPatHelpProps {
  /** Scope guidance — differs between a personal and an org-wide token. */
  scopeHint?: string
}

export function GitHubPatHelp({
  scopeHint = 'Select the "repo" scope for repository access',
}: GitHubPatHelpProps) {
  return (
    <Alert>
      <AlertTitle>How to create a Personal Access Token</AlertTitle>
      <AlertDescription className="space-y-2">
        <ol className="list-decimal list-inside space-y-1 text-sm">
          <li>
            Go to{' '}
            <a
              href="https://github.com/settings/tokens/new"
              target="_blank"
              rel="noopener noreferrer"
              className="text-blue-600 hover:underline inline-flex items-center gap-1"
            >
              GitHub Settings → Personal access tokens
              <ExternalLink className="h-3 w-3" />
            </a>
          </li>
          <li>Click "Generate new token (classic)"</li>
          <li>Give your token a descriptive name</li>
          <li>{scopeHint}</li>
          <li>Click "Generate token" and copy it</li>
        </ol>
      </AlertDescription>
    </Alert>
  )
}
