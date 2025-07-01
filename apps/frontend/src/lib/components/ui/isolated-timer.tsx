import React, { useEffect, useState } from 'react'

interface IsolatedTimerProps {
  startTime: Date | string | number
  className?: string
}

export const IsolatedTimer = React.memo(function IsolatedTimer({
  startTime,
  className = '',
}: IsolatedTimerProps) {
  const [now, setNow] = useState(Date.now())

  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [])

  const start = new Date(startTime).getTime()
  const elapsed = Math.max(0, now - start)
  const seconds = Math.floor(elapsed / 1000)
  const minutes = Math.floor(seconds / 60)
  const hours = Math.floor(minutes / 60)

  const formatted =
    hours > 0
      ? `${hours}h ${minutes % 60}m`
      : minutes > 0
        ? `${minutes}m ${seconds % 60}s`
        : `${seconds}s`

  return <span className={className}>{formatted}</span>
})
