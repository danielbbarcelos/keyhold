import { secretInputProps, useFieldAutocomplete } from '../secretInput'
import { useRef, useState } from 'react'
import {
  CopyPlus,
  Eye,
  EyeOff,
  GripVertical,
  List,
  ListPlus,
  MessageSquareText,
  Plus,
  Replace,
  Star,
  Trash2,
  WandSparkles,
} from 'lucide-react'

import { DatePicker } from '@/components/common/DatePicker'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { t } from '@/i18n'
import { cn } from '@/lib/utils'

import { api, ApiError, type Field, type FieldKind } from '../../api'
import { toast } from '../toast'
import { CARD_PARTS, FIELD_TYPES, fieldType, hasParts, isSecret, newField, primaryField, subtitleField } from '../fields'
import { MenuButton } from '../menu'
import { PasswordGenerator } from '../PasswordGenerator'

/**
 * The fields of a record being written: each with a handle to drag, the icon of its kind, its
 * label, its value, and a menu of its own.
 *
 * The two marks a field can carry, "mostrar na lista" and "ação principal", are shown on the field
 * that currently plays each part, even when nobody marked it, so the rule is visible rather than
 * guessed at.
 */
export function FieldEditor({
  fields,
  onChange,
  framed = true,
  recordId,
}: {
  fields: Field[]
  onChange: (fields: Field[]) => void
  framed?: boolean
  /** The saved record, for fetching a sealed secret. */
  recordId?: string
}) {
  const [dragging, setDragging] = useState<number | null>(null)
  const [over, setOver] = useState<number | null>(null)
  // A stable key per field, moved along with it. Keyed by position, a field's "revealed" state
  // stayed in place when a field was removed or dragged, and showed the next secret in clear.
  const keys = useRef<string[]>([])
  if (keys.current.length !== fields.length) {
    keys.current = fields.map((_, at) => keys.current[at] ?? newKey())
  }
  const change = (next: Field[], nextKeys: string[]) => {
    keys.current = nextKeys
    onChange(next)
  }
  const listed = subtitleField(fields)
  const primary = primaryField(fields)

  const update = (at: number, change: Partial<Field>) =>
    onChange(fields.map((field, index) => (index === at ? { ...field, ...change } : field)))

  /** Only one field plays each part: marking one unmarks the rest. */
  const mark = (at: number, part: 'list' | 'primary') =>
    onChange(fields.map((field, index) => ({ ...field, [part]: index === at ? !field[part] : false })))

  const move = (from: number, to: number) => {
    if (from === to) return
    const next = [...fields]
    const nextKeys = [...keys.current]
    const [moved] = next.splice(from, 1)
    const [movedKey] = nextKeys.splice(from, 1)
    next.splice(to, 0, moved)
    nextKeys.splice(to, 0, movedKey)
    change(next, nextKeys)
  }

  return (
    <div className={cn(framed && 'overflow-visible rounded-xl border bg-raised')}>
      {fields.map((field, index) => (
        <div
          key={keys.current[index]}
          onDragOver={(event) => {
            if (dragging === null) return
            event.preventDefault()
            setOver(index)
          }}
          onDrop={(event) => {
            event.preventDefault()
            if (dragging !== null) move(dragging, index)
            setDragging(null)
            setOver(null)
          }}
          className={cn(
            'relative grid grid-cols-[16px_32px_minmax(120px,190px)_minmax(0,1fr)_32px] items-start gap-2 border-t px-4 py-2.5 transition-colors first:rounded-t-[11px] first:border-t-0',
            dragging === null && 'hover:bg-hover',
            over === index && dragging !== null && dragging !== index && 'bg-primary-soft',
            dragging === index && 'opacity-50',
          )}
        >
          <span
            draggable
            onDragStart={(event) => {
              event.dataTransfer.effectAllowed = 'move'
              event.dataTransfer.setData('text/keyhold-field', String(index))
              setDragging(index)
            }}
            onDragEnd={() => {
              setDragging(null)
              setOver(null)
            }}
            title={t('Arraste para reordenar')}
            className="mt-2.5 cursor-grab text-faint active:cursor-grabbing"
          >
            <GripVertical className="size-4" />
          </span>

          <TypeButton
            kind={field.kind}
            onPick={(kind) => update(index, retype(field, kind))}
          />

          <div className="relative">
            <Input
              aria-label={t('Nome do campo')}
              className={cn((listed === field || primary === field) && 'pr-[74px]')}
              value={field.label}
              placeholder={fieldType(field.kind).label}
              onChange={(event) => update(index, { label: event.target.value })}
            />
            {(listed === field || primary === field) && (
              <span className="pointer-events-none absolute top-1/2 right-2 flex -translate-y-1/2 gap-1">
                {primary === field ? <Mark primary>{t('principal')}</Mark> : <Mark>{t('na lista')}</Mark>}
              </span>
            )}
          </div>

          <ValueEditor field={field} recordId={recordId} onChange={(change) => update(index, change)} />

          <span className="mt-0.5">
            <MenuButton
              size="icon"
              label={t('Ações do campo {label}', { label: field.label })}
              align="end"
              entries={[
                ...(!isSecret(field)
                  ? [
                      {
                        kind: 'item' as const,
                        label: t('Mostrar na lista'),
                        icon: List,
                        checked: !!field.list,
                        onSelect: () => mark(index, 'list'),
                      },
                    ]
                  : []),
                ...(isSecret(field) && field.kind !== 'totp'
                  ? [
                      {
                        kind: 'item' as const,
                        label: t('Ação principal'),
                        icon: Star,
                        checked: !!field.primary,
                        onSelect: () => mark(index, 'primary'),
                      },
                    ]
                  : []),
                {
                  kind: 'sub',
                  label: t('Trocar tipo'),
                  icon: Replace,
                  items: FIELD_TYPES.map((type) => ({
                    label: type.label,
                    onSelect: () => update(index, retype(field, type.kind)),
                  })),
                },
                {
                  kind: 'item',
                  label: t('Duplicar'),
                  icon: CopyPlus,
                  onSelect: () =>
                    change(
                      [...fields.slice(0, index + 1), { ...field, list: false, primary: false }, ...fields.slice(index + 1)],
                      [...keys.current.slice(0, index + 1), newKey(), ...keys.current.slice(index + 1)],
                    ),
                },
                field.note === undefined
                  ? {
                      kind: 'item' as const,
                      label: t('Adicionar observação'),
                      icon: MessageSquareText,
                      onSelect: () => update(index, { note: '' }),
                    }
                  : {
                      kind: 'item' as const,
                      label: t('Remover observação'),
                      icon: MessageSquareText,
                      onSelect: () => update(index, { note: undefined }),
                    },
                { kind: 'separator' },
                {
                  kind: 'item',
                  label: t('Remover campo'),
                  icon: Trash2,
                  destructive: true,
                  onSelect: () =>
                    change(
                      fields.filter((_, at) => at !== index),
                      keys.current.filter((_, at) => at !== index),
                    ),
                },
              ]}
            />
          </span>

          {field.note !== undefined && (
            <div className="col-start-3 col-end-5">
              <div className="mb-1 flex items-center gap-1.5 text-xs text-muted-foreground">
                <MessageSquareText className="size-3.5" />
                {t('Observação')}
                <span className="flex-1" />
                <button
                  type="button"
                  className="text-faint hover:text-foreground"
                  onClick={() => update(index, { note: undefined })}
                >
                  {t('Remover')}
                </button>
              </div>
              <Textarea
                aria-label={t('Observação do campo {label}', { label: field.label })}
                className="min-h-16"
                placeholder={t('Algo que vale saber sobre este campo')}
                value={field.note}
                autoFocus={field.note === ''}
                onChange={(event) => update(index, { note: event.target.value })}
              />
            </div>
          )}
        </div>
      ))}

      {fields.length === 0 ? (
        <div className="flex flex-col items-center gap-2 px-4 py-6 text-center text-[13px] text-muted-foreground">
          <ListPlus className="size-5 text-faint" />
          <span>{t('Nenhum campo adicionado. Escolha um modelo acima ou adicione os campos necessários.')}</span>
          <AddField onAdd={(kind) => change([...fields, newField(kind)], [...keys.current, newKey()])} button />
        </div>
      ) : (
        <AddField onAdd={(kind) => change([...fields, newField(kind)], [...keys.current, newKey()])} />
      )}
    </div>
  )
}

