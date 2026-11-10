import { useState } from 'react'
import { FileText, Folder, KeyRound, CreditCard, SquareDashed, Terminal } from 'lucide-react'
import type { LucideIcon } from 'lucide-react'

import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { cn } from '@/lib/utils'

import type { Payload } from '../../api'
import { fieldsOf, TEMPLATES } from '../fields'
import { IconPicker } from '../IconPicker'
import { FieldEditor } from './FieldEditor'

/**
 * A new record: a title, an icon, and the fields it needs.
 *
 * It starts empty. The templates are shortcuts that add a set of fields at once (Login adds e-mail,
 * password, address and 2FA); they do not give the record a type, and every field they add can be
 * removed. Switching template replaces only fields nobody has filled in yet.
 */
const TEMPLATE_ICONS: Record<string, LucideIcon> = {
  login: KeyRound,
  cartao: CreditCard,
  ssh: Terminal,
  nota: FileText,
}

export function NewRecordDialog({
  initial,
  where,
  cloning,
  onCreate,
  onCancel,
}: {
  initial: Payload
  where: string
  cloning?: boolean
  onCreate: (payload: Payload) => Promise<void>
  onCancel: () => void
}) {
  const [payload, setPayload] = useState<Payload>(initial)
  const [template, setTemplate] = useState<string>(cloning ? '' : 'branco')
  const [busy, setBusy] = useState(false)

  function pick(id: string) {
    const chosen = TEMPLATES.find((one) => one.id === id)
    const filled = payload.fields.filter((field) => field.value.trim() !== '' || Object.values(field.extra ?? {}).some(Boolean))

    setTemplate(id)
    setPayload({
      ...payload,
      fields: [...filled, ...(chosen ? fieldsOf(chosen) : [])],
      icon: payload.icon ?? (chosen ? { name: chosen.icon, colour: chosen.colour } : null),
    })
  }

  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="max-h-[calc(100vh-48px)] gap-0 overflow-hidden p-0 sm:max-w-[640px]">
        <DialogHeader className="border-b px-6 pt-5 pb-4">
          <DialogTitle className="text-lg">{cloning ? 'Clonar registro' : 'Novo registro'}</DialogTitle>
          <DialogDescription>
            Só o título e um campo de identificação aparecem na lista. O resto fica selado.
          </DialogDescription>
        </DialogHeader>

        <div className="min-h-0 space-y-5 overflow-y-auto px-6 py-5">
          <div className="flex items-end gap-3">
            <label className="grid gap-1.5">
              <span className="text-[13px] font-medium">Ícone</span>
              <IconPicker icon={payload.icon} onChange={(icon) => setPayload({ ...payload, icon })} />
            </label>
            <label className="grid flex-1 gap-1.5">
              <span className="text-[13px] font-medium">Título</span>
              <Input
                autoFocus
                value={payload.title}
                placeholder="Ex.: GitHub"
                onChange={(event) => setPayload({ ...payload, title: event.target.value })}
              />
            </label>
          </div>

          {!cloning && (
            <div className="grid gap-1.5">
              <span className="text-[13px] font-medium">Começar com</span>
              <div className="flex flex-wrap gap-1.5">
                <Chip on={template === 'branco'} icon={SquareDashed} onClick={() => pick('branco')}>
                  Em branco
                </Chip>
                {TEMPLATES.map((one) => (
                  <Chip key={one.id} on={template === one.id} icon={TEMPLATE_ICONS[one.id]} onClick={() => pick(one.id)}>
                    {one.label}
                  </Chip>
                ))}
              </div>
            </div>
          )}

          <div className="grid gap-1.5">
            <span className="text-[13px] font-medium">Campos</span>
            <FieldEditor fields={payload.fields} onChange={(fields) => setPayload({ ...payload, fields })} />
          </div>

          <label className="grid gap-1.5">
            <span className="text-[13px] font-medium">Observação</span>
            <Textarea
              className="min-h-[72px]"
              value={payload.notes}
              placeholder="Qualquer coisa que ajude a lembrar depois"
              onChange={(event) => setPayload({ ...payload, notes: event.target.value })}
            />
          </label>
        </div>

        <DialogFooter className="m-0 flex-row items-center rounded-b-2xl border-t px-6 py-4 sm:justify-start">
          <span className="mr-auto flex min-w-0 items-center gap-1.5 truncate text-xs text-faint">
            <Folder className="size-3.5 shrink-0" />
            {where}
          </span>
          <Button variant="outline" onClick={onCancel}>
            Cancelar
          </Button>
          <Button
            disabled={!payload.title.trim() || busy}
            onClick={() => {
              setBusy(true)
              void onCreate({
                ...payload,
                fields: payload.fields.filter(
                  (field) => field.value.trim() !== '' || Object.values(field.extra ?? {}).some((part) => part.trim() !== ''),
                ),
              }).finally(() => setBusy(false))
            }}
          >
            Criar registro
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function Chip({
  on,
  icon: Icon,
  onClick,
  children,
}: {
  on: boolean
  icon: LucideIcon
  onClick: () => void
  children: React.ReactNode
}) {
  return (
    <button
      type="button"
      aria-pressed={on}
      onClick={onClick}
      className={cn(
        'inline-flex h-[30px] items-center gap-1.5 rounded-full border px-3 text-[13px] font-medium',
        on ? 'border-transparent bg-primary-soft text-primary-text' : 'text-muted-foreground hover:bg-hover hover:text-foreground',
      )}
    >
      <Icon className="size-3.5" />
      {children}
    </button>
  )
}
