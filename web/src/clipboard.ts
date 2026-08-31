/**
 * Handing a secret to the clipboard, and taking it back.
 *
 * A copy is the one moment keyhold puts a plaintext secret somewhere it does not control, so it
 * leaves it there for a deadline and then overwrites it. Three things make that harder than a
 * `setTimeout`, and all three belong here rather than in whichever component asked:
 *
 *  - **The window is usually not focused when the timer fires.** Copying a password is what you do
 *    right before switching to whatever you are pasting into, and browsers refuse a clipboard write
 *    from a document that is not focused. Swallowing that refusal — which is what a bare `catch`
 *    does — leaves the secret on the clipboard forever, in exactly the case that happens every
 *    time. So the write waits for focus to come back instead of giving up.
 *  - **The secret may already be gone.** If you copied something of your own in the meantime,
 *    overwriting would wipe *your* clipboard, not ours. A copy made inside this page is observed,
 *    and cancels the pending clear.
 *  - **The hand-off outlives whoever started it.** The row holding the copy button is often gone
 *    before the timer fires, so this lives at module scope — one hand-off at a time for the whole
 *    page, because there is only one clipboard.
 *
 * It is still best effort, and the interface says so rather than promising more: the operating
 * system may keep a clipboard history that no page can reach.
 */

interface Handoff {
  timer: number
  onCleared: () => void
  release: () => void
}

let current: Handoff | null = null

/** End a hand-off exactly once, whatever ended it, without touching the clipboard. */
function settle(handoff: Handoff | null): void {
  if (!handoff || current !== handoff) {
    return
  }

  current = null
  handoff.release()
  handoff.onCleared()
}

/**
 * Overwrite the clipboard, waiting for focus if the window does not have it.
 *
 * An empty string is what we mean; some browsers refuse it, so a single space is the fallback — a
 * space is not the secret, which is the whole requirement.
 */
async function overwrite(): Promise<void> {
  const write = async () => {
    try {
      await navigator.clipboard.writeText('')
    } catch {
      await navigator.clipboard.writeText(' ')
    }
  }

  if (document.hasFocus()) {
    await write()

    return
  }

  await new Promise<void>((resolve) => {
    const onFocus = () => {
      window.removeEventListener('focus', onFocus)
      void write().then(resolve)
    }

    window.addEventListener('focus', onFocus)
  })
}

/**
 * Copy `value`, and clear it after `seconds`.
 *
 * `onCleared` runs when the value is off the clipboard — because the deadline passed, or because
 * the person replaced it themselves.
 */
export async function copyForSeconds(
  value: string,
  seconds: number,
  onCleared: () => void,
): Promise<void> {
  settle(current)

  await navigator.clipboard.writeText(value)

  const onCopyElsewhere = () => settle(handoff)

  const handoff: Handoff = {
    timer: window.setTimeout(() => {
      void overwrite().then(() => settle(handoff))
    }, seconds * 1000),
    onCleared,
    release: () => {
      window.clearTimeout(handoff.timer)
      document.removeEventListener('copy', onCopyElsewhere)
    },
  }

  // A copy made anywhere on this page means the clipboard is no longer ours to clear.
  document.addEventListener('copy', onCopyElsewhere)

  current = handoff
}

/** Give the clipboard back now — used when the vault locks. */
export function clearNow(): void {
  if (current) {
    void overwrite().then(() => settle(current))
  }
}
