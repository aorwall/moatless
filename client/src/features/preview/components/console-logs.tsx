import { useEffect, useRef, useState } from 'react'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { usePreviewStore } from '../stores/preview-store'

interface ConsoleLog {
  id: string
  timestamp: string
  level: 'log' | 'info' | 'warn' | 'error' | 'debug'
  message: string
  args?: unknown[]
}

interface ConsoleMessage {
  type: 'console'
  level: ConsoleLog['level']
  message: string
  args?: unknown[]
}

export function ConsoleLogs() {
  const [logs, setLogs] = useState<ConsoleLog[]>([])
  const scrollAreaRef = useRef<HTMLDivElement>(null)
  const shouldAutoScroll = useRef(true)
  const config = usePreviewStore((state) => state.config)

  // Listen for console messages from the preview iframe
  useEffect(() => {
    const handleMessage = (event: MessageEvent) => {
      // Only process messages from the preview iframe
      if (event.data?.type === 'console' && event.data?.source === 'preview-iframe') {
        const { level, message, args } = event.data as ConsoleMessage & { source: string }

        const newLog: ConsoleLog = {
          id: `${Date.now()}-${Math.random()}`,
          timestamp: new Date().toISOString(),
          level,
          message,
          args,
        }

        setLogs((prev) => [...prev, newLog])
      }
    }

    window.addEventListener('message', handleMessage)
    return () => window.removeEventListener('message', handleMessage)
  }, [])

  // Clear logs when preview URL changes
  useEffect(() => {
    setLogs([])
  }, [config.url])

  // Auto-scroll to bottom when new logs arrive
  useEffect(() => {
    if (shouldAutoScroll.current && scrollAreaRef.current) {
      const scrollContainer = scrollAreaRef.current.querySelector(
        '[data-radix-scroll-area-viewport]',
      )
      if (scrollContainer) {
        scrollContainer.scrollTop = scrollContainer.scrollHeight
      }
    }
  }, [logs])

  const handleScroll = (e: React.UIEvent<HTMLDivElement>) => {
    const element = e.currentTarget
    const isAtBottom =
      Math.abs(element.scrollHeight - element.scrollTop - element.clientHeight) < 10
    shouldAutoScroll.current = isAtBottom
  }

  const getLevelColor = (level: ConsoleLog['level']) => {
    switch (level) {
      case 'error':
        return 'text-red-400'
      case 'warn':
        return 'text-yellow-400'
      case 'info':
        return 'text-blue-400'
      case 'debug':
        return 'text-gray-500'
      default:
        return 'text-slate-100 dark:text-slate-200'
    }
  }

  const formatMessage = (log: ConsoleLog) => {
    const timestamp = new Date(log.timestamp).toLocaleTimeString()
    const prefix = `[${timestamp}] [${log.level.toUpperCase()}]`
    return `${prefix} ${log.message}`
  }

  if (logs.length === 0) {
    return (
      <div className="flex h-full items-center justify-center p-8 text-center">
        <div className="text-muted-foreground">
          <p className="text-lg font-medium">No console output</p>
          <p className="mt-2 text-sm">Console logs from the preview app will appear here</p>
        </div>
      </div>
    )
  }

  return (
    <div className="h-full w-full bg-slate-950 dark:bg-black">
      <ScrollArea ref={scrollAreaRef} className="h-full w-full" onScroll={handleScroll}>
        <div className="p-4 space-y-1">
          {logs.map((log) => (
            <div
              key={log.id}
              className="text-xs leading-relaxed font-mono whitespace-pre-wrap break-all"
            >
              <span className={getLevelColor(log.level)}>{formatMessage(log)}</span>
            </div>
          ))}
        </div>
      </ScrollArea>
    </div>
  )
}
