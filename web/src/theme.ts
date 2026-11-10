import type { Appearance } from './api'

/**
 * Put the chosen theme and accent on the document.
 *
 * The choice is kept by the binary (`settings::appearance`), never by the browser: keyhold keeps no
 * persistent storage in the page. "sistema" follows the desktop and keeps following it, so a
 * desktop that switches to dark at night takes keyhold along.
 */
let stopFollowing: (() => void) | null = null

export function applyAppearance(appearance: Appearance) {
  const root = document.documentElement

  stopFollowing?.()
  stopFollowing = null

  if (appearance.theme === 'sistema') {
    const query = window.matchMedia('(prefers-color-scheme: dark)')
    const follow = () => root.classList.toggle('dark', query.matches)

    follow()
    query.addEventListener('change', follow)
    stopFollowing = () => query.removeEventListener('change', follow)
  } else {
    root.classList.toggle('dark', appearance.theme !== 'claro')
  }

  if (appearance.accent === 'ambar') {
    delete root.dataset.accent
  } else {
    root.dataset.accent = appearance.accent
  }
}
