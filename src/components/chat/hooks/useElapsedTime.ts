import { useState, useEffect } from 'react'
import { formatDuration } from '../time-utils'

export function useElapsedTime(startTime: number | null): string | null {
  const [elapsed, setElapsed] = useState<number | null>(null)

  useEffect(() => {
    if (startTime == null) {
      setElapsed(null)
      return
    }
    setElapsed(Date.now() - startTime)
    const id = setInterval(() => setElapsed(Date.now() - startTime), 1000)
    return () => clearInterval(id)
  }, [startTime])

  if (elapsed == null) return null
  return formatDuration(elapsed)
}
