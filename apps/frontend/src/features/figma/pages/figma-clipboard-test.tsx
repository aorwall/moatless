import { componentTreeToPrompt, type TransformOptions } from '@moatless/figma'
import { ChevronLeft, ChevronRight } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/lib/components/ui/button'
import { ClipboardInput } from '../components/clipboard-input'
import { ClipboardTestOutput, ClipboardTestOutputEmpty } from '../components/clipboard-test-output'
import { TransformOptionsPanel } from '../components/transform-options-panel'
import { useFigmaTestParser } from '../hooks/use-figma-test-parser'

/**
 * Figma Clipboard Parser Test Page
 * Playground for testing the Figma clipboard parser with configurable options
 */
export function FigmaClipboardTestPage() {
  const [clipboardInput, setClipboardInput] = useState('')
  const [isLeftPanelCollapsed, setIsLeftPanelCollapsed] = useState(false)
  const [transformOptions, setTransformOptions] = useState<TransformOptions>({
    expandInstances: true,
    applyInstanceOverrides: true,
    applyComponentProps: true,
    filterHidden: true,
    detectStructuralFrames: true,
    skipIconTailwind: true,
    sortChildrenByPosition: true,
    includePositionCoords: false,
    flattenStructuralWrappers: true,
    useFlexibleDimensions: true,
    includeNodeStyle: true,
    includeTextStyle: true,
    includeLayoutDimensions: true,
    includeMetadata: false,
  })

  const { parse, result, error, isLoading, clear } = useFigmaTestParser()

  const handleParse = () => {
    parse(clipboardInput, transformOptions)
  }

  const handleClear = () => {
    setClipboardInput('')
    clear()
  }

  // Generate prompt from parsed result
  const generatedPrompt = result
    ? componentTreeToPrompt(result.componentTree, result.fileName, result.nodeName)
    : null

  return (
    <div className="flex h-screen flex-col">
      {/* Header */}
      <div className="border-b bg-background px-6 py-4">
        <h1 className="text-2xl font-bold">Figma Clipboard Parser Test & Playground</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Test the Figma clipboard parser with configurable transformer options
        </p>
      </div>

      {/* Main Content */}
      <div className="flex flex-1 overflow-hidden">
        {/* Left Panel: Input and Options */}
        {!isLeftPanelCollapsed && (
          <div className="w-2/5 overflow-auto border-r bg-background p-6">
            <div className="space-y-6">
              <ClipboardInput
                value={clipboardInput}
                onChange={setClipboardInput}
                onParse={handleParse}
                onClear={handleClear}
                isLoading={isLoading}
                error={error}
              />

              <div className="border-t pt-6">
                <TransformOptionsPanel options={transformOptions} onChange={setTransformOptions} />
              </div>
            </div>
          </div>
        )}

        {/* Collapse/Expand Toggle Button */}
        <div className="relative">
          <Button
            variant="ghost"
            size="icon"
            className="absolute left-0 top-4 z-10 h-8 w-8 rounded-r-md border border-l-0 bg-background shadow-sm hover:bg-muted"
            onClick={() => setIsLeftPanelCollapsed(!isLeftPanelCollapsed)}
          >
            {isLeftPanelCollapsed ? (
              <ChevronRight className="h-4 w-4" />
            ) : (
              <ChevronLeft className="h-4 w-4" />
            )}
          </Button>
        </div>

        {/* Right Panel: Output */}
        <div className="flex-1 overflow-auto bg-muted/30 p-6">
          {generatedPrompt && result ? (
            <ClipboardTestOutput
              prompt={generatedPrompt}
              componentTree={result.componentTree}
              rawClipboardJson={result.rawClipboardJson}
              page1Canvas={result.page1Canvas}
              internalCanvas={result.internalCanvas}
            />
          ) : (
            <ClipboardTestOutputEmpty />
          )}
        </div>
      </div>
    </div>
  )
}
