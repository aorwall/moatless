import type { CommandExecutionEvent, CommandExecutionOptions } from '@moatless/types'
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
      lineIdCounter.current = 0

      // Add initial system message
      addLine('system', `Starting execution of ${options.commands.length} command(s)...`)

      // Create abort controller for cancellation
      const abortController = new AbortController()
      abortControllerRef.current = abortController

      try {
        // Send the POST request to initiate the command execution
        const response = await fetch(`/api/v1/repositories/${repositoryId}/execute-commands`, {
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
                const data = JSON.parse(line.substring(6)) as CommandExecutionEvent['data']
                handleSSEEvent(currentEventType, data)
              } catch (error) {
                console.error('Failed to parse SSE data:', error)
              }
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

      function handleSSEEvent(eventType: string, data: CommandExecutionEvent['data']) {
        switch (eventType) {
          case 'start':
            if (data.command) {
              // Starting a specific command
              setState((prev) => ({
                ...prev,
                currentCommandIndex: data.commandIndex ?? null,
              }))
              addLine('command', data.command)
              addLine(
                'system',
                `Executing command ${(data.commandIndex ?? 0) + 1} of ${options.commands.length}...`,
                data.timestamp,
              )
            } else {
              // Starting the overall execution
              setState((prev) => ({
                ...prev,
                totalCommands: data.total ?? options.commands.length,
              }))
            }
            break

          case 'output':
            if (data.text) {
              const outputType = data.outputType === 'stderr' ? 'stderr' : 'stdout'
              addLine(outputType, data.text)
            }
            break

          case 'complete':
            {
              const exitCode = data.exitCode ?? 0
              const duration = data.duration ?? 0
              const commandNum = (data.commandIndex ?? 0) + 1

              if (exitCode === 0) {
                addLine(
                  'system',
                  `✓ Command ${commandNum} completed successfully (${duration}ms)`,
                  data.timestamp,
                )
              } else {
                addLine(
                  'system',
                  `✗ Command ${commandNum} failed with exit code ${exitCode} (${duration}ms)`,
                  data.timestamp,
                )
              }
            }
            break

          case 'done':
            setState((prev) => ({
              ...prev,
              isExecuting: false,
              summary: data.summary
                ? {
                    total: data.summary.total,
                    succeeded: data.summary.succeeded,
                    failed: data.summary.failed,
                    duration: data.summary.totalDuration, // Map totalDuration to duration
                  }
                : null,
              currentCommandIndex: null,
            }))

            if (data.summary) {
              addLine('system', '─'.repeat(50))
              addLine(
                'system',
                `Execution completed: ${data.summary.succeeded}/${data.summary.total} succeeded`,
                data.timestamp,
              )
              addLine('system', `Total duration: ${data.summary.totalDuration}ms`)
            }
            break

          case 'error':
            if (data.error) {
              addLine('stderr', `Error: ${data.error}`)
              setState((prev) => ({
                ...prev,
                error: data.error ?? 'Unknown error',
              }))
            }
            break
        }
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
