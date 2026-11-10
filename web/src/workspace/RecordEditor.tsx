import { useRef, useState } from 'react'
import { GripVertical, Lock, Plus, Trash2, Wand2 } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { ScrollArea } from '@/components/ui/scroll-area'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Textarea } from '@/components/ui/textarea'
import { cn } from '@/lib/utils'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'

import { PasswordInput } from '@/components/common/PasswordInput'

import { api, type Field, type Payload } from '../api'
import { RECORD_KINDS, kindOf } from './kinds'

/**
 * Writing a record down.
 *
 * **The order of the fields is the feature.** A real credential is rarely just a password — it is a
 * port, a connection string, a recovery code and three other things somebody had to write down —
 * and the sequence they are read in is part of what makes the record usable. That order lives
 * inside the encrypted payload, so it survives everything the vault does to the record afterwards.
 *
 * Which is why it is dragged rather than nudged. Arrows moved one step per click and made
 * rearranging four fields a counting exercise; a handle says "this row is a thing you can pick up",
 * and the row you drop it on is where it goes.
 */

const FIELD_KINDS: Array<[Field['kind'], string]> = [
  ['password', 'Senha'],
  ['text', 'Texto'],
  ['hidden', 'Oculto'],
  ['url', 'Endereço'],
  ['date', 'Data'],
]

interface RecordEditorProps {
  kind: string
  payload: Payload
  onKindChange?: (kind: string) => void
  onChange: (payload: Payload) => void
  onSave: () => void
  onCancel: () => void
  /** What the save button says: "Criar registro" for a new one, "Salvar alterações" otherwise. */
  saveLabel?: string
  /** Shown above the form when editing in place, where there is no dialog title. */
  heading?: React.ReactNode
  busy?: boolean
  error?: string
}

