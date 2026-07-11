import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/lib/components/ui/dialog'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/lib/components/ui/tabs'
import { figmaApi } from '../api/figma'
import { useFigmaImportQuery } from '../hooks/use-figma-import'
import { FigmaNodePrompt } from './figma-node-prompt'

interface FigmaImportViewerProps {
  importId: string | null
  onClose: () => void
}

/**
 * Dialog component for viewing Figma import details with three tabs:
 * - Preview: Shows the design image
 * - Prompt: Shows the formatted text sent to AI
 * - Design Details: Shows visual breakdown of design properties
 */
export function FigmaImportViewer({ importId, onClose }: FigmaImportViewerProps) {
  // Fetch full Figma import metadata when dialog is open
  const { data: figmaImport, isLoading: isFigmaLoading } = useFigmaImportQuery(importId)

  return (
    <Dialog open={importId !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-h-[85vh] w-[90vw] max-w-5xl sm:max-w-5xl p-0 flex flex-col">
        {importId && figmaImport && (
          <>
            <DialogHeader className="p-6 pb-0">
              <DialogTitle className="truncate">
                {figmaImport.fileName} - {figmaImport.nodeName}
              </DialogTitle>
              <DialogDescription>Figma Import</DialogDescription>
            </DialogHeader>
            <Tabs
              defaultValue={figmaImport.hasImage ? 'preview' : 'prompt'}
              className="flex flex-col flex-1 min-h-0 mt-4"
            >
              <TabsList className="mx-6 flex-shrink-0">
                {figmaImport.hasImage && <TabsTrigger value="preview">Preview</TabsTrigger>}
                <TabsTrigger value="prompt">Prompt</TabsTrigger>
              </TabsList>
              {figmaImport.hasImage && (
                <TabsContent
                  value="preview"
                  className="flex-1 px-6 pb-6 min-h-0 data-[state=active]:flex data-[state=active]:flex-col"
                >
                  <div className="flex-1 flex items-center justify-center min-h-0">
                    <img
                      src={figmaApi.getImageUrl(importId)}
                      alt={figmaImport.nodeName ?? undefined}
                      className="max-h-full max-w-full rounded-lg object-contain"
                    />
                  </div>
                </TabsContent>
              )}
              <TabsContent
                value="prompt"
                className="flex-1 min-h-0 data-[state=active]:flex data-[state=active]:flex-col"
              >
                <FigmaNodePrompt prompt={figmaImport.prompt || ''} />
              </TabsContent>
            </Tabs>
          </>
        )}
        {importId && isFigmaLoading && (
          <div className="p-6">
            <div className="flex items-center justify-center h-64">
              <div className="text-muted-foreground">Loading Figma data...</div>
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}
