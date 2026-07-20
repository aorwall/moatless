import { cn } from '@/lib/utils'

interface ListAvatarProps {
  /** Image URL; falls back to initials derived from `label` when absent. */
  src?: string | null | undefined
  /** Used to derive the initials fallback (and as a stable identity for the row). */
  label: string
  /** `sm` (h-8) for list rows, `lg` (h-14) for detail headers. */
  size?: 'sm' | 'lg'
  className?: string
}

function initials(label: string): string {
  return label.trim().slice(0, 2).toUpperCase()
}

/**
 * Circular avatar showing an image when available, otherwise initials.
 * Shared by list sidebars (sm) and detail headers (lg).
 */
export function ListAvatar({ src, label, size = 'sm', className }: ListAvatarProps) {
  return (
    <span
      className={cn(
        'flex shrink-0 items-center justify-center overflow-hidden rounded-full bg-muted font-medium text-muted-foreground',
        size === 'sm' ? 'h-8 w-8 text-xs' : 'h-14 w-14 text-lg',
        className,
      )}
    >
      {src ? <img src={src} alt="" className="h-full w-full object-cover" /> : initials(label)}
    </span>
  )
}
