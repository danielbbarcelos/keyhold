import { useEffect, useState } from 'react'
import {
  CalendarClock,
  Clock,
  Copy,
  CopyPlus,
  Eye,
  Folder,
  FolderInput,
  Pencil,
  ShieldAlert,
  Star,
  Trash2,
  Vault,
} from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'

import { api, type FolderNode, type OpenRecord, type VersionEntry } from '../../api'
import { copyForSeconds } from '../../clipboard'
import { Attachments } from '../Attachments'
import { primaryField, subtitleField } from '../fields'
import { RecordTile } from '../icons'
import { MenuButton } from '../menu'
import { toast, toastError } from '../toast'
import { VersionDialog } from '../VersionDialog'
import { FieldRow } from './FieldRow'
import { folderPath, when } from '../vault/paths'

/**
 * One open record: what it holds, and the few things that can be done to it.
 *
 * One action in colour: copying the main secret is what a vault is opened for, nine times in ten.
 * Editing and favouriting sit beside it; everything rarer, and everything that removes, goes behind
 * the menu, away from a hurried click.
 */
const CLIPBOARD_SECONDS = 30

export function RecordView({
  record,
  folders,
  onEdit,
  onReload,
  onClone,
  onMoveToFolder,
  onMoveToVault,
  onTrash,
}: {
  record: OpenRecord
  folders: FolderNode[]
  onEdit: () => void
  onReload: () => Promise<void>
  onClone: () => void
  onMoveToFolder: (folderId: string | null) => void
  onMoveToVault: () => void
  onTrash: () => void
}) {
  const [tab, setTab] = useState<'campos' | 'arquivos' | 'historico'>('campos')
  const [versions, setVersions] = useState<VersionEntry[]>([])
  const [files, setFiles] = useState<number | null>(null)
  const [viewing, setViewing] = useState<number | null>(null)
  const [copied, setCopied] = useState(0)

  const { payload } = record
  const primary = primaryField(payload.fields)
  const listed = subtitleField(payload.fields)

  useEffect(() => {
    setTab('campos')
  }, [record.id])

  useEffect(() => {
    void api.versions(record.id).then((found) => setVersions(found.versions)).catch(() => setVersions([]))
    void api.attachments(record.id).then((found) => setFiles(found.attachments.length)).catch(() => setFiles(null))
  }, [record.id, record.version])

  useEffect(() => {
    if (copied <= 0) return
    const tick = window.setInterval(() => setCopied((left) => Math.max(0, left - 1)), 1000)

    return () => window.clearInterval(tick)
  }, [copied])

  async function run(action: () => Promise<unknown>, done?: string) {
    try {
      await action()
      await onReload()
      if (done) toast(done)
    } catch (cause) {
      toastError(cause)
    }
  }

  const overdue = record.rotation_due_at !== null && new Date(record.rotation_due_at) <= new Date()

  return (
    <div className="flex h-full min-h-0 flex-col overflow-auto rounded-[14px] border bg-card">
      <div className="flex items-start gap-4 px-5 pt-5">
        <RecordTile icon={payload.icon} kind={record.kind} size="lg" />
        <div className="min-w-0 flex-1">
          <h2 className="truncate text-xl font-semibold tracking-tight">{payload.title}</h2>
          <div className="mt-1.5 flex flex-wrap gap-1.5">
            <Pill icon={Folder}>{folderPath(folders, record.folder_id) || 'Raiz do cofre'}</Pill>
            <Pill icon={Clock}>alterado {when(record.updated_at)}</Pill>
            {record.compromised_at && (
              <Pill icon={ShieldAlert} tone="rose">
                comprometido desde {record.compromised_at.slice(0, 10).split('-').reverse().join('/')}
              </Pill>
            )}
            {record.rotation_due_at && (
              <Pill icon={CalendarClock} tone={overdue ? 'rose' : undefined}>
                {overdue ? 'troca vencida' : `trocar até ${record.rotation_due_at.slice(0, 10).split('-').reverse().join('/')}`}
              </Pill>
            )}
          </div>
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-2 px-5 pt-4 pb-4">
        {primary ? (
          <Button
            onClick={() => {
              void api.reportAccess(record.id, 'copy', primary.label).catch(() => {})
              void copyForSeconds(primary.value, CLIPBOARD_SECONDS, () => setCopied(0)).then(() => {
                setCopied(CLIPBOARD_SECONDS)
                toast(`${primary.label} copiado`, 'A área de transferência é limpa em 30 segundos.')
              })
            }}
          >
            <Copy />
            {copied > 0 ? `Copiado, limpa em ${copied}s` : `Copiar ${primary.label.toLowerCase()}`}
          </Button>
        ) : null}
        <Button variant={primary ? 'outline' : 'default'} onClick={onEdit}>
          <Pencil />
          Editar
        </Button>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="outline"
              size="icon-lg"
              aria-label={record.favourite ? 'Tirar dos favoritos' : 'Favoritar'}
              aria-pressed={record.favourite}
              onClick={() => void run(() => api.favouriteRecord(record.id, !record.favourite))}
            >
              <Star className={cn(record.favourite && 'fill-primary text-primary')} />
            </Button>
          </TooltipTrigger>
          <TooltipContent>{record.favourite ? 'Tirar dos favoritos' : 'Favoritar'}</TooltipContent>
        </Tooltip>
        <span className="flex-1" />
        <MenuButton
          size="icon-lg"
          variant="outline"
          align="end"
          label="Mais ações"
          entries={[
            { kind: 'item', label: 'Clonar', icon: CopyPlus, onSelect: onClone },
            {
              kind: 'sub',
              label: 'Mover para outra pasta',
              icon: FolderInput,
              items: [
                { label: 'Raiz do cofre', onSelect: () => onMoveToFolder(null) },
                ...folders.map((folder) => ({
                  label: `${'  '.repeat(folder.depth)}${folder.name}`,
                  onSelect: () => onMoveToFolder(folder.id),
                })),
              ],
            },
            { kind: 'item', label: 'Mover para outro cofre', icon: Vault, onSelect: onMoveToVault },
            {
              kind: 'item',
              label: record.compromised_at ? 'Não está mais comprometido' : 'Marcar como comprometido',
              icon: ShieldAlert,
              onSelect: () =>
                void run(
                  () => api.setCompromised(record.id, record.compromised_at === null),
                  record.compromised_at ? 'Marca de comprometido retirada' : 'Marcado como comprometido',
                ),
            },
            { kind: 'separator' },
            { kind: 'item', label: 'Mandar para a lixeira', icon: Trash2, destructive: true, onSelect: onTrash },
          ]}
        />
      </div>

      <div className="flex gap-1 border-b px-5">
        <Tab on={tab === 'campos'} count={payload.fields.length} onClick={() => setTab('campos')}>
          Campos
        </Tab>
        <Tab on={tab === 'arquivos'} count={files ?? undefined} onClick={() => setTab('arquivos')}>
          Arquivos
        </Tab>
        <Tab on={tab === 'historico'} count={versions.length} onClick={() => setTab('historico')}>
          Histórico
        </Tab>
      </div>

      {tab === 'campos' && (
        <>
          <div>
            {payload.fields.map((field, index) => (
              <FieldRow
                key={`${record.version}-${index}`}
                field={field}
                index={index}
                recordId={record.id}
                badge={field === primary ? 'primary' : field === listed ? 'list' : undefined}
              />
            ))}
            {payload.fields.length === 0 && (
              <p className="px-5 py-6 text-sm text-faint">
                Este registro ainda não tem campos. Em Editar, "Adicionar campo" mostra os tipos.
              </p>
            )}
          </div>
          {payload.notes.trim() && (
            <div className="border-t px-5 py-4">
              <span className="text-[13px] text-muted-foreground">Observação</span>
              <p className="mt-2 text-sm whitespace-pre-wrap">{payload.notes}</p>
            </div>
          )}
        </>
      )}

      {tab === 'arquivos' && (
        <div className="p-5">
          <Attachments recordId={record.id} />
        </div>
      )}

      {tab === 'historico' && (
        <div>
          <div className="flex items-center gap-3 border-b px-5 py-3 text-sm">
            <span className="font-medium">Versão {record.version}</span>
            <span className="text-faint">atual, {when(record.updated_at)}</span>
          </div>
          {versions.map((entry) => (
            <div key={entry.version} className="flex items-center gap-3 border-b px-5 py-3 text-sm last:border-b-0">
              <span className="font-medium">Versão {entry.version}</span>
              <span className="flex-1 text-faint">substituída {when(entry.created_at)}</span>
              <Button variant="outline" size="sm" onClick={() => setViewing(entry.version)}>
                <Eye />
                Ver
              </Button>
            </div>
          ))}
          {versions.length === 0 && (
            <p className="px-5 py-6 text-sm text-faint">Nenhuma versão anterior. Cada vez que você salva, a de antes fica aqui.</p>
          )}
        </div>
      )}

      {viewing !== null && (
        <VersionDialog
          recordId={record.id}
          version={viewing}
          onCancel={() => setViewing(null)}
          onRestore={() => {
            const version = viewing
            setViewing(null)
            void run(() => api.restoreVersion(record.id, version), `Versão ${version} restaurada`)
          }}
        />
      )}
    </div>
  )
}

function Tab({
  on,
  count,
  onClick,
  children,
}: {
  on: boolean
  count?: number
  onClick: () => void
  children: React.ReactNode
}) {
  return (
    <button
      type="button"
      aria-current={on}
      onClick={onClick}
      className={cn(
        '-mb-px flex h-10 items-center gap-1.5 border-b-2 px-2.5 text-sm font-medium',
        on ? 'border-primary text-foreground' : 'border-transparent text-muted-foreground hover:text-foreground',
      )}
    >
      {children}
      {count !== undefined && <span className="text-xs text-faint">{count}</span>}
    </button>
  )
}

export function Pill({
  icon: Icon,
  tone,
  children,
}: {
  icon?: React.ComponentType<{ className?: string }>
  tone?: 'rose' | 'amber'
  children: React.ReactNode
}) {
  return (
    <span
      className={cn(
        'inline-flex h-6 items-center gap-1.5 rounded-full px-2.5 text-xs',
        tone === 'rose' ? 'bg-rose-soft text-rose' : tone === 'amber' ? 'bg-amber-soft text-amber' : 'bg-stone-soft text-muted-foreground',
      )}
    >
      {Icon && <Icon className="size-3" />}
      {children}
    </span>
  )
}