/**
 * A field changing kind. Parts that only a card or a security question has are dropped when the
 * new kind has none, so a card's code does not stay stored, unseen, in a text field; and a field
 * that stops being secret loses its marks rather than surfacing in a list.
 */
function retype(field: Field, kind: FieldKind): Partial<Field> {
  const next: Partial<Field> = { kind, extra: hasParts(kind) ? (field.extra ?? {}) : undefined }

  if (isSecret({ kind })) next.list = false
  else next.primary = false

  return next
}

let counter = 0
function newKey(): string {
  counter += 1

  return `campo-${counter}`
}

function Mark({ children, primary }: { children: React.ReactNode; primary?: boolean }) {
  return (
    <span
      className={cn(
        'rounded-full px-2 py-px text-[11px] font-medium',
        primary ? 'bg-primary-soft text-primary-text' : 'bg-stone-soft text-muted-foreground',
      )}
    >
      {children}
    </span>
  )
}

function ValueEditor({
  field,
  recordId,
  onChange,
}: {
  field: Field
  recordId?: string
  onChange: (change: Partial<Field>) => void
}) {
  const type = fieldType(field.kind)
  const [showing, setShowing] = useState(false)
  // Revealed for editing: the value was fetched so it could be read, so the inputs it turns into
  // open showing it rather than masked behind a second click.
  const [revealed, setRevealed] = useState(false)
  const extra = field.extra ?? {}
  const setExtra = (key: string, value: string) => onChange({ extra: { ...extra, [key]: value } })

  // A saved secret the page was never given. It stays as it is unless somebody reveals it (fetched
  // and noted in the trail, then editable) or replaces it with a new value.
  if (field.sealed !== undefined && type.secret) {
    const unseal = (value: string, parts: Record<string, string>) => {
      setRevealed(true)
      setShowing(true)
      onChange({ value, extra: { ...extra, ...parts }, sealed: undefined, hint: undefined })
    }

    return (
      <div className="space-y-2">
        {field.kind === 'security_question' && (
          <Input
            aria-label={t('Pergunta')}
            placeholder={t('Pergunta')}
            value={extra.question ?? ''}
            onChange={(event) => setExtra('question', event.target.value)}
          />
        )}
        {field.kind === 'card' && (
          <div className="grid grid-cols-2 gap-2">
            {CARD_PARTS.filter((part) => !part.secret).map((part) => (
              <Input
                key={part.key}
                aria-label={part.label}
                placeholder={part.placeholder ?? part.label}
                value={extra[part.key] ?? ''}
                onChange={(event) => setExtra(part.key, event.target.value)}
              />
            ))}
          </div>
        )}
        <SealedValue
          hint={field.hint}
          onReveal={
            recordId
              ? () =>
                  api.secret(recordId, field.sealed!, 'reveal').then((secret) => unseal(secret.value, secret.extra))
              : undefined
          }
          onReplace={() => {
            const parts = Object.fromEntries(Object.entries(extra).filter(([key]) => key !== 'code' && key !== 'pin'))
            onChange({ value: '', extra: parts, sealed: undefined, hint: undefined })
          }}
        />
      </div>
    )
  }

  if (field.kind === 'security_question') {
    return (
      <div className="space-y-2">
        <Input
          aria-label={t('Pergunta')}
          placeholder={t('Pergunta')}
          value={extra.question ?? ''}
          onChange={(event) => setExtra('question', event.target.value)}
        />
        <SecretInput startShowing={revealed} value={field.value} placeholder={t('Resposta')} onChange={(value) => onChange({ value })} />
      </div>
    )
  }

  if (field.kind === 'card') {
    return (
      <div className="grid grid-cols-2 gap-2">
        <div className="col-span-2">
          <SecretInput
              startShowing={revealed}
            value={field.value}
            placeholder={t('Número do cartão')}
            mono
            onChange={(value) => onChange({ value })}
          />
        </div>
        {CARD_PARTS.map((part) =>
          part.secret ? (
            <SecretInput
              startShowing={revealed}
              key={part.key}
              value={extra[part.key] ?? ''}
              placeholder={part.placeholder ?? part.label}
              onChange={(value) => setExtra(part.key, value)}
            />
          ) : (
            <Input
              key={part.key}
              aria-label={part.label}
              placeholder={part.placeholder ?? part.label}
              value={extra[part.key] ?? ''}
              onChange={(event) => setExtra(part.key, event.target.value)}
            />
          ),
        )}
      </div>
    )
  }

  if (type.multiline) {
    // A hidden multi-line value is not drawn at all until asked for: `-webkit-text-security` does
    // not exist in every browser, and a textarea it does not cover shows the secret in full.
    if (type.secret && !showing) {
      return (
        <button
          type="button"
          onClick={() => setShowing(true)}
          className="flex min-h-20 w-full items-start justify-between rounded-[9px] border bg-field px-3 py-2 text-left text-sm text-muted-foreground"
        >
          <span className="tracking-[0.2em]">{field.value ? '••••••••••••' : <span className="tracking-normal text-faint">{t('Vazio')}</span>}</span>
          <span className="flex items-center gap-1 text-xs">
            <Eye className="size-3.5" />
            {t('Mostrar para editar')}
          </span>
        </button>
      )
    }

    return (
      <div className="relative">
        <Textarea
          aria-label={t('Valor')}
          autoComplete="off"
          spellCheck={!type.secret}
          autoCorrect={type.secret ? 'off' : undefined}
          autoCapitalize={type.secret ? 'off' : undefined}
          className={cn('min-h-20 pr-10', field.kind === 'multiline_hidden' && 'font-mono text-[13px]')}
          value={field.value}
          placeholder={type.placeholder}
          onChange={(event) => onChange({ value: event.target.value })}
        />
        {type.secret && (
          <Button
            variant="ghost"
            size="icon-sm"
            className="absolute top-1.5 right-1.5"
            aria-label={t('Ocultar')}
            onClick={() => setShowing(false)}
          >
            <EyeOff />
          </Button>
        )}
      </div>
    )
  }

  if (type.secret) {
    return (
      <SecretInput
        startShowing={revealed}
        value={field.value}
        placeholder={type.placeholder}
        mono={field.kind === 'pin' || field.kind === 'totp'}
        generate={field.kind === 'password'}
        onChange={(value) => onChange({ value })}
      />
    )
  }

  if (field.kind === 'date') {
    return <DatePicker value={field.value} placeholder={t('dd/mm/aaaa')} onChange={(value) => onChange({ value })} />
  }

  return (
    <Input
      aria-label={t('Valor')}
      type={field.kind === 'email' ? 'email' : field.kind === 'phone' ? 'tel' : 'text'}
      inputMode={field.kind === 'phone' ? 'tel' : undefined}
      value={field.value}
      placeholder={type.placeholder}
      onChange={(event) => onChange({ value: event.target.value })}
    />
  )
}

