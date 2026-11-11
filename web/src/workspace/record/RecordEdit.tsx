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
import { Checkbox } from '@/components/ui/checkbox'
import { awsPathOf, AwsNameField, join, leafOf, suggestName, titleProblem } from '../awsName'
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
  /** AWS: "Mudar o nome na AWS também", ticked by the vault's default. */
  rename?: boolean
  /** AWS: the name typed in "Editar nome", or `null` while it follows the title. */
  leaf?: string | null
}

/** The AWS rename a draft asks for, or `null`: only when the title changed or a name was typed. */
export function renameOf(draft: EditDraft, originalTitle: string): string | null {
  const asked = draft.leaf != null || draft.payload.title !== originalTitle
  if (!draft.rename || !asked) return null

  return draft.leaf ?? suggestName(draft.payload.title)
}

const ROOT = '__root__'

export function RecordEdit({
  draft,
  kind,
  folders,
  onChange,
  aws = false,
  parameter,
  originalTitle = '',
}: {
  draft: EditDraft
  kind?: string
  aws?: boolean
  /** AWS: the parameter's name now. */
  parameter?: string
  originalTitle?: string
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
              aria-invalid={aws && titleProblem(draft.payload.title) !== null}
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
            {aws && <span className="text-xs text-faint">Mudar a pasta na AWS apaga o histórico do nome antigo.</span>}
          </label>
          <div className="grid w-[190px] gap-1.5">
            <span className="text-[13px] font-medium">Trocar até</span>
            <DatePicker
              value={draft.rotation?.slice(0, 10) ?? ''}
              onChange={(value) => onChange({ ...draft, rotation: value || null })}
            />
          </div>
        </div>
        {aws && parameter && <AwsName draft={draft} parameter={parameter} originalTitle={originalTitle} onChange={onChange} />}
      </Card>

      <Card title="Campos" hint="Arraste para reordenar">
        <FieldEditor framed={false} fields={draft.payload.fields} onChange={(fields) => setPayload({ fields })} />
      </Card>

      <Card title="Observação">
        <div className="px-5 pt-4 pb-5">
          <Textarea
            value={draft.payload.notes}
            placeholder="Qualquer coisa que ajude a lembrar depois"
              spellCheck={false}
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

/**
 * The parameter's name under the title. It stays as it is unless asked: a changed title offers
 * "Mudar o nome na AWS também" (ticked or not by the vault's default), and "Editar nome" asks
 * directly. Moving it is a new parameter, so the old one goes with its history.
 */
function AwsName({
  draft,
  parameter,
  originalTitle,
  onChange,
}: {
  draft: EditDraft
  parameter: string
  originalTitle: string
  onChange: (draft: EditDraft) => void
}) {
  const titleWrong = titleProblem(draft.payload.title)
  const asked = draft.leaf != null || draft.payload.title !== originalTitle
  const parent = awsPathOf(draft.folderId)
  const now = join(parent, leafOf(parameter))

  return (
    <div className="grid gap-2 border-t px-5 py-3.5">
      {titleWrong && <p className="text-xs text-destructive">{titleWrong}</p>}
      {!asked ? (
        <span className="flex flex-wrap items-center gap-x-2 text-xs text-faint">
          <span>
            Na AWS: <code className="font-mono text-muted-foreground">{now}</code>
          </span>
          <button
            type="button"
            className="text-primary hover:underline"
            onClick={() => onChange({ ...draft, rename: true, leaf: leafOf(parameter) })}
          >
            Editar nome
          </button>
        </span>
      ) : (
        <>
          <label className="flex items-center gap-2 text-[13px]">
            <Checkbox
              checked={draft.rename ?? false}
              onCheckedChange={(on) => onChange({ ...draft, rename: on === true })}
            />
            Mudar o nome na AWS também
          </label>
          {draft.rename ? (
            <>
              <AwsNameField
                parent={parent}
                from={draft.payload.title}
                value={draft.leaf ?? null}
                onChange={(leaf) => onChange({ ...draft, leaf })}
              />
              <span className="text-xs text-amber">
                O Keyhold grava um parâmetro novo e apaga {parameter}, com o histórico dele. Quem lê o nome
                antigo para de achar.
              </span>
            </>
          ) : (
            <span className="text-xs text-faint">
              Na AWS continua <code className="font-mono text-muted-foreground">{now}</code>. Só o título muda.
            </span>
          )}
        </>
      )}
    </div>
  )
}
