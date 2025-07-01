import { Code, Eye } from 'lucide-react'
import { Tabs, TabsList, TabsTrigger } from '@/lib/components/ui/tabs'

export type PreviewMode = 'code' | 'preview'

interface PreviewModeToggleProps {
  mode: PreviewMode
  onModeChange: (mode: PreviewMode) => void
}

export function PreviewModeToggle({ mode, onModeChange }: PreviewModeToggleProps) {
  return (
    <Tabs value={mode} onValueChange={(value) => onModeChange(value as PreviewMode)}>
      <TabsList className="h-8">
        <TabsTrigger value="code" className="h-7 px-2 text-xs">
          <Code className="h-3 w-3 mr-1" />
          Code
        </TabsTrigger>
        <TabsTrigger value="preview" className="h-7 px-2 text-xs">
          <Eye className="h-3 w-3 mr-1" />
          Preview
        </TabsTrigger>
      </TabsList>
    </Tabs>
  )
}