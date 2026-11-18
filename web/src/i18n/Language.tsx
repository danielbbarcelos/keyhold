import { useId } from 'react'

import { api } from '@/api'
import { cn } from '@/lib/utils'

import { setLang, t, useLang, type Lang } from '.'

/**
 * The language switch: the toggle with flags (first run, unlock, the footer) and the names the
 * Idioma section shows. Each language is named in itself, so a reader who cannot read the current one
 * still finds theirs.
 */
export const LANGUAGES: Array<{ lang: Lang; short: string; name: string }> = [
  { lang: 'pt-BR', short: 'PT', name: 'Português (Brasil)' },
  { lang: 'en', short: 'EN', name: 'English (US)' },
]

/** Switch the screen now and save it for this Keyhold (and the binary's messages). */
export async function chooseLanguage(lang: Lang): Promise<void> {
  setLang(lang)
  await api.setLanguage(lang).catch(() => undefined)
}

/** Drawn here rather than taken from an icon set, so no third-party artwork ships with Keyhold. */
export function Flag({ lang, className }: { lang: Lang; className?: string }) {
  const clip = useId()

  return (
    <svg viewBox="0 0 20 14" aria-hidden="true" className={cn('h-[13px] w-[18px] shrink-0', className)}>
      <clipPath id={clip}>
        <rect width="20" height="14" rx="2.2" />
      </clipPath>
      <g clipPath={`url(#${CSS.escape(clip)})`}>
        {lang === 'pt-BR' ? (
          <>
            <rect width="20" height="14" fill="#009C3B" />
            <path d="M10 1.7 18.2 7 10 12.3 1.8 7Z" fill="#FFDF00" />
            <circle cx="10" cy="7" r="3.3" fill="#002776" />
            <path d="M6.8 6.3a7 7 0 0 1 6.4 1.3" stroke="#fff" strokeWidth="0.7" fill="none" />
          </>
        ) : (
          <>
            <rect width="20" height="14" fill="#fff" />
            {[0, 2, 4, 6, 8, 10, 12].map((stripe) => (
              <rect key={stripe} y={(stripe * 14) / 13} width="20" height={14 / 13} fill="#B22234" />
            ))}
            <rect width="9" height={(7 * 14) / 13} fill="#3C3B6E" />
            {[1.5, 4.5, 7.5].flatMap((x) =>
              [1.5, 3.8, 6].map((y) => <circle key={`${x}-${y}`} cx={x} cy={y} r="0.55" fill="#fff" />),
            )}
          </>
        )}
      </g>
    </svg>
  )
}

/** Two options, the app's segmented control, each with its flag. */
export function LanguageToggle({ onChanged }: { onChanged?: () => void }) {
  const current = useLang()

  return (
    <div role="group" aria-label={t('Idioma')} className="flex shrink-0 rounded-lg border bg-raised p-0.5">
      {LANGUAGES.map(({ lang, short, name }) => (
        <button
          key={lang}
          type="button"
          lang={lang}
          title={name}
          aria-pressed={current === lang}
          onClick={() => void chooseLanguage(lang).then(onChanged)}
          className={cn(
            'inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-[13px] font-medium text-muted-foreground hover:text-foreground',
            current === lang && 'bg-card text-foreground shadow-sm',
          )}
        >
          <Flag lang={lang} />
          {short}
        </button>
      ))}
    </div>
  )
}

/** What the footer says about Keyhold, from /api/status (the interface carries no address of its own). */
let about: { version?: string; homepage?: string } = {}

/** Called by the app whenever the status arrives, so every footer has the version and the link. */
export function setAbout(next: { version?: string; homepage?: string }): void {
  about = next
}

/**
 * The foot of every page: Keyhold on the left, the language on the right. It sits after the
 * content, at the end of the scroll; a page with little content still has it at the bottom of the
 * window, because the column it ends is at least as tall as the window.
 */
export function PageFooter({ className }: { className?: string }) {
  const [before, after] = t('Keyhold {version} · código aberto no {link}', { version: about.version ?? '' }).split('{link}')

  return (
    <footer className={cn('mt-auto flex shrink-0 flex-wrap items-center justify-between gap-x-4 gap-y-2 border-t pt-3 text-xs text-faint', className)}>
      <p>
        {before}
        {about.homepage ? (
          <a href={about.homepage} target="_blank" rel="noopener noreferrer" className="text-link hover:underline">
            GitHub
          </a>
        ) : (
          'GitHub'
        )}
        {after}
      </p>
      <LanguageToggle />
    </footer>
  )
}
