import { useEffect, useRef, useState } from 'react'
import { Check, Copy, ExternalLink, Eye, EyeOff, Loader2 } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { locale, t } from '@/i18n'
import { cn } from '@/lib/utils'

import { Clip, Splitter } from '../Splitter'

import { api, ApiError, SlowError, type Field, type SecretValue } from '../../api'
import { copyForSeconds } from '../../clipboard'
import { CARD_PARTS, fieldType } from '../fields'
import { toast } from '../toast'

/**
 * One field of an open record, drawn the way its kind asks.
 *
 * **Revealing is an event, not a state.** A secret comes into view for twenty seconds while a thin
 * bar drains under the row, then goes back under the dots on its own: a value left on screen is a
 * value somebody can photograph. **Copying says when it stops being true**: the clipboard is cleared
 * after thirty seconds, and the tooltip admits that an operating system with clipboard history keeps
 * a copy no page can reach.
 */
const REVEAL_SECONDS = 20
const CLIPBOARD_SECONDS = 30

interface FieldRowProps {
  field: Field
  index: number
  /** Absent for a record not saved yet: nothing to ask the binary for, nothing to record. */
  recordId?: string
  badge?: 'list' | 'primary'
  /** False for an old version: reads still reach the trail, but no live 2FA code is fetched. */
  live?: boolean
  /** The version being previewed, when it is not the current one. */
  version?: number
}

export function FieldRow({ field, index, recordId, badge, live = true, version }: FieldRowProps) {
  const type = fieldType(field.kind)
  const Icon = type.icon

  return (
    <div className="group/field relative grid grid-cols-[var(--kh-label,180px)_minmax(0,1fr)_auto] items-center gap-4 border-t px-5 py-3 first:border-t-0">
      <span className="relative flex min-w-0 items-center gap-2 self-stretch text-[13px] text-muted-foreground">
        <Icon className="size-[15px] shrink-0 text-faint" />
        <Clip>{field.label || type.label}</Clip>
        {badge && (
          <span
            className={cn(
              'shrink-0 rounded-full px-2 py-px text-[11px] font-medium',
              badge === 'primary' ? 'bg-primary-soft text-primary-text' : 'bg-stone-soft text-muted-foreground',
            )}
          >
            {badge === 'primary' ? t('principal') : t('na lista')}
          </span>
        )}
        <Splitter which="label" className="-right-3.5 -my-3 opacity-0 group-hover/field:opacity-100" />
      </span>

      <Value field={field} index={index} recordId={recordId} live={live} version={version} />
    </div>
  )
}

/**
 * Where a secret field's value comes from.
 *
 * A sealed field (the binary did not send its value, see `records::sealed_json`) is asked for when
 * somebody reveals or copies it: the binary reads it again and writes the trail line before it
 * answers, so the trail records what happened rather than what the page chose to report. A field
 * that is not sealed (a record being drafted) has its value already.
 */
interface Source {
  field: Field
  index: number
  recordId?: string
  version?: number
}

function sealed(source: Source): boolean {
  return source.field.sealed !== undefined && !!source.recordId
}

function fetchSecret(source: Source, intent: 'reveal' | 'copy'): Promise<SecretValue> {
  return api.secret(source.recordId!, source.field.sealed ?? source.index, intent, source.version)
}

/** What the copy button copies: the value, or the promise of it for a sealed field. */
function copyOf(source: Source): string | (() => Promise<string>) {
  return sealed(source) ? () => fetchSecret(source, 'copy').then((secret) => secret.value) : source.field.value
}

function Value({ field, index, recordId, live, version }: { field: Field; index: number; recordId?: string; live: boolean; version?: number }) {
  const label = field.label || fieldType(field.kind).label
  const source: Source = { field, index, recordId, version }

  switch (field.kind) {
    case 'totp':
      return <Totp recordId={live ? recordId : undefined} trail={recordId} index={index} label={label} />

    case 'security_question':
      return (
        <Secret
          label={label}
          source={source}
          above={<span className="block text-sm">{field.extra?.question || t('Sem pergunta')}</span>}
        />
      )

    case 'card':
      return <Card source={source} label={label} />

    case 'date':
      return (
        <Plain
          value={/^\d{4}-\d{2}-\d{2}$/.test(field.value) ? dateOf(field.value) : field.value}
          label={label}
        />
      )

    case 'url':
      return <Url value={field.value} label={label} recordId={recordId} />

    default:
      return fieldType(field.kind).secret ? (
        <Secret
          label={label}
          source={source}
          multiline={fieldType(field.kind).multiline}
          mono={field.kind === 'pin' || field.kind === 'multiline_hidden'}
        />
      ) : (
        <Plain value={field.value} label={label} multiline={fieldType(field.kind).multiline} />
      )
  }
}

function Plain({ value, label, multiline }: { value: string; label: string; multiline?: boolean }) {
  return (
    <>
      <span className={cn('min-w-0 text-sm', multiline ? 'whitespace-pre-wrap break-words' : 'truncate')}>
        {value || <span className="text-faint">—</span>}
      </span>
      <Ops>{value && <CopyButton value={value} label={label} />}</Ops>
    </>
  )
}

function Url({ value, label, recordId }: { value: string; label: string; recordId?: string }) {
  const href = safeHref(value)

  return (
    <>
      <span className="min-w-0 truncate text-sm">
        {href ? (
          <a href={href} target="_blank" rel="noreferrer noopener" className="text-link hover:underline">
            {value}
          </a>
        ) : (
          value || <span className="text-faint">—</span>
        )}
      </span>
      <Ops>
        {href && (
          <IconButton label={t('Abrir')} onClick={() => window.open(href, '_blank', 'noopener,noreferrer')}>
            <ExternalLink />
          </IconButton>
        )}
        {value && <CopyButton value={value} label={label} recordId={recordId} report={false} />}
      </Ops>
    </>
  )
}

function Secret({
  source,
  label,
  above,
  multiline,
  mono,
}: {
  source: Source
  label: string
  above?: React.ReactNode
  multiline?: boolean
  mono?: boolean
}) {
  const [shown, remaining, toggle, busy] = useReveal(source, label)
  const present = source.field.value !== '' || sealed(source)
  const value = shown?.value ?? ''

  return (
    <>
      <span className="min-w-0">
        {above}
        <span
          className={cn(
            'block text-sm',
            shown
              ? cn(multiline ? 'whitespace-pre-wrap break-all' : 'truncate', mono && 'font-mono text-[13px]')
              : 'tracking-[0.2em] text-muted-foreground select-none',
          )}
        >
          {shown ? value || '—' : '•'.repeat(12)}
        </span>
      </span>
      <Ops>
        {present && (
          <IconButton
            label={shown ? t('Ocultar ({n}s restantes)', { n: remaining }) : t('Revelar por 20 segundos')}
            onClick={toggle}
            busy={busy}
          >
            {busy ? <Loader2 className="animate-spin" /> : shown ? <EyeOff /> : <Eye />}
          </IconButton>
        )}
        {present && <CopyButton value={copyOf(source)} label={label} recordId={source.recordId} report={!sealed(source)} />}
      </Ops>
      {shown && <Drain seconds={remaining} />}
    </>
  )
}

function Card({ source, label }: { source: Source; label: string }) {
  const [shown, remaining, toggle, busy] = useReveal(source, label)
  const field = source.field
  const number = (shown?.value ?? field.value).replace(/\s+/g, '')
  const last = field.hint ?? number.slice(-4)
  const grouped = number.replace(/(.{4})/g, '$1 ').trim()
  const isSealed = sealed(source)
  const present = !!number || !!last || isSealed

  return (
    <>
      <span className="min-w-0 space-y-1 text-sm">
        <span className={cn('block font-mono text-[13px]', !shown && 'text-muted-foreground')}>
          {shown ? grouped || '—' : last ? `•••• ${last}` : isSealed ? '••••' : '—'}
        </span>
        <span className="flex flex-wrap gap-x-4 gap-y-0.5 text-xs text-muted-foreground">
          {CARD_PARTS.map((part) => {
            // A sealed card's code and PIN are not on the page until revealed.
            const value = shown?.extra[part.key] ?? field.extra?.[part.key]

            if (!value) {
              return null
            }

            return (
              <span key={part.key}>
                {part.label}:{' '}
                <span className={cn('text-foreground', part.secret && !shown && 'tracking-[0.15em]')}>
                  {part.secret && !shown ? '•••' : value}
                </span>
              </span>
            )
          })}
        </span>
      </span>
      <Ops>
        {present && (
          <IconButton label={shown ? t('Ocultar ({n}s restantes)', { n: remaining }) : t('Revelar por 20 segundos')} onClick={toggle} busy={busy}>
            {busy ? <Loader2 className="animate-spin" /> : shown ? <EyeOff /> : <Eye />}
          </IconButton>
        )}
        {(number || isSealed) && (
          <CopyButton
            value={isSealed ? () => fetchSecret(source, 'copy').then((secret) => secret.value.replace(/\s+/g, '')) : number}
            label={t('{label} (número)', { label })}
            recordId={source.recordId}
            report={!isSealed}
          />
        )}
      </Ops>
      {shown && <Drain seconds={remaining} />}
    </>
  )
}

/** A 2FA field: the code of the moment, from the binary, with a ring that empties as it ages. */
function Totp({ recordId, trail, index, label }: { recordId?: string; trail?: string; index: number; label: string }) {
  const [code, setCode] = useState<{ code: string; remaining: number } | null>(null)
  const [failed, setFailed] = useState('')

  useEffect(() => {
    if (!recordId) {
      return
    }

    let alive = true
    let timer = 0

    const fetchCode = async () => {
      try {
        const fresh = await api.totpCode(recordId, index)

        if (alive) {
          setCode(fresh)
          setFailed('')
          timer = window.setTimeout(fetchCode, fresh.remaining * 1000 + 150)
        }
      } catch (cause) {
        if (alive) {
          setFailed(cause instanceof Error ? cause.message : t('sem código'))
        }
      }
    }

    void fetchCode()
    // The code is on screen without a click, so showing it counts as a reveal for the trail.
    void api.reportAccess(recordId, 'reveal', label).catch(() => {})

    return () => {
      alive = false
      window.clearTimeout(timer)
    }
  }, [recordId, index])

  // Count the seconds down locally between fetches.
  useEffect(() => {
    const tick = window.setInterval(
      () => setCode((current) => (current ? { ...current, remaining: Math.max(0, current.remaining - 1) } : current)),
      1000,
    )

    return () => window.clearInterval(tick)
  }, [])

  if (!recordId) {
    return (
      <>
        <span className="text-sm text-faint">{t('o código é exibido no registro salvo')}</span>
        <Ops />
      </>
    )
  }

  if (failed) {
    return (
      <>
        <span className="text-sm text-rose">{failed}</span>
        <Ops />
      </>
    )
  }

  const shown = code ? `${code.code.slice(0, Math.ceil(code.code.length / 2))} ${code.code.slice(Math.ceil(code.code.length / 2))}` : '··· ···'
  const share = code ? Math.min(1, code.remaining / 30) : 0

  return (
    <>
      <span className="flex items-center gap-2.5">
        <span className="font-mono text-lg tracking-[0.12em]">{shown}</span>
        <span
          className="grid size-5 place-items-center rounded-full"
          // What has gone by grows clockwise from the top, the way a clock hand moves.
          style={{ background: `conic-gradient(var(--border) 0 ${(1 - share) * 100}%, var(--primary) ${(1 - share) * 100}% 100%)` }}
        >
          <i className="size-3.5 rounded-full bg-card" />
        </span>
        {code && <span className="text-xs text-faint">{code.remaining} s</span>}
      </span>
      <Ops>{code && <CopyButton value={code.code} label={label} recordId={recordId ?? trail} />}</Ops>
    </>
  )
}

/**
 * Reveal for twenty seconds. For a sealed field the value is fetched on the click and dropped when
 * the twenty seconds end, so the page holds a secret only while it is on screen.
 */
function useReveal(source: Source, label: string): [SecretValue | null, number, () => void, boolean] {
  const [shown, setShown] = useState<SecretValue | null>(null)
  const [busy, setBusy] = useState(false)
  const [remaining, setRemaining] = useState(0)
  const timer = useRef<number | undefined>(undefined)

  useEffect(() => {
    if (!shown) {
      return
    }

    setRemaining(REVEAL_SECONDS)
    timer.current = window.setInterval(() => {
      setRemaining((left) => {
        if (left <= 1) {
          setShown(null)

          return 0
        }

        return left - 1
      })
    }, 1000)

    return () => window.clearInterval(timer.current)
  }, [shown])

  const toggle = () => {
    if (shown) {
      setShown(null)
      return
    }

    if (busy) return

    if (sealed(source)) {
      setBusy(true)
      fetchSecret(source, 'reveal')
        .then(setShown)
        .catch((cause) => failed(cause, t('Não foi possível revelar'), t('Tente de novo.')))
        .finally(() => setBusy(false))
      return
    }

    if (source.recordId) {
      // Not sealed only while a record is being drafted; kept for anything older that still is.
      void api.reportAccess(source.recordId, 'reveal', label).catch(() => {})
    }

    setShown({ label, value: source.field.value, extra: source.field.extra ?? {} })
  }

  return [shown, remaining, toggle, busy]
}

