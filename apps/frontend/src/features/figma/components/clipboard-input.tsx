import { AlertCircle, FileJson } from 'lucide-react'
import type React from 'react'
import { useCallback } from 'react'
import { Button } from '@/lib/components/ui/button'
import { Label } from '@/lib/components/ui/label'
import { Textarea } from '@/lib/components/ui/textarea'

interface ClipboardInputProps {
  value: string
  onChange: (value: string) => void
  onParse: () => void
  onClear: () => void
  isLoading?: boolean
  error?: string | null
}

/**
 * Clipboard Input Component
 * Textarea for pasting Figma clipboard data with validation
 */
export function ClipboardInput({
  value,
  onChange,
  onParse,
  onClear,
  isLoading = false,
  error = null,
}: ClipboardInputProps) {
  const handlePaste = useCallback(
    (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
      const clipboardData = e.clipboardData

      if (!clipboardData) {
        return
      }

      // Check if clipboard has HTML content (Figma clipboard format)
      if (clipboardData.types.includes('text/html')) {
        const html = clipboardData.getData('text/html')

        // Check if it's Figma clipboard format
        if (html.includes('data-metadata') && html.includes('data-buffer')) {
          e.preventDefault()
          onChange(html)
          return
        }
      }

      // Allow normal paste behavior for JSON
    },
    [onChange],
  )

  const isFigmaFormat = value.includes('data-metadata') && value.includes('data-buffer')
  const isJsonFormat = value.trim().startsWith('{') || value.trim().startsWith('[')

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-2">
        <Label htmlFor="clipboard-input" className="text-sm font-medium">
          Figma Clipboard Data
        </Label>
        <Textarea
          id="clipboard-input"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onPaste={handlePaste}
          placeholder="Paste Figma clipboard HTML or JSON data here..."
          className="min-h-[300px] font-mono text-xs"
          disabled={isLoading}
        />
        {value && (
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <FileJson className="h-4 w-4" />
            {isFigmaFormat && 'Figma clipboard HTML format detected'}
            {isJsonFormat && !isFigmaFormat && 'JSON format detected'}
            {!(isFigmaFormat || isJsonFormat) && 'Unknown format'}
          </div>
        )}
        {error && (
          <div className="flex items-start gap-2 rounded-md bg-destructive/10 p-3 text-sm text-destructive">
            <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
            <div>
              <p className="font-medium">Parse Error</p>
              <p className="mt-1 text-xs">{error}</p>
            </div>
          </div>
        )}
      </div>

      <div className="flex gap-2">
        <Button onClick={onParse} disabled={!value || isLoading} className="flex-1">
          {isLoading ? 'Parsing...' : 'Parse'}
        </Button>
        <Button onClick={onClear} variant="outline" disabled={!value || isLoading}>
          Clear
        </Button>
      </div>

      <div className="rounded-md bg-muted/50 p-3 text-xs text-muted-foreground">
        <p className="font-medium">How to use:</p>
        <ol className="mt-2 ml-4 list-decimal space-y-1">
          <li>Copy component from Figma (Cmd/Ctrl + C)</li>
          <li>Paste HTML or JSON clipboard data into textarea above</li>
          <li>Configure transformer options in the panel below</li>
          <li>Click "Parse" to see the generated prompt and component tree</li>
        </ol>
        <p className="mt-2 text-xs text-emerald-600">
          ✓ Both HTML clipboard (from Figma) and raw JSON formats are supported
        </p>
      </div>
    </div>
  )
}
