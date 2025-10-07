import { Loader2 } from 'lucide-react'
import { Button } from '@/lib/components/ui/button'

interface PublishButtonProps {
  vibe: boolean | undefined
  siteUrl: string | undefined
  onPublish: () => void
  isPending: boolean
}

export function PublishButton({ vibe, siteUrl, onPublish, isPending }: PublishButtonProps) {
  if (!vibe) {
    return null
  }

  if (siteUrl) {
    return (
      <Button variant="outline" size="sm" asChild className="h-8 bg-white hover:bg-gray-50">
        <a href={siteUrl} target="_blank" rel="noopener noreferrer">
          Published
        </a>
      </Button>
    )
  }

  return (
    <Button
      variant="outline"
      size="sm"
      onClick={onPublish}
      disabled={isPending}
      className="h-8 bg-white hover:bg-gray-50"
    >
      {isPending ? (
        <>
          <Loader2 className="mr-2 h-4 w-4 animate-spin" />
          Publishing...
        </>
      ) : (
        'Publish'
      )}
    </Button>
  )
}
