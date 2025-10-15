import { Check, Copy } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/lib/components/ui/button'

interface FigmaNodePromptProps {
  prompt: string
}

export function FigmaNodePrompt({ prompt }: FigmaNodePromptProps) {
  const [copied, setCopied] = useState(false)

  const handleCopy = async () => {
    await navigator.clipboard.writeText(prompt)
    setCopied(true)
    setTimeout(() => setCopied(false), 2000)
  }

  return (
    <div className="flex flex-col h-full min-h-0">
      <div className="flex items-center justify-between p-3 border-b flex-shrink-0">
        <div>
          <h3 className="text-sm font-semibold">AI Prompt Context</h3>
          <p className="text-xs text-muted-foreground mt-0.5">
            This is the exact text sent to the AI agent
          </p>
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={handleCopy}
          className="flex items-center gap-2"
        >
          {copied ? (
            <>
              <Check className="h-4 w-4" />
              Copied
            </>
          ) : (
            <>
              <Copy className="h-4 w-4" />
              Copy
            </>
          )}
        </Button>
      </div>
      <div className="flex-1 overflow-auto p-4 min-h-0">
        <pre className="text-xs font-mono whitespace-pre-wrap break-words">{prompt}</pre>
      </div>
    </div>
  )
}
