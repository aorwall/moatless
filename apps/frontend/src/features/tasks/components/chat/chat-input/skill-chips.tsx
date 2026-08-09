import { Sparkles, X } from 'lucide-react'
import { Button } from '@/lib/components/ui/button'

interface SkillChipsProps {
  /** Skill names activated for the next message. */
  skills: string[]
  onRemove: (name: string) => void
}

/**
 * The skills picked from the "/" dropdown, shown above the textarea. They ride
 * the message as a field, so the draft text stays as the person wrote it.
 */
export function SkillChips({ skills, onRemove }: SkillChipsProps) {
  if (skills.length === 0) {
    return null
  }

  return (
    <div className="flex flex-wrap gap-2 mb-2">
      {skills.map((name) => (
        <div
          key={name}
          className="flex items-center gap-1.5 px-2 py-1 bg-secondary/10 rounded text-xs group"
        >
          <Sparkles className="h-3 w-3 text-muted-foreground" />
          <span className="max-w-[200px] truncate" title={name}>
            {name}
          </span>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            onClick={() => onRemove(name)}
            className="h-4 w-4 p-0 text-muted-foreground hover:text-foreground transition-colors opacity-0 group-hover:opacity-100"
            aria-label={`Remove skill ${name}`}
          >
            <X className="h-3 w-3" />
          </Button>
        </div>
      ))}
    </div>
  )
}
