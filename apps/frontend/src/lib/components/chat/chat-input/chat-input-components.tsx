import type { FileSearchResult } from '@moatless/types'
import { ArrowUp, Circle, Loader2, Paperclip } from 'lucide-react'
import type React from 'react'
import { useRef } from 'react'
import { FileAutocomplete } from '@/lib/components/chat/file-autocomplete'
import { Button } from '@/lib/components/ui/button'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/lib/components/ui/tooltip'

interface SelectorsProps {
  children: React.ReactNode
}

export function Selectors({ children }: SelectorsProps) {
  return <div className="flex items-center gap-2">{children}</div>
}

// Re-export FileAttachments component
export { FileAttachments } from './file-attachments'

interface AutocompleteProps {
  results: FileSearchResult[]
  selectedIndex: number
  onSelect: (file: FileSearchResult) => void
  isLoading: boolean
  error?: Error | null
  position?: 'above' | 'below'
}

export function Autocomplete({
  results,
  selectedIndex,
  onSelect,
  isLoading,
  error,
  position = 'below',
}: AutocompleteProps) {
  return (
    <FileAutocomplete
      results={results}
      selectedIndex={selectedIndex}
      onSelect={onSelect}
      isLoading={isLoading}
      error={error}
      position={position}
    />
  )
}

interface UploadButtonProps {
  onFileSelect: (file: File) => void
  disabled?: boolean
  accept?: string
}

export function UploadButton({
  onFileSelect,
  disabled = false,
  accept = 'image/*,.pdf,.txt,.md,.js,.ts,.jsx,.tsx,.py,.java,.c,.cpp,.h,.hpp,.cs,.rb,.go,.rs,.swift,.kt,.m,.mm,.php,.html,.css,.scss,.json,.xml,.yaml,.yml,.toml,.ini,.conf,.sh,.bash,.zsh,.fish,.ps1,.bat,.cmd',
}: UploadButtonProps) {
  const fileInputRef = useRef<HTMLInputElement>(null)

  const handleClick = () => {
    fileInputRef.current?.click()
  }

  const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (file) {
      onFileSelect(file)
    }
    e.target.value = ''
  }

  return (
    <>
      <input
        ref={fileInputRef}
        type="file"
        onChange={handleChange}
        className="hidden"
        accept={accept}
      />
      <TooltipProvider>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              type="button"
              size="icon"
              variant="ghost"
              className="h-6 w-6"
              onClick={handleClick}
              disabled={disabled}
            >
              <Paperclip className="h-4 w-4" />
              <span className="sr-only">Upload file</span>
            </Button>
          </TooltipTrigger>
          <TooltipContent>
            <p>Attach files to your message</p>
          </TooltipContent>
        </Tooltip>
      </TooltipProvider>
    </>
  )
}

interface StopButtonProps {
  onClick: () => void
  disabled?: boolean
}

export function StopButton({ onClick, disabled = false }: StopButtonProps) {
  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            type="button"
            onClick={onClick}
            disabled={disabled}
            size="sm"
            className="flex items-center justify-center gap-1.5 rounded-full bg-transparent border border-border text-muted-foreground transition-all duration-150 ease-out disabled:cursor-not-allowed disabled:opacity-50 h-6 px-3 text-xs font-medium hover:bg-accent hover:text-accent-foreground hover:border-accent"
          >
            <span>Running</span>
            <Circle className="h-3 w-3 text-muted-foreground" />
          </Button>
        </TooltipTrigger>
        <TooltipContent>
          <p>Click to stop the agent</p>
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  )
}

interface SendButtonProps {
  disabled?: boolean
  loading?: boolean
  buttonText?: string
}

export function SendButton({ disabled = false, loading = false }: SendButtonProps) {
  return (
    <Button
      type="submit"
      disabled={disabled}
      size="icon"
      className="flex size-6 items-center justify-center rounded-full bg-foreground text-background transition-opacity duration-150 ease-out disabled:cursor-not-allowed disabled:opacity-50 h-6 w-6"
    >
      {loading ? <Loader2 className="h-5 w-5 animate-spin" /> : <ArrowUp className="h-5 w-5" />}
      <span className="sr-only">Send</span>
    </Button>
  )
}
