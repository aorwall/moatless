import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/lib/components/ui/dialog'

export interface TaskDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  screenshotUrl: string
  alt?: string
  taskTitle: string
  repositoryName: string
  timestamp: string
}

export function TaskDialog({
  open,
  onOpenChange,
  screenshotUrl,
  alt = 'Full screenshot',
  taskTitle,
  repositoryName,
  timestamp,
}: TaskDialogProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-[min(1100px,90vw)] max-h-[90vh] p-6 sm:max-w-[min(1100px,90vw)]">
        <DialogHeader>
          <DialogTitle className="text-left">{taskTitle}</DialogTitle>
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <span>{repositoryName}</span>
            <span>•</span>
            <span>{timestamp}</span>
          </div>
        </DialogHeader>
        <div className="overflow-auto max-h-[calc(90vh-8rem)]">
          <img src={screenshotUrl} alt={alt} className="max-w-full h-auto" loading="lazy" />
        </div>
      </DialogContent>
    </Dialog>
  )
}