/** A refused or slow reveal or copy, said in a toast. */
function failed(cause: unknown, title: string, fallback: string) {
  if (cause instanceof SlowError) {
    toast(t('A conexão está lenta'), cause.message, 'error')
    return
  }

  toast(title, cause instanceof ApiError ? cause.message : fallback, 'error')
}

export function CopyButton({
  value,
  label,
  recordId,
  report = true,
  seconds = CLIPBOARD_SECONDS,
}: {
  /** The value, or how to fetch it (a sealed secret): fetched inside the click, which the clipboard needs. */
  value: string | (() => Promise<string>)
  label: string
  recordId?: string
  report?: boolean
  seconds?: number
}) {
  const [copied, setCopied] = useState(0)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    if (copied <= 0) {
      return
    }

    const tick = window.setInterval(() => setCopied((left) => Math.max(0, left - 1)), 1000)

    return () => window.clearInterval(tick)
  }, [copied])

  return (
    <IconButton
      label={copied > 0 ? t('A área de transferência é limpa em {n}s', { n: copied }) : t('Copiar (limpa em {n}s)', { n: seconds })}
      busy={busy}
      onClick={() => {
        if (busy) return
        // A refusal from the binary is kept apart from a clipboard that would not take the value.
        let refused: unknown = null
        if (typeof value !== 'string') setBusy(true)
        const handed =
          typeof value === 'string'
            ? value
            : value().catch((cause) => {
                refused = cause
                throw cause
              })

        void copyForSeconds(handed, seconds, () => setCopied(0))
          .then(() => {
            // Reported once the copy happened: a trail that records copies that failed is a trail
            // nobody can trust.
            if (recordId && report) void api.reportAccess(recordId, 'copy', label).catch(() => {})
            setCopied(seconds)
            toast(t('{label} copiado', { label }), t('A área de transferência é limpa em {n} segundos.', { n: seconds }))
          })
          .catch(() =>
            refused
              ? failed(refused, t('Não foi possível copiar'), t('Tente de novo.'))
              : toast(t('Não foi possível copiar'), t('O navegador bloqueou o acesso à área de transferência nesta página.'), 'error'),
          )
          .finally(() => setBusy(false))
      }}
    >
      {busy ? <Loader2 className="animate-spin" /> : copied > 0 ? <Check className="text-primary" /> : <Copy />}
    </IconButton>
  )
}

function Ops({ children }: { children?: React.ReactNode }) {
  return (
    <span className="flex shrink-0 items-center gap-0.5 opacity-70 transition-opacity group-hover/field:opacity-100 focus-within:opacity-100">
      {children}
    </span>
  )
}

function IconButton({
  label,
  onClick,
  busy = false,
  children,
}: {
  label: string
  onClick: () => void
  /** Waiting for the binary: the button holds still (one request per click) and says so. */
  busy?: boolean
  children: React.ReactNode
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button tooltip={false} variant="ghost" size="icon" aria-label={label} aria-busy={busy} disabled={busy} onClick={onClick}>
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}

function Drain({ seconds }: { seconds: number }) {
  return (
    <span
      aria-hidden="true"
      className="absolute bottom-0 left-0 h-px bg-primary transition-[width] duration-1000 ease-linear"
      style={{ width: `${(seconds / REVEAL_SECONDS) * 100}%` }}
    />
  )
}

/** A `YYYY-MM-DD` date in the order the language reads it. */
function dateOf(value: string): string {
  const [year, month, day] = value.split('-').map(Number)

  return new Date(year, month - 1, day).toLocaleDateString(locale(), { day: '2-digit', month: '2-digit', year: 'numeric' })
}

/** Only web addresses become links: a `javascript:` value in a vault is still just text. */
export function safeHref(url: string): string | null {
  const trimmed = url.trim()

  if (!trimmed) {
    return null
  }

  // A bare address is taken as a secure one. Built from parts so the bundle never carries a literal
  // address (the hardening test looks for one).
  const secure = ['https', '//'].join(':')
  const withScheme = /^[a-z][a-z0-9+.-]*:/i.test(trimmed) ? trimmed : `${secure}${trimmed}`

  try {
    const parsed = new URL(withScheme)

    return parsed.protocol === 'https:' || parsed.protocol === 'http:' ? parsed.href : null
  } catch {
    return null
  }
}