/** A saved secret in the editor: unchanged unless revealed for editing or replaced. */
function SealedValue({
  hint,
  onReveal,
  onReplace,
}: {
  hint?: string
  onReveal?: () => Promise<void>
  onReplace: () => void
}) {
  const [busy, setBusy] = useState(false)

  return (
    <div className="flex min-h-9 w-full flex-wrap items-center justify-between gap-2 rounded-[9px] border bg-field px-3 py-1.5 text-sm text-muted-foreground">
      <span className="tracking-[0.2em]">{hint ? `•••• ${hint}` : '••••••••••••'}</span>
      <span className="flex items-center gap-1">
        {onReveal && (
          <Button
            variant="ghost"
            size="sm"
            disabled={busy}
            onClick={() => {
              setBusy(true)
              onReveal()
                .catch((cause) =>
                  toast(t('Não foi possível revelar'), cause instanceof ApiError ? cause.message : t('Tente de novo.'), 'error'),
                )
                .finally(() => setBusy(false))
            }}
          >
            <Eye />
            {t('Mostrar para editar')}
          </Button>
        )}
        <Button variant="ghost" size="sm" onClick={onReplace}>
          <Replace />
          {t('Substituir')}
        </Button>
      </span>
    </div>
  )
}

function SecretInput({
  value,
  placeholder,
  mono,
  generate,
  startShowing = false,
  onChange,
}: {
  /** Open showing the value: it was just revealed on purpose. */
  startShowing?: boolean
  value: string
  placeholder?: string
  mono?: boolean
  generate?: boolean
  onChange: (value: string) => void
}) {
  const [showing, setShowing] = useState(startShowing)
  const [generating, setGenerating] = useState(false)
  const allow = useFieldAutocomplete()

  return (
    <div className="relative">
      <Input
        aria-label={placeholder ?? t('Valor')}
        {...secretInputProps(showing, allow)}
        spellCheck={false}
        autoCorrect="off"
        autoCapitalize="off"
        className={cn(generate ? 'pr-[68px]' : 'pr-10', mono && showing && 'font-mono text-[13px]')}
        value={value}
        placeholder={placeholder}
        onChange={(event) => onChange(event.target.value)}
      />
      <span className="absolute top-1/2 right-1 flex -translate-y-1/2 items-center">
        {generate && (
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                tooltip={false}
                variant="ghost"
                size="icon-sm"
                aria-label={t('Gerar senha')}
                onClick={() => setGenerating(true)}
              >
                <WandSparkles />
              </Button>
            </TooltipTrigger>
            <TooltipContent>{t('Gerar senha')}</TooltipContent>
          </Tooltip>
        )}
        {generating && (
          <PasswordGenerator
            onCancel={() => setGenerating(false)}
            onUse={(made) => {
              setGenerating(false)
              onChange(made)
              setShowing(true)
            }}
          />
        )}
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={showing ? t('Ocultar') : t('Mostrar')}
          onClick={() => setShowing(!showing)}
        >
          {showing ? <EyeOff /> : <Eye />}
        </Button>
      </span>
    </div>
  )
}

