interface DetailNotFoundProps {
  title: string
  description?: string
}

/**
 * Centered "not found" state for a detail pane whose entity is missing
 * (e.g. deleted, or an invalid id in the URL).
 */
export function DetailNotFound({ title, description }: DetailNotFoundProps) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-1 text-center text-muted-foreground">
      <p className="font-medium">{title}</p>
      {description ? <p className="text-sm">{description}</p> : null}
    </div>
  )
}
