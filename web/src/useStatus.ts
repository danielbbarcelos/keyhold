import { useCallback, useEffect, useState } from 'react'

import { api, type Status } from './api'

/**
 * The one piece of state the whole interface hangs off, and the clock that keeps it honest.
 *
 * Three things happen here, and all three exist because the lock is real rather than decorative:
 *
 *  - **The countdown is refreshed from the binary**, once a second while unlocked. The number on
 *    screen is the daemon's, not a timer this page is running: a tab that gets throttled or
 *    suspended would otherwise show time the vault no longer has.
 *  - **The window's visibility is reported**, because a hidden window has a much shorter fuse than
 *    an idle one, and the browser is the only thing that knows.
 *  - **It fails closed.** Before the first answer arrives, and after any failure, the vault reads as
 *    locked. An interface that renders optimistically is one that will one day show an unlocked
 *    vault that is not.
 */
export function useStatus() {
  const [status, setStatus] = useState<Status | null>(null)
  const [offline, setOffline] = useState(false)

  const refresh = useCallback(async () => {
    try {
      setStatus(await api.status())
      setOffline(false)
    } catch {
      setOffline(true)
    }
  }, [])

  useEffect(() => {
    void refresh()
  }, [refresh])

  // While the vault is open, keep the countdown truthful.
  useEffect(() => {
    if (!status?.lock.unlocked) {
      return
    }

    const timer = window.setInterval(() => void refresh(), 1000)

    return () => window.clearInterval(timer)
  }, [status?.lock.unlocked, refresh])

  useEffect(() => {
    const report = () => {
      void api.visibility(document.visibilityState === 'visible').then((reply) => {
        setStatus((current) => (current ? { ...current, lock: reply.lock } : current))
      })
    }

    document.addEventListener('visibilitychange', report)

    return () => document.removeEventListener('visibilitychange', report)
  }, [])

  return { status, offline, refresh, setStatus }
}
