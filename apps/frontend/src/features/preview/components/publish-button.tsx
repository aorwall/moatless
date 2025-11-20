import { ExternalLink, Loader2 } from 'lucide-react'
import { Button } from '@/lib/components/ui/button'
import { Popover, PopoverContent, PopoverTrigger } from '@/lib/components/ui/popover'

interface PublishButtonProps {
  canPublish: boolean | undefined
  siteUrl: string | undefined
  onPublish: () => void
  isPending: boolean
}

export function PublishButton({ canPublish, siteUrl, onPublish, isPending }: PublishButtonProps) {
  if (!canPublish) {
    return null
  }

  if (siteUrl) {
    return (
      <Popover>
        <PopoverTrigger asChild>
          <Button variant="outline" size="sm" className="h-8 bg-white hover:bg-gray-50">
            Published
          </Button>
        </PopoverTrigger>
        <PopoverContent className="w-64 p-3" align="end">
          <div className="flex flex-col gap-3">
            <a
              href={siteUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="flex items-center gap-2 text-sm text-primary hover:underline"
            >
              <ExternalLink className="h-4 w-4" />
              View published site
            </a>
            <Button
              variant="outline"
              size="sm"
              onClick={onPublish}
              disabled={isPending}
              className="w-full"
            >
              {isPending ? (
                <>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Publishing...
                </>
              ) : (
                'Publish Again'
              )}
            </Button>
          </div>
        </PopoverContent>
      </Popover>
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
