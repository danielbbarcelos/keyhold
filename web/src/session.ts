/**
 * The session, as far as this page can know it.
 *
 * The session itself is an `HttpOnly` cookie: the browser keeps it and sends it, and no script on
 * this page can read it, which is the point. What arrives in the address is a one-time code, good
 * for thirty seconds and one exchange. It is traded for the cookie before anything renders and
 * taken out of the address at once, so the address bar, a reload and the history hold nothing that
 * still opens anything.
 *
 * There is still no `localStorage`, no `sessionStorage`, no `document.cookie` and no IndexedDB in
 * this bundle, and a test enforces that.
 */

/** The header the binary wants on every request. Its value is irrelevant; its presence is what
 * another site cannot add without asking, and keyhold never answers that question. */
export const MARK = { 'X-Keyhold': '1' } as const

let open = false
let replayed = false
let language: string | null = null

/**
 * Redeem the code in the address, if there is one, and find out whether this tab has a session.
 *
 * Without a code this is a reload, a second tab, or a bookmark: the cookie, if the browser still
 * has a live one, is what decides.
 */
export async function adoptSession(): Promise<void> {
  const params = new URLSearchParams(window.location.hash.replace(/^#/, ''))
  const code = params.get('code')

  if (code) {
    params.delete('code')
    const rest = params.toString()
    window.history.replaceState(null, '', `${window.location.pathname}${rest ? `#${rest}` : ''}`)

    try {
      const response = await fetch('/session', {
        method: 'POST',
        credentials: 'same-origin',
        headers: { 'Content-Type': 'application/json', Accept: 'application/json', ...MARK },
        body: JSON.stringify({ code }),
      })

      replayed = response.status === 409
    } catch {
      // Keyhold is gone: the probe below says so too.
    }
  }

  try {
    const response = await fetch('/api/status', {
      credentials: 'same-origin',
      headers: { Accept: 'application/json', ...MARK },
    })

    open = response.ok
    if (response.ok) {
      const status = await response.json().catch(() => ({}))
      language = typeof status.language === 'string' ? status.language : null
    }
  } catch {
    open = false
  }
}

/**
 * Forget it.
 *
 * Called when the binary says this tab's credential is not one of its own: a restart, a takeover,
 * or an unlock in another address.
 */
export function disown(): void {
  open = false
}

export function hasToken(): boolean {
  return open
}

/** The language the binary keeps for this Keyhold, as the first status said it; `null` when nobody
 * chose yet, or when this tab has no session to ask with. */
export function savedLanguage(): string | null {
  return language
}

/** Whether the code in the address had already been used when this tab presented it. */
export function wasReplayed(): boolean {
  return replayed
}

/** Only the API client calls this. */
export function authHeaders(): Record<string, string> {
  return { ...MARK }
}

/**
 * This page's own address with `extra` in the fragment, for a second tab. Nothing secret goes in
 * it: the new tab has the same cookie this one has.
 */
export function tabAddress(extra: URLSearchParams): string {
  const fragment = extra.toString()

  return `${window.location.origin}${window.location.pathname}${fragment ? `#${fragment}` : ''}`
}
