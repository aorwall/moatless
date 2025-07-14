import type { AssistantMessage, AssistantMessageContent } from '@moatless/types'
import { Terminal, ChevronRight, ChevronDown } from 'lucide-react'
import { Button } from '@/lib/components/ui/button'
import { useState } from 'react'

interface AssistantMessageProps {
  message: AssistantMessage
  expandedToolCalls: Set<string>
  onToggleToolCall: (toolUseId: string) => void
}

interface ToolCallProps {
  content: AssistantMessageContent
  expanded: boolean
  onToggle: () => void
}

function ToolCallComponent({ content, expanded, onToggle }: ToolCallProps) {
  const hasResult = !!content.tool_result
  const resultLines = content.tool_result?.content?.split('\n') || []
  const shouldTruncate = resultLines.length > 3
  const displayLines = expanded ? resultLines : resultLines.slice(0, 3)
  const hiddenLineCount = resultLines.length - 3
  
  // Format tool input for display
  const formatToolInput = () => {
    if (!content.tool_input) return ''
    if (typeof content.tool_input === 'string') return content.tool_input
    
    const entries = Object.entries(content.tool_input)
    if (entries.length === 0) return ''
    
    // Show only first parameter in compact view
    const [_key, value] = entries[0]
    const displayValue = typeof value === 'string' ? value : JSON.stringify(value)
    const truncatedValue = displayValue.length > 50 ? `${displayValue.substring(0, 50)}...` : displayValue
    
    return entries.length > 1 
      ? `${truncatedValue} +${entries.length - 1} more`
      : truncatedValue
  }

  return (
    <div className="text-xs font-mono">
      <div className="flex items-start gap-2">
        <div className="w-1.5 h-1.5 bg-blue-500 rounded-full mt-1 flex-shrink-0" />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1">
            <span className="font-medium">{content.tool_name}</span>
            {content.tool_input && (
              <span className="text-muted-foreground truncate">
                ({formatToolInput()})
              </span>
            )}
          </div>

          {hasResult && (
            <div className="mt-1 ml-2 pl-2 border-l-2 border-border/50">
              <div className="bg-muted/30 rounded p-1.5 text-muted-foreground">
                {displayLines.map((line, index) => (
                  <div key={`${content.tool_use_id}-${index}`} className="whitespace-pre-wrap break-all">
                    {line || ' '}
                  </div>
                ))}

                {shouldTruncate && (
                  <Button
                    variant="link"
                    size="sm"
                    onClick={onToggle}
                    className="h-auto p-0 text-xs text-muted-foreground hover:text-foreground"
                  >
                    {expanded ? '← collapse' : `… +${hiddenLineCount} lines →`}
                  </Button>
                )}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

interface ToolCallSummaryProps {
  toolCalls: AssistantMessageContent[]
  expanded: boolean
  onToggle: () => void
}

function ToolCallSummary({ toolCalls, expanded, onToggle }: ToolCallSummaryProps) {
  // Group tool calls by name
  const toolCallCounts = toolCalls.reduce((acc, tool) => {
    const name = tool.tool_name || 'Unknown'
    acc[name] = (acc[name] || 0) + 1
    return acc
  }, {} as Record<string, number>)

  const totalCalls = toolCalls.length
  
  // Format as "5 tool calls (2 Read, 3 Bash)"
  const toolSummary = Object.entries(toolCallCounts)
    .sort(([, a], [, b]) => b - a) // Sort by count descending
    .map(([name, count]) => count > 1 ? `${count} ${name}` : name)
    .join(', ')

  return (
    <Button
      variant="ghost"
      size="sm"
      onClick={onToggle}
      className="w-full justify-start p-2 h-auto font-mono text-xs hover:bg-muted/50"
    >
      <div className="flex items-center gap-2 w-full">
        {expanded ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
        <Terminal className="h-3 w-3 text-muted-foreground" />
        <span className="text-muted-foreground">
          {totalCalls} tool {totalCalls === 1 ? 'call' : 'calls'} ({toolSummary})
        </span>
        <span className="ml-auto text-muted-foreground">
          {expanded ? 'collapse' : 'expand'}
        </span>
      </div>
    </Button>
  )
}

export function AssistantMessageComponent({
  message,
  expandedToolCalls,
  onToggleToolCall,
}: AssistantMessageProps) {
  const [toolCallsExpanded, setToolCallsExpanded] = useState(false)
  const isStringContent = typeof message.content === 'string'
  const contentArray = isStringContent ? [] : (message.content as AssistantMessageContent[])

  // Group content into sections (text sections and tool call groups)
  const contentSections: Array<{ type: 'text' | 'tools'; content: string | AssistantMessageContent[] }> = []
  let currentText = ''
  let currentTools: AssistantMessageContent[] = []

  contentArray.forEach((item) => {
    if (item.type === 'text') {
      // If we have accumulated tools, push them first
      if (currentTools.length > 0) {
        contentSections.push({ type: 'tools', content: currentTools })
        currentTools = []
      }
      currentText += (currentText ? '\n' : '') + item.text
    } else if (item.type === 'tool_use') {
      // If we have accumulated text, push it first
      if (currentText) {
        contentSections.push({ type: 'text', content: currentText })
        currentText = ''
      }
      currentTools.push(item)
    }
  })

  // Push any remaining content
  if (currentText) {
    contentSections.push({ type: 'text', content: currentText })
  }
  if (currentTools.length > 0) {
    contentSections.push({ type: 'tools', content: currentTools })
  }

  // Handle string content
  if (isStringContent) {
    contentSections.push({ type: 'text', content: message.content as string })
  }

  const handleToggleAllTools = () => {
    setToolCallsExpanded(!toolCallsExpanded)
  }

  return (
    <div className="space-y-2">
      {contentSections.map((section, index) => {
        if (section.type === 'text') {
          return (
            <div key={`${message.id}-text-${index}`} className="text-sm whitespace-pre-wrap break-words">
              {section.content as string}
            </div>
          )
        } else {
          const toolCalls = section.content as AssistantMessageContent[]
          return (
            <div key={`${message.id}-tools-${index}`} className="border border-border/50 rounded-lg overflow-hidden">
              <ToolCallSummary
                toolCalls={toolCalls}
                expanded={toolCallsExpanded}
                onToggle={handleToggleAllTools}
              />
              
              {toolCallsExpanded && (
                <div className="p-2 space-y-2 border-t border-border/50">
                  {toolCalls.map((toolCall) => (
                    <ToolCallComponent
                      key={toolCall.tool_use_id}
                      content={toolCall}
                      expanded={expandedToolCalls.has(toolCall.tool_use_id || '')}
                      onToggle={() => onToggleToolCall(toolCall.tool_use_id || '')}
                    />
                  ))}
                </div>
              )}
            </div>
          )
        }
      })}
    </div>
  )
}
