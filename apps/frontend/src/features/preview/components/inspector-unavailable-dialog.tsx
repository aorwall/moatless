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
      </DialogContent>
    </Dialog>
  )
}
