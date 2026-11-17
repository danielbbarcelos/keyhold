import { useSyncExternalStore } from 'react'

import { EN } from './en'

/**
 * Portuguese and English for every word on screen.
 *
 * Portuguese is the source: the code says `t('Copiar senha')`, and the English comes from a catalog
 * keyed by that Portuguese text, one file per area under `en/`. A text with values names them in
 * braces, `t('{n} registros', { n })`, filled in either language. `npm run i18n:check` fails on
 * Portuguese copy outside `t()` and on a `t()` without English.
 *
 * The language is this Keyhold's (a plain setting the binary keeps, readable before unlocking);
 * until somebody chooses, the first screen follows the browser.
 */
export type Lang = 'pt-BR' | 'en'

let current: Lang = 'pt-BR'
const listeners = new Set<() => void>()

export function getLang(): Lang {
  return current
}

export function setLang(lang: Lang): void {
  if (lang === current) return
  current = lang
  document.documentElement.lang = lang
  listeners.forEach((listener) => listener())
}

/** Re-render when the language changes. */
export function useLang(): Lang {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    () => current,
  )
}

/** The browser's language, as Keyhold speaks it: English for any `en`, Portuguese otherwise. */
export function browserLang(): Lang {
  return navigator.languages?.some((tag) => tag.toLowerCase().startsWith('en')) &&
    !navigator.languages.some((tag, index) => tag.toLowerCase().startsWith('pt') && index === 0)
    ? 'en'
    : 'pt-BR'
}

export function fromTag(tag: string | null | undefined): Lang | null {
  if (!tag) return null
  return tag.toLowerCase().startsWith('en') ? 'en' : 'pt-BR'
}

/** `pt` in the current language, with `{name}` replaced by each value. */
export function t(pt: string, values?: Record<string, string | number>): string {
  const chosen = current === 'en' ? (EN[pt] ?? pt) : pt
  if (!values) return chosen

  return chosen.replace(/\{(\w+)\}/g, (whole, name: string) => (name in values ? String(values[name]) : whole))
}

/** The locale for dates and numbers. */
export function locale(): string {
  return current === 'en' ? 'en-US' : 'pt-BR'
}
