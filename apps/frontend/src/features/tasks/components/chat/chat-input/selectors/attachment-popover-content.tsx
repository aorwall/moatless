import { Paperclip } from 'lucide-react'
import type React from 'react'
import { useRef } from 'react'
import { Button } from '@/lib/components/ui/button.tsx'

interface AttachmentPopoverContentProps {
  onFileSelect: (file: File) => void
  disabled?: boolean
}

export function AttachmentPopoverContent({
  onFileSelect,
  disabled = false,
}: AttachmentPopoverContentProps) {
  const fileInputRef = useRef<HTMLInputElement>(null)

  const handleUploadClick = () => {
    fileInputRef.current?.click()
  }

  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (file) {
      onFileSelect(file)
    }
    // Reset input so the same file can be selected again
    e.target.value = ''
  }

  return (
    <>
      <input
        ref={fileInputRef}
        type="file"
        onChange={handleFileChange}
        className="hidden"
        accept="image/*,.pdf,.txt,.md,.js,.ts,.jsx,.tsx,.py,.java,.c,.cpp,.h,.hpp,.cs,.rb,.go,.rs,.swift,.kt,.m,.mm,.php,.html,.css,.scss,.json,.xml,.yaml,.yml,.toml,.ini,.conf,.sh,.bash,.zsh,.fish,.ps1,.bat,.cmd"
      />
      <div className="p-1 flex flex-col w-fit">
        <Button
          variant="ghost"
          className="w-full justify-start h-auto px-3 py-2 text-left gap-2"
          onClick={handleUploadClick}
          disabled={disabled}
        >
          <Paperclip className="h-4 w-4 shrink-0" />
          <span className="font-medium whitespace-nowrap">Upload File</span>
        </Button>
        {/* Future options can be added here:
          - Import from URL
          - etc.
        */}
      </div>
    </>
  )
}
