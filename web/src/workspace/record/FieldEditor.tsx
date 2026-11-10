import { useRef, useState } from 'react'
import {
  CopyPlus,
  Eye,
  EyeOff,
  GripVertical,
  List,
  ListPlus,
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
import { cn } from '@/lib/utils'

import { api, type Field, type FieldKind } from '../../api'
import { CARD_PARTS, FIELD_TYPES, fieldType, hasParts, isSecret, newField, primaryField, subtitleField } from '../fields'
import { MenuButton } from '../menu'
import { toastError } from '../toast'

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
}: {
  fields: Field[]
  onChange: (fields: Field[]) => void
  framed?: boolean
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
            'relative grid grid-cols-[16px_32px_minmax(120px,190px)_minmax(0,1fr)_32px] items-start gap-2 border-t px-4 py-2.5 first:border-t-0',
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
            title="Arraste para reordenar"
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
              aria-label="Nome do campo"
              className={cn((listed === field || primary === field) && 'pr-[74px]')}
              value={field.label}
              placeholder={fieldType(field.kind).label}
              onChange={(event) => update(index, { label: event.target.value })}
            />
            {(listed === field || primary === field) && (
              <span className="pointer-events-none absolute top-1/2 right-2 flex -translate-y-1/2 gap-1">
                {primary === field ? <Mark primary>principal</Mark> : <Mark>na lista</Mark>}
              </span>
            )}
          </div>

          <ValueEditor field={field} onChange={(change) => update(index, change)} />

          <span className="mt-0.5">
            <MenuButton
              size="icon"
              label={`Ações do campo ${field.label}`}
              align="end"
              entries={[
                ...(!isSecret(field)
                  ? [
                      {
                        kind: 'item' as const,
                        label: 'Mostrar na lista',
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
                        label: 'Ação principal',
                        icon: Star,
                        checked: !!field.primary,
                        onSelect: () => mark(index, 'primary'),
                      },
                    ]
                  : []),
                {
                  kind: 'sub',
                  label: 'Trocar o tipo',
                  icon: Replace,
                  items: FIELD_TYPES.map((type) => ({
                    label: type.label,
                    onSelect: () => update(index, retype(field, type.kind)),
                  })),
                },
                {
                  kind: 'item',
                  label: 'Duplicar',
                  icon: CopyPlus,
                  onSelect: () =>
                    change(
                      [...fields.slice(0, index + 1), { ...field, list: false, primary: false }, ...fields.slice(index + 1)],
                      [...keys.current.slice(0, index + 1), newKey(), ...keys.current.slice(index + 1)],
                    ),
                },
                { kind: 'separator' },
                {
                  kind: 'item',
                  label: 'Remover o campo',
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
        </div>
      ))}

      {fields.length === 0 ? (
        <div className="flex flex-col items-center gap-2 px-4 py-6 text-center text-[13px] text-muted-foreground">
          <ListPlus className="size-5 text-faint" />
          <span>Nenhum campo ainda. Escolha um modelo acima ou adicione os campos que precisar.</span>
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

function ValueEditor({ field, onChange }: { field: Field; onChange: (change: Partial<Field>) => void }) {
  const type = fieldType(field.kind)
  const [showing, setShowing] = useState(false)
  const extra = field.extra ?? {}
  const setExtra = (key: string, value: string) => onChange({ extra: { ...extra, [key]: value } })

  if (field.kind === 'security_question') {
    return (
      <div className="space-y-2">
        <Input
          aria-label="Pergunta"
          placeholder="Pergunta"
          value={extra.question ?? ''}
          onChange={(event) => setExtra('question', event.target.value)}
        />
        <SecretInput value={field.value} placeholder="Resposta" onChange={(value) => onChange({ value })} />
      </div>
    )
  }

  if (field.kind === 'card') {
    return (
      <div className="grid grid-cols-2 gap-2">
        <div className="col-span-2">
          <SecretInput
            value={field.value}
            placeholder="Número do cartão"
            mono
            onChange={(value) => onChange({ value })}
          />
        </div>
        {CARD_PARTS.map((part) =>
          part.secret ? (
            <SecretInput
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
          <span className="tracking-[0.2em]">{field.value ? '••••••••••••' : <span className="tracking-normal text-faint">Vazio</span>}</span>
          <span className="flex items-center gap-1 text-xs">
            <Eye className="size-3.5" />
            Mostrar para editar
          </span>
        </button>
      )
    }

    return (
      <div className="relative">
        <Textarea
          aria-label="Valor"
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
            aria-label="Esconder"
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
        value={field.value}
        placeholder={type.placeholder}
        mono={field.kind === 'pin' || field.kind === 'totp'}
        generate={field.kind === 'password'}
        onChange={(value) => onChange({ value })}
      />
    )
  }

  if (field.kind === 'date') {
    return <DatePicker value={field.value} placeholder="dd/mm/aaaa" onChange={(value) => onChange({ value })} />
  }

  return (
    <Input
      aria-label="Valor"
      type={field.kind === 'email' ? 'email' : field.kind === 'phone' ? 'tel' : 'text'}
      inputMode={field.kind === 'phone' ? 'tel' : undefined}
      value={field.value}
      placeholder={type.placeholder}
      onChange={(event) => onChange({ value: event.target.value })}
    />
  )
}

function SecretInput({
  value,
  placeholder,
  mono,
  generate,
  onChange,
}: {
  value: string
  placeholder?: string
  mono?: boolean
  generate?: boolean
  onChange: (value: string) => void
}) {
  const [showing, setShowing] = useState(false)

  return (
    <div className="relative">
      <Input
        aria-label={placeholder ?? 'Valor'}
        type={showing ? 'text' : 'password'}
        autoComplete="new-password"
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
                variant="ghost"
                size="icon-sm"
                aria-label="Gerar uma senha"
                onClick={() =>
                  void api
                    .generatePassword(20)
                    .then((made) => {
                      onChange(made.password)
                      setShowing(true)
                    })
                    .catch((cause) => toastError(cause, 'Não gerei a senha'))
                }
              >
                <WandSparkles />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Gerar uma senha de 20 caracteres</TooltipContent>
          </Tooltip>
        )}
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={showing ? 'Esconder' : 'Mostrar'}
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
          title={`${fieldType(kind).label} (trocar o tipo)`}
          className="mt-0.5 grid size-8 place-items-center rounded-lg border bg-card text-muted-foreground hover:text-foreground"
        >
          <Icon className="size-[15px]" />
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-[560px] p-0">
        <TypeGrid
          title="Trocar o tipo"
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
            Adicionar campo
          </Button>
        ) : (
          <button
            type="button"
            className="flex h-11 w-full items-center gap-2 rounded-b-xl border-t px-5 text-sm font-medium text-muted-foreground hover:bg-hover hover:text-foreground"
          >
            <Plus className="size-4" />
            Adicionar campo
          </button>
        )}
      </PopoverTrigger>
      <PopoverContent className="w-[560px] p-0" align="start">
        <TypeGrid
          title="Adicionar campo"
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
