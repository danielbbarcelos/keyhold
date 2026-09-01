/**
 * The session token, held the only place keyhold allows: memory.
 *
 * It arrives in the URL fragment — the one part of an address the browser never sends anywhere —
 * and stays in this module for the life of the tab. There is no `localStorage`, no
 * `sessionStorage`, no cookie and no IndexedDB anywhere in this bundle, and a test enforces that:
 * a vault that survives a reboot in a place a script can read is a vault that never locked.
 *
 * The fragment is deliberately **not** stripped from the address bar. Clearing it would look
 * tidier and would mean a reload lands on a page that can no longer talk to the binary, which
 * teaches people to restart keyhold to fix it — a worse habit than a visible token in a URL that
 * is void the moment the process exits.
 */

let token = ''

export function adoptTokenFromLocation(): void {
  const fragment = window.location.hash.replace(/^#/, '')
  const found = new URLSearchParams(fragment).get('token')

  if (found) {
    token = found
  }
}

/**
 * Forget it.
 *
 * Called when the binary says the credential is not one of its own — most often a tab left open
 * across a restart, or one whose instance was replaced by another. Keeping it would leave the
 * interface asking a lock screen to fix something a master password cannot fix.
 */
export function disown(): void {
  token = ''
}

export function hasToken(): boolean {
  return token.length > 0
}

/** Only the API client calls this. */
export function authHeaders(): Record<string, string> {
  return token ? { 'X-Keyhold-Token': token } : {}
}
