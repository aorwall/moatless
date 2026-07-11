import { File, X } from 'lucide-react'
import { useState } from 'react'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/lib/components/ui/dialog'

interface FileAttachmentsProps {
  files: Array<{
    fileId: string
    filename: string
    mimeType: string
  }>
  onRemove?: ((fileId: string) => void) | undefined
}

export function FileAttachments({ files, onRemove }: FileAttachmentsProps) {
  const [selectedImage, setSelectedImage] = useState<{ url: string; filename: string } | null>(null)

  if (files.length === 0) {
    return null
  }

  // Separate images from other files
  const images = files.filter((file) => file.mimeType.startsWith('image/'))
  const otherFiles = files.filter((file) => !file.mimeType.startsWith('image/'))

  return (
    <div className="flex flex-col gap-2 mb-2">
      {/* Image thumbnails */}
      {images.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {images.map((file) => {
            const imageUrl = `/api/v1/uploads/${file.fileId}`

            return (
              <div key={file.fileId} className="relative group w-fit">
                <button
                  type="button"
                  onClick={() => setSelectedImage({ url: imageUrl, filename: file.filename })}
                  className="block"
                >
                  <img
                    src={imageUrl}
                    alt={file.filename}
                    className="h-12 w-12 object-cover rounded border-1 border-border hover:border-foreground/50 transition-colors cursor-pointer"
                  />
                </button>
                {onRemove && (
                  <button
                    type="button"
                    onClick={() => onRemove(file.fileId)}
                    className="absolute top-1 right-1 bg-background/80 hover:bg-background rounded-full p-1 opacity-0 group-hover:opacity-100 transition-opacity"
                    aria-label="Remove image"
                  >
                    <X className="h-3 w-3" />
                  </button>
                )}
              </div>
            )
          })}
        </div>
      )}

      {/* Non-image files */}
      {otherFiles.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {otherFiles.map((file) => (
            <div
              key={file.fileId}
              className="flex items-center gap-1.5 px-2 py-1 bg-secondary/10 rounded text-xs"
            >
              <File className="h-3 w-3 text-muted-foreground" />
              <span className="max-w-[150px] truncate">{file.filename}</span>
              {onRemove && (
                <button
                  type="button"
                  onClick={() => onRemove(file.fileId)}
                  className="text-muted-foreground hover:text-foreground transition-colors"
                  aria-label="Remove file"
                >
                  <X className="h-3 w-3" />
                </button>
              )}
            </div>
          ))}
        </div>
      )}

      {/* Full-size image dialog */}
      <Dialog
        open={selectedImage !== null}
        onOpenChange={(open) => !open && setSelectedImage(null)}
      >
        <DialogContent className="max-h-[85vh] w-[90vw] max-w-4xl sm:max-w-4xl p-6">
          {selectedImage && (
            <>
              <DialogHeader>
                <DialogTitle className="truncate">{selectedImage.filename}</DialogTitle>
                <DialogDescription>Image</DialogDescription>
              </DialogHeader>
              <div className="h-[calc(85vh-8rem)] flex items-center justify-center">
                <img
                  src={selectedImage.url}
                  alt={selectedImage.filename}
                  className="max-h-full max-w-full rounded-lg object-contain"
                />
              </div>
            </>
          )}
        </DialogContent>
      </Dialog>
    </div>
  )
}
