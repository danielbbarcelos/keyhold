import { DatePicker } from '@/components/common/DatePicker'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'

import type { FolderNode, Payload } from '../../api'
import { IconPicker } from '../IconPicker'
import { FieldEditor } from './FieldEditor'

/**
 * A record being written, as a page rather than a dialog: the tree stays on the left, the record
 * gets the room it needs, and the save and cancel buttons sit in the top bar where they cannot be
 * scrolled away.
 */
export interface EditDraft {
  payload: Payload
  folderId: string | null
  rotation: string | null
}

const ROOT = '__root__'

export function RecordEdit({
  draft,
  kind,
  folders,
  onChange,
}: {
  draft: EditDraft
  kind?: string
  folders: FolderNode[]
  onChange: (draft: EditDraft) => void
}) {
  const setPayload = (change: Partial<Payload>) => onChange({ ...draft, payload: { ...draft.payload, ...change } })

  return (
    <div className="space-y-5 pb-6">
      <Card title="Registro">
        <div className="flex flex-wrap items-end gap-4 px-5 pt-4 pb-5">
          <label className="grid gap-1.5">
            <span className="text-[13px] font-medium">Ícone</span>
            <IconPicker icon={draft.payload.icon} kind={kind} onChange={(icon) => setPayload({ icon })} />
          </label>
          <label className="grid min-w-[220px] flex-1 gap-1.5">
            <span className="text-[13px] font-medium">Título</span>
            <Input
              autoFocus
              value={draft.payload.title}
              placeholder="Ex.: GitHub"
              onChange={(event) => setPayload({ title: event.target.value })}
            />
          </label>
          <label className="grid w-[220px] gap-1.5">
            <span className="text-[13px] font-medium">Pasta</span>
            <Select
              value={draft.folderId ?? ROOT}
              onValueChange={(value) => onChange({ ...draft, folderId: value === ROOT ? null : value })}
            >
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={ROOT}>Raiz do cofre</SelectItem>
                {folders.map((folder) => (
                  <SelectItem key={folder.id} value={folder.id}>
                    {'  '.repeat(folder.depth)}
                    {folder.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </label>
          <div className="grid w-[190px] gap-1.5">
            <span className="text-[13px] font-medium">Trocar até</span>
            <DatePicker
              value={draft.rotation?.slice(0, 10) ?? ''}
              onChange={(value) => onChange({ ...draft, rotation: value || null })}
            />
          </div>
        </div>
      </Card>

      <Card title="Campos" hint="Arraste para reordenar">
        <FieldEditor framed={false} fields={draft.payload.fields} onChange={(fields) => setPayload({ fields })} />
      </Card>

      <Card title="Observação">
        <div className="px-5 pt-4 pb-5">
          <Textarea
            value={draft.payload.notes}
            placeholder="Qualquer coisa que ajude a lembrar depois"
            onChange={(event) => setPayload({ notes: event.target.value })}
          />
        </div>
      </Card>
    </div>
  )
}

function Card({ title, hint, children }: { title: string; hint?: string; children: React.ReactNode }) {
  return (
    <section className="rounded-[14px] border bg-card">
      <div className="flex items-center border-b px-5 py-3.5">
        <h3 className="flex-1 text-[15px] font-semibold">{title}</h3>
        {hint && <span className="text-xs text-faint">{hint}</span>}
      </div>
      {children}
    </section>
  )
}
