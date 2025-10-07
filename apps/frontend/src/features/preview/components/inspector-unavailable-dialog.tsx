import { ExternalLink } from 'lucide-react'
import { Button } from '@/lib/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/lib/components/ui/dialog'

interface InspectorUnavailableDialogProps {
  isOpen: boolean
  onOpenChange: (open: boolean) => void
}

export function InspectorUnavailableDialog({
  isOpen,
  onOpenChange,
}: InspectorUnavailableDialogProps) {
  return (
    <Dialog open={isOpen} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Element Inspector Unavailable</DialogTitle>
          <DialogDescription>
            The element inspector requires code inspector attributes in your components.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <p className="text-sm text-muted-foreground">
            Install the <strong>React Inspector Plugin</strong> to enable this feature. This plugin
            adds <code className="text-xs bg-muted px-1 py-0.5 rounded">data-insp-path</code>{' '}
            attributes to your React components, allowing the inspector to identify and navigate to
            source code.
          </p>
          <Button
            variant="outline"
            className="w-full"
            onClick={() => {
              window.open('https://inspector.fe-dev.cn/en/', '_blank', 'noopener,noreferrer')
            }}
          >
            <ExternalLink className="mr-2 h-4 w-4" />
            Learn more about React Inspector Plugin
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}
