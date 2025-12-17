import type { ComponentTree } from '@moatless/figma'
import { Check, Copy, FileText, Search } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'
import { JsonViewer } from '@/lib/components/json-viewer'
import { Button } from '@/lib/components/ui/button'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/lib/components/ui/resizable'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/lib/components/ui/tabs'
import { CanvasSearch } from './canvas-search'

interface ClipboardTestOutputProps {
  prompt: string
  componentTree: ComponentTree
  rawClipboardJson: unknown
  page1Canvas: unknown | null
  internalCanvas: unknown | null
}

/**
 * Clipboard Test Output Component
 * Tabbed view showing generated prompt and component tree JSON
 */
export function ClipboardTestOutput({
  prompt,
  componentTree,
  page1Canvas,
  internalCanvas,
}: ClipboardTestOutputProps) {
  const [copiedPrompt, setCopiedPrompt] = useState(false)
  const [externalSearchQuery, setExternalSearchQuery] = useState<string>('')

  const handleCopyPrompt = async () => {
    try {
      await navigator.clipboard.writeText(prompt)
      setCopiedPrompt(true)
      toast.success('Prompt copied to clipboard')
      setTimeout(() => setCopiedPrompt(false), 2000)
    } catch (_err) {
      toast.error('Failed to copy prompt')
    }
  }

  const handlePrimitiveClick = (value: string | number) => {
    const searchQuery = String(value)
    setExternalSearchQuery(searchQuery)
  }

  return (
    <Tabs defaultValue="prompt" className="h-full w-full">
      <TabsList className="w-full">
        <TabsTrigger value="prompt" className="flex-1">
          <FileText className="mr-2 h-4 w-4" />
          Generated Prompt
        </TabsTrigger>
        <TabsTrigger value="component-tree" className="flex-1">
          Component Tree
        </TabsTrigger>
        <TabsTrigger value="page1" className="flex-1">
          Page 1
        </TabsTrigger>
        <TabsTrigger value="internal-canvas" className="flex-1">
          Internal Canvas
        </TabsTrigger>
        <TabsTrigger value="search" className="flex-1">
          <Search className="mr-2 h-4 w-4" />
          Search
        </TabsTrigger>
      </TabsList>

      <TabsContent value="prompt" className="mt-4 h-[calc(100%-3rem)]">
        <div className="relative h-full">
          <div className="absolute right-2 top-2 z-10">
            <Button size="sm" variant="outline" onClick={handleCopyPrompt} className="h-8 gap-2">
              {copiedPrompt ? (
                <>
                  <Check className="h-3 w-3" />
                  Copied
                </>
              ) : (
                <>
                  <Copy className="h-3 w-3" />
                  Copy Prompt
                </>
              )}
            </Button>
          </div>
          <div className="h-full overflow-auto rounded-md border bg-muted/50 p-4">
            <pre className="whitespace-pre-wrap font-mono text-xs">{prompt}</pre>
          </div>
        </div>
      </TabsContent>

      <TabsContent value="component-tree" className="mt-4 h-[calc(100%-3rem)]">
        <div className="h-full overflow-auto">
          <JsonViewer data={componentTree} />
        </div>
      </TabsContent>

      <TabsContent value="page1" className="mt-4 h-[calc(100%-3rem)]">
        <div className="h-full overflow-auto">
          {page1Canvas ? (
            <JsonViewer data={page1Canvas} />
          ) : (
            <div className="flex h-full items-center justify-center rounded-md border border-dashed p-8">
              <p className="text-sm text-muted-foreground">
                No "Page 1" canvas found in clipboard data
              </p>
            </div>
          )}
        </div>
      </TabsContent>

      <TabsContent value="internal-canvas" className="mt-4 h-[calc(100%-3rem)]">
        <div className="h-full overflow-auto">
          {internalCanvas ? (
            <JsonViewer data={internalCanvas} />
          ) : (
            <div className="flex h-full items-center justify-center rounded-md border border-dashed p-8">
              <p className="text-sm text-muted-foreground">
                No "Internal Only Canvas" found in clipboard data
              </p>
            </div>
          )}
        </div>
      </TabsContent>

      <TabsContent value="search" className="mt-4 h-[calc(100%-3rem)]">
        {internalCanvas ? (
          <ResizablePanelGroup direction="horizontal" className="h-full">
            {/* Left: Page 1 Canvas (Read-only) */}
            <ResizablePanel defaultSize={50} minSize={30}>
              <div className="flex h-full flex-col">
                <div className="border-b bg-muted/30 px-4 py-2">
                  <h3 className="text-sm font-medium">Page 1 Canvas</h3>
                </div>
                <div className="flex-1 overflow-auto p-4">
                  {page1Canvas ? (
                    <JsonViewer
                      data={page1Canvas}
                      enableClickableValues={true}
                      onPrimitiveClick={handlePrimitiveClick}
                    />
                  ) : (
                    <div className="flex h-full items-center justify-center rounded-md border border-dashed p-8">
                      <p className="text-sm text-muted-foreground">No "Page 1" canvas found</p>
                    </div>
                  )}
                </div>
              </div>
            </ResizablePanel>

            <ResizableHandle withHandle />

            {/* Right: Search Interface */}
            <ResizablePanel defaultSize={50} minSize={30}>
              <CanvasSearch data={internalCanvas} externalSearchQuery={externalSearchQuery} />
            </ResizablePanel>
          </ResizablePanelGroup>
        ) : (
          <div className="flex h-full items-center justify-center rounded-md border border-dashed p-8">
            <p className="text-sm text-muted-foreground">
              No "Internal Only Canvas" found - cannot search
            </p>
          </div>
        )}
      </TabsContent>
    </Tabs>
  )
}

/**
 * Empty State Component
 * Shown when no data has been parsed yet
 */
export function ClipboardTestOutputEmpty() {
  return (
    <div className="flex h-full items-center justify-center rounded-md border border-dashed p-12 text-center">
      <div className="flex flex-col items-center gap-2">
        <FileText className="h-12 w-12 text-muted-foreground/50" />
        <p className="text-sm font-medium text-muted-foreground">No data parsed yet</p>
        <p className="text-xs text-muted-foreground/70">
          Paste Figma clipboard data and click "Parse" to see output
        </p>
      </div>
    </div>
  )
}