/** The icon of a field's kind, which is also where its kind is changed. */
function TypeButton({ kind, onPick }: { kind: FieldKind; onPick: (kind: FieldKind) => void }) {
  const [open, setOpen] = useState(false)
  const Icon = fieldType(kind).icon

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          title={t('{type} (trocar o tipo)', { type: fieldType(kind).label })}
          className="mt-0.5 grid size-8 place-items-center rounded-lg border bg-card text-muted-foreground hover:text-foreground"
        >
          <Icon className="size-[15px]" />
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-[560px] p-0">
        <TypeGrid
          title={t('Trocar tipo')}
          onPick={(picked) => {
            onPick(picked)
            setOpen(false)
          }}
        />
      </PopoverContent>
    </Popover>
  )
}

function AddField({ onAdd, button }: { onAdd: (kind: FieldKind) => void; button?: boolean }) {
  const [open, setOpen] = useState(false)

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        {button ? (
          <Button variant="outline" size="sm">
            <Plus />
            {t('Adicionar campo')}
          </Button>
        ) : (
          <button
            type="button"
            className="flex h-11 w-full items-center gap-2 rounded-b-xl border-t px-5 text-sm font-medium text-muted-foreground hover:bg-hover hover:text-foreground"
          >
            <Plus className="size-4" />
            {t('Adicionar campo')}
          </button>
        )}
      </PopoverTrigger>
      <PopoverContent className="w-[560px] p-0" align="start">
        <TypeGrid
          title={t('Adicionar campo')}
          onPick={(kind) => {
            onAdd(kind)
            setOpen(false)
          }}
        />
      </PopoverContent>
    </Popover>
  )
}

function TypeGrid({ title, onPick }: { title: string; onPick: (kind: FieldKind) => void }) {
  return (
    <>
      <div className="flex h-12 items-center px-4 text-sm font-semibold">{title}</div>
      <div className="grid grid-cols-3 gap-0.5 px-1.5 pb-1.5">
        {FIELD_TYPES.map(({ kind, label, icon: Icon }) => (
          <button
            key={kind}
            type="button"
            onClick={() => onPick(kind)}
            className="flex h-[38px] items-center gap-2.5 rounded-lg px-2.5 text-left text-[13px] hover:bg-hover"
          >
            <Icon className="size-4 text-muted-foreground" />
            {label}
          </button>
        ))}
      </div>
    </>
  )
}
