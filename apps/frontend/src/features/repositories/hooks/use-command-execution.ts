import type { CommandExecutionOptions } from '@moatless/types'
import { useCallback, useRef, useState } from 'react'
import type { TerminalLine } from '@/lib/components/terminal/terminal'

interface CommandExecutionState {
  lines: TerminalLine[]
  isExecuting: boolean
  error: string | null
  currentCommandIndex: number | null
  totalCommands: number
  summary: {
    total: number
    succeeded: number
    failed: number
    duration: number
  } | null
}

export function useCommandExecution() {
  const [state, setState] = useState<CommandExecutionState>({
    lines: [],
    isExecuting: false,
    error: null,
    currentCommandIndex: null,
    totalCommands: 0,
    summary: null,
  })

  const abortControllerRef = useRef<AbortController | null>(null)
  const lineIdCounter = useRef(0)

  const addLine = useCallback((type: TerminalLine['type'], text: string, timestamp?: string) => {
    const line: TerminalLine = {
      id: `line-${lineIdCounter.current++}`,
      type,
      text,
      timestamp,
    }
    setState((prev) => ({
      ...prev,
      lines: [...prev.lines, line],
    }))
  }, [])

  const executeCommands = useCallback(
    async (repositoryId: string, options: CommandExecutionOptions) => {
      // Reset state
      setState({
        lines: [],
        isExecuting: true,
        error: null,
        currentCommandIndex: null,
        totalCommands: options.commands.length,
        summary: null,
      })

      // Create abort controller for cancellation
      const abortController = new AbortController()
      abortControllerRef.current = abortController

      // Add initial system message
      addLine('system', `Executing ${options.commands.length} command(s)...`)
      addLine('system', `Purpose: ${options.purpose || 'custom'}`)
      addLine('system', '─'.repeat(50))

      try {
        const response = await fetch(`/api/v1/repositories/${repositoryId}/execute`, {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify(options),
          signal: abortController.signal,
        })

        if (!response.ok) {
          throw new Error(`Failed to start execution: ${response.statusText}`)
        }

        // The response is an SSE stream
        const reader = response.body?.getReader()
        const decoder = new TextDecoder()

        if (!reader) {
          throw new Error('No response body')
        }

        let buffer = ''
        let currentEventType = ''
        let commandCounter = 0
        let successCount = 0
        let failureCount = 0
        const startTime = Date.now()

        while (true) {
          const { done, value } = await reader.read()
          if (done) break

          buffer += decoder.decode(value, { stream: true })
          const lines = buffer.split('\n')
          buffer = lines.pop() || ''

          for (const line of lines) {
            if (line.startsWith('event: ')) {
              currentEventType = line.substring(7).trim()
            } else if (line.startsWith('data: ')) {
              try {
                const data = JSON.parse(line.substring(6))
                handleSSEEvent(currentEventType, data)
              } catch (error) {
                console.error('Failed to parse SSE data:', error)
              }
            }
          }
        }

        // Final summary if we completed all commands
        if (!abortController.signal.aborted) {
          const totalDuration = Date.now() - startTime
          setState((prev) => ({
            ...prev,
            isExecuting: false,
            summary: {
              total: options.commands.length,
              succeeded: successCount,
              failed: failureCount,
              duration: totalDuration,
            },
          }))

          addLine('system', '─'.repeat(50))
          addLine(
            'system',
            `Execution completed: ${successCount}/${options.commands.length} succeeded`,
          )
          addLine('system', `Total duration: ${totalDuration}ms`)
        }

        function handleSSEEvent(eventType: string, data: any) {
          // The backend now sends 'shell-log' events with ShellCommandLogEvent data
          if (eventType === 'shell-log') {
            const { logType, timestamp, text } = data

            // Map ShellCommandLogEvent types to TerminalLine types
            switch (logType) {
              case 'stdout':
                addLine('stdout', text, timestamp)
                break
              case 'stderr':
                addLine('stderr', text, timestamp)
                break
              case 'system':
                // Parse system messages to extract meaningful information
                if (text.includes('Starting command:')) {
                  const command = text.split('Starting command:')[1]?.split('\n')[0]?.trim()
                  if (command) {
                    commandCounter++
                    setState((prev) => ({
                      ...prev,
                      currentCommandIndex: commandCounter - 1,
                    }))
                    addLine('command', command)
                    addLine(
                      'system',
                      `Executing command ${commandCounter} of ${options.commands.length}...`,
                      timestamp,
                    )
                  }
                } else if (text.includes('Command completed')) {
                  // Extract exit code and duration from the text
                  const exitCodeMatch = text.match(/Exit code: (\d+)/)
                  const durationMatch = text.match(/Duration: (\d+)ms/)
                  const exitCode = exitCodeMatch ? Number.parseInt(exitCodeMatch[1]) : 0
                  const duration = durationMatch ? Number.parseInt(durationMatch[1]) : 0

                  if (exitCode === 0) {
                    successCount++
                    addLine(
                      'system',
                      `✓ Command ${commandCounter} completed successfully (${duration}ms)`,
                      timestamp,
                    )
                  } else {
                    failureCount++
                    addLine(
                      'system',
                      `✗ Command ${commandCounter} failed with exit code ${exitCode} (${duration}ms)`,
                      timestamp,
                    )
                  }
                } else if (text.includes('Error:')) {
                  addLine('stderr', text, timestamp)
                  setState((prev) => ({
                    ...prev,
                    error: text,
                  }))
                } else {
                  // Other system messages
                  addLine('system', text, timestamp)
                }
                break
              default:
                // Unknown log type, treat as system
                addLine('system', text, timestamp)
                break
            }
          }
        }
      } catch (error: any) {
        if (error.name !== 'AbortError') {
          setState((prev) => ({
            ...prev,
            isExecuting: false,
            error: error.message || 'Failed to execute commands',
          }))
          addLine('stderr', `Error: ${error.message}`)
        }
      } finally {
        abortControllerRef.current = null
      }
    },
    [addLine],
  )

  const stopExecution = useCallback(() => {
    if (abortControllerRef.current) {
      abortControllerRef.current.abort()
      abortControllerRef.current = null
      setState((prev) => ({
        ...prev,
        isExecuting: false,
      }))
      addLine('system', 'Execution stopped by user')
    }
  }, [addLine])

  return {
    ...state,
    executeCommands,
    stopExecution,
  }
}
