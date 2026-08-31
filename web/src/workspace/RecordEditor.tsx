import { useRef, useState } from 'react'
import { GripVertical, Plus, Trash2, Wand2 } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
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
    <div className="w-full space-y-6 p-6">
      {onKindChange && (
        <div className="space-y-2">
          <Label>Tipo</Label>
          <div className="flex flex-wrap gap-1.5">
            {RECORD_KINDS.map((entry) => {
              const Icon = entry.icon

              return (
                <Button
                  key={entry.value}
                  type="button"
                  size="sm"
                  variant={entry.value === kind ? 'default' : 'outline'}
                  onClick={() => {
                    onKindChange(entry.value)
                    patch({ fields: entry.starter.map((field) => ({ ...field })) })
                  }}
                >
                  <Icon />
                  {entry.label}
                </Button>
              )
            })}
          </div>
          <p className="text-sm text-muted-foreground">{template.hint}</p>
        </div>
      )}

      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-2 sm:col-span-2">
          <Label htmlFor="title">Título</Label>
          <Input
            id="title"
            value={payload.title}
            autoFocus
            onChange={(event) => patch({ title: event.target.value })}
          />
        </div>

        <div className="space-y-2">
          <Label htmlFor="username">Usuário</Label>
          <Input
            id="username"
            value={payload.username}
            onChange={(event) => patch({ username: event.target.value })}
          />
        </div>

        <div className="space-y-2">
          <Label htmlFor="url">Endereço</Label>
          <Input
            id="url"
            value={payload.url}
            onChange={(event) => patch({ url: event.target.value })}
          />
        </div>
      </div>

      <div className="space-y-2">
        <Label>Campos</Label>

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
              'flex items-center gap-1.5 rounded-md transition-colors',
              dragging === index && 'opacity-50',
              over === index && dragging !== index && 'ring-1 ring-primary',
            )}
          >
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
              className="cursor-grab px-0.5 text-muted-foreground active:cursor-grabbing"
            >
              <GripVertical className="size-4" />
            </span>

            <Input
              className="w-40"
              value={field.label}
              placeholder="Rótulo"
              onChange={(event) => setField(index, { label: event.target.value })}
            />

            <Select
              value={field.kind}
              onValueChange={(value) => setField(index, { kind: value as Field['kind'] })}
            >
              <SelectTrigger className="w-32">
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

            {field.kind === 'password' || field.kind === 'hidden' ? (
              <PasswordInput
                value={field.value}
                placeholder="Valor"
                onChange={(event) => setField(index, { value: event.target.value })}
              />
            ) : (
              <Input
                className="flex-1"
                value={field.value}
                placeholder="Valor"
                onChange={(event) => setField(index, { value: event.target.value })}
              />
            )}

            {field.kind === 'password' && (
              <IconAction
                label="Gerar uma senha forte"
                onClick={() =>
                  void api.generatePassword().then((made) => setField(index, { value: made.password }))
                }
              >
                <Wand2 />
              </IconAction>
            )}

            <IconAction
              label="Remover"
              onClick={() => patch({ fields: payload.fields.filter((_, at) => at !== index) })}
            >
              <Trash2 />
            </IconAction>
          </div>
        ))}

        <Button
          type="button"
          variant="outline"
          size="sm"
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
          onChange={(event) => patch({ notes: event.target.value })}
        />
      </div>

      {error && <p className="text-sm text-destructive">{error}</p>}

      <div className="flex items-center gap-2">
        <Button onClick={onSave} disabled={busy || payload.title.trim().length === 0}>
          Salvar
        </Button>
        <Button variant="ghost" onClick={onCancel}>
          Descartar
        </Button>
      </div>
    </div>
  )
}

function IconAction({
  label,
  onClick,
  children,
}: {
  label: string
  onClick: () => void
  children: React.ReactNode
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button type="button" variant="ghost" size="icon-sm" aria-label={label} onClick={onClick}>
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
