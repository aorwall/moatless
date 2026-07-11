import { useState } from 'react'

interface ImageThumbnailProps {
  data: string // Base64 encoded image data
  mediaType: string
  alt?: string
  size?: 'small' | 'expanded'
}

export function ImageThumbnail({
  data,
  mediaType,
  alt = 'Tool result image',
  size = 'small',
}: ImageThumbnailProps) {
  const [hasError, setHasError] = useState(false)

  const sizeClasses = {
    small: 'w-16 h-16',
    expanded: 'max-w-full max-h-96',
  }

  if (hasError) {
    return (
      <div
        className={`${sizeClasses[size]} bg-muted/30 rounded border border-muted-foreground/20 flex items-center justify-center`}
      >
        <span className="text-xs text-muted-foreground">Invalid image</span>
      </div>
    )
  }

  const src = `data:${mediaType};base64,${data}`

  return (
    <img
      src={src}
      alt={alt}
      className={`${sizeClasses[size]} object-contain rounded border border-muted-foreground/20 bg-muted/10`}
      onError={() => setHasError(true)}
    />
  )
}