export function RecordEditor({
  kind,
  payload,
  onKindChange,
  onChange,
  onSave,
  onCancel,
  saveLabel = 'Salvar alterações',
  heading,
  busy,
  error,
}: RecordEditorProps) {
  const template = kindOf(kind)
  /**
   * Which row is being dragged, in a ref rather than in state.
   *
   * The drop handler reads this at the moment of the drop, and a state value would be whatever it
   * was when that handler was last rendered — which, if the drag started and ended without a render
   * in between, is still `null`. The state below exists only to grey out the row being moved.
   */
  const draggingFrom = useRef<number | null>(null)
  const [dragging, setDragging] = useState<number | null>(null)
  const [over, setOver] = useState<number | null>(null)

  function patch(part: Partial<Payload>) {
    onChange({ ...payload, ...part })
  }

  function setField(index: number, part: Partial<Field>) {
    patch({
      fields: payload.fields.map((field, at) => (at === index ? { ...field, ...part } : field)),
    })
  }

  function moveTo(from: number, to: number) {
    if (from === to || to < 0 || to >= payload.fields.length) {
      return
    }

    const fields = [...payload.fields]
    const [moved] = fields.splice(from, 1)
    fields.splice(to, 0, moved)

    patch({ fields })
  }

  return (
    // The footer stays put and the form scrolls under it: a long record should never push "save"
    // out of reach.
    <div className="flex h-full min-h-0 flex-col">
      <ScrollArea className="min-h-0 flex-1">
        <div className={cn('w-full space-y-6', heading ? 'max-w-3xl px-8 pt-7 pb-8' : 'px-6 pt-1 pb-6')}>
          {heading}

          {onKindChange && (
            <div className="space-y-2">
              <Label>Tipo</Label>
              <div className="grid grid-cols-5 gap-2">
                {RECORD_KINDS.map((entry) => {
                  const Icon = entry.icon
                  const chosen = entry.value === kind

                  return (
                    <button
                      key={entry.value}
                      type="button"
                      aria-pressed={chosen}
                      onClick={() => {
                        onKindChange(entry.value)
                        patch({ fields: entry.starter.map((field) => ({ ...field })) })
                      }}
                      className={cn(
                        'flex flex-col items-center gap-2 rounded-xl border px-1.5 pt-3.5 pb-3 text-xs font-semibold transition-colors hover:bg-hover',
                        chosen && 'border-ring bg-hover ring-1 ring-ring',
                      )}
                    >
                      <span className={cn('grid size-9 place-items-center rounded-lg', entry.tone)}>
                        <Icon className="size-[18px]" />
                      </span>
                      {entry.label}
                    </button>
                  )
                })}
              </div>
              <p className="text-xs text-muted-foreground">{template.hint}</p>
            </div>
          )}

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2 sm:col-span-2">
              <Label htmlFor="title">Título</Label>
              <Input
                id="title"
                value={payload.title}
                placeholder="Ex.: GitHub"
                autoFocus
                onChange={(event) => patch({ title: event.target.value })}
              />
            </div>

            <div className="space-y-2">
              <Label htmlFor="username">Usuário</Label>
              <Input
                id="username"
                value={payload.username}
                placeholder="nome@exemplo.com"
                onChange={(event) => patch({ username: event.target.value })}
              />
            </div>

            <div className="space-y-2">
              <Label htmlFor="url">Endereço</Label>
              <Input
                id="url"
                value={payload.url}
                placeholder="Ex.: github.com"
                onChange={(event) => patch({ url: event.target.value })}
              />
            </div>
          </div>

          <div className="space-y-2.5">
            <Label>Campos</Label>

            {/* Each field is a block: what it is called and what kind it is on top, the value on a
                line of its own underneath, where a long token has room. */}
            {payload.fields.map((field, index) => (
              <div
                key={index}
                onDragOver={(event) => {
                  event.preventDefault()
                  setOver(index)
                }}
                onDrop={(event) => {
                  event.preventDefault()

                  if (draggingFrom.current !== null) {
                    moveTo(draggingFrom.current, index)
                  }

                  draggingFrom.current = null
                  setDragging(null)
                  setOver(null)
                }}
                className={cn(
                  'space-y-2.5 rounded-xl border bg-surface p-3 transition-colors',
                  dragging === index && 'opacity-50',
                  over === index && dragging !== index && 'ring-2 ring-ring',
                )}
              >
                <div className="grid grid-cols-[1.25rem_minmax(0,1fr)_9.5rem_2.25rem] items-center gap-2">
                  {/* Only the handle is draggable: the inputs beside it still select text. */}
                  <span
                    draggable
                    onDragStart={(event) => {
                      event.dataTransfer.effectAllowed = 'move'
                      draggingFrom.current = index
                      setDragging(index)
                    }}
                    onDragEnd={() => {
                      draggingFrom.current = null
                      setDragging(null)
                      setOver(null)
                    }}
                    aria-label="Arrastar para reordenar"
                    className="cursor-grab text-muted-foreground active:cursor-grabbing"
                  >
                    <GripVertical className="size-4" />
                  </span>

                  <Input
                    value={field.label}
                    placeholder="Rótulo"
                    aria-label="Rótulo"
                    onChange={(event) => setField(index, { label: event.target.value })}
                  />

                  <Select
                    value={field.kind}
                    onValueChange={(value) => setField(index, { kind: value as Field['kind'] })}
                  >
                    <SelectTrigger className="w-full" aria-label="Tipo do campo">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {FIELD_KINDS.map(([value, label]) => (
                        <SelectItem key={value} value={value}>
                          {label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>

                  <IconAction
                    label="Remover campo"
                    onClick={() => patch({ fields: payload.fields.filter((_, at) => at !== index) })}
                  >
                    <Trash2 />
                  </IconAction>
                </div>

                <div className="pl-7">
                  {field.kind === 'password' || field.kind === 'hidden' ? (
                    <PasswordInput
                      value={field.value}
                      placeholder="Valor"
                      aria-label={`Valor de ${field.label || 'campo sem rótulo'}`}
                      onChange={(event) => setField(index, { value: event.target.value })}
                      trailing={
                        field.kind === 'password' && (
                          <IconAction
                            label="Gerar uma senha forte"
                            small
                            onClick={() =>
                              void api
                                .generatePassword()
                                .then((made) => setField(index, { value: made.password }))
                            }
                          >
                            <Wand2 />
                          </IconAction>
                        )
                      }
                    />
                  ) : (
                    <Input
                      value={field.value}
                      placeholder="Valor"
                      aria-label={`Valor de ${field.label || 'campo sem rótulo'}`}
                      onChange={(event) => setField(index, { value: event.target.value })}
                    />
                  )}
                </div>
              </div>
            ))}

            <Button
              type="button"
              variant="outline"
              className="border-dashed"
              onClick={() =>
                patch({ fields: [...payload.fields, { label: '', kind: 'text', value: '' }] })
              }
            >
              <Plus />
              Adicionar campo
            </Button>
          </div>

          <div className="space-y-2">
            <Label htmlFor="notes">Notas</Label>
            <Textarea
              id="notes"
              rows={4}
              value={payload.notes}
              placeholder="Qualquer coisa que ajude a lembrar depois"
              onChange={(event) => patch({ notes: event.target.value })}
            />
          </div>

          {error && <p className="text-sm text-destructive">{error}</p>}
        </div>
      </ScrollArea>

      <div className="flex shrink-0 items-center justify-end gap-2.5 border-t px-6 py-4">
        <span className="mr-auto flex items-center gap-1.5 text-xs text-muted-foreground">
          <Lock className="size-3.5" />
          Selado antes de tocar o disco
        </span>
        <Button variant="outline" onClick={onCancel}>
          Cancelar
        </Button>
        <Button onClick={onSave} disabled={busy || payload.title.trim().length === 0}>
          {saveLabel}
        </Button>
      </div>
    </div>
  )
}

function IconAction({
  label,
  small,
  onClick,
  children,
}: {
  label: string
  small?: boolean
  onClick: () => void
  children: React.ReactNode
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size={small ? 'icon-sm' : 'icon'}
          className="text-muted-foreground"
          aria-label={label}
          onClick={onClick}
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}

/**
 * A blank record of a given kind — blank meaning empty values, not missing fields.
 *
 * A new login starts with a password field already in place, because that is the field somebody
 * came here to fill in. Starting with nothing meant the first thing every new record needed was a
 * click on "Adicionar campo".
 */
export function emptyPayload(kind = 'login'): Payload {
  return {
    title: '',
    username: '',
    url: '',
    notes: '',
    fields: kindOf(kind).starter.map((field) => ({ ...field })),
  }
}
