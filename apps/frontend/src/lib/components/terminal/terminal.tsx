import { type HTMLAttributes, useEffect, useRef } from 'react'
import { cn } from '@/lib/utils'

export interface TerminalLine {
  id: string
  type: 'command' | 'stdout' | 'stderr' | 'system'
  text: string
  timestamp?: string
}

interface TerminalProps extends HTMLAttributes<HTMLDivElement> {
  lines: TerminalLine[]
  autoScroll?: boolean
}

export function Terminal({ lines, autoScroll = true, className, ...props }: TerminalProps) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const lastLineCountRef = useRef(lines.length)

  useEffect(() => {
    if (autoScroll && scrollRef.current && lines.length > lastLineCountRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight
    }
    lastLineCountRef.current = lines.length
  }, [lines, autoScroll])

  return (
    <div
      ref={scrollRef}
      className={cn(
        'bg-black text-green-400 font-mono text-sm p-4 rounded-lg overflow-auto',
        'min-h-[200px] max-h-[500px]',
        className,
      )}
      {...props}
    >
      {lines.map((line) => (
        <div key={line.id} className="leading-relaxed">
          {line.type === 'command' && (
            <div className="flex items-start">
              <span className="text-blue-400 mr-2">$</span>
              <span className="text-white font-semibold">{line.text}</span>
            </div>
          )}
          {line.type === 'stdout' && (
            <div className="text-gray-300 whitespace-pre-wrap">{line.text}</div>
          )}
          {line.type === 'stderr' && (
            <div className="text-red-400 whitespace-pre-wrap">{line.text}</div>
          )}
          {line.type === 'system' && (
            <div className="text-yellow-400 italic">
              {line.timestamp && <span className="text-gray-500 mr-2">[{line.timestamp}]</span>}
              {line.text}
            </div>
          )}
        </div>
      ))}
      {lines.length === 0 && <div className="text-gray-500 italic">Waiting for output...</div>}
    </div>
  )
}
