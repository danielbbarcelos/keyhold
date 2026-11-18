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
  Share2,
  ShieldOff,
  Star,
  Trash2,
  Vault,
  Loader2,
} from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { locale, t } from '@/i18n'
import { cn } from '@/lib/utils'

import { api, ApiError, SlowError, type FolderNode, type OpenRecord, type SharedLink, type VersionEntry } from '../../api'
import { copyForSeconds } from '../../clipboard'
import { Attachments } from '../Attachments'
import { primaryField, subtitleField } from '../fields'
import { RecordTile } from '../icons'
import { MenuButton } from '../menu'
import { toast, toastError } from '../toast'
import { VersionDialog } from '../VersionDialog'
import { Activity } from './Activity'
import { RecordAlerts } from './Alerts'
import { ShareDialog } from './ShareDialog'
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

/** A `YYYY-MM-DD…` date in the order the language reads it. */
function day(iso: string): string {
  const [year, month, date] = iso.slice(0, 10).split('-').map(Number)

  return new Date(year, month - 1, date).toLocaleDateString(locale(), { day: '2-digit', month: '2-digit', year: 'numeric' })
}

export function RecordView({
  record,
  folders,
  onEdit,
  onReload,
  onClone,
  onMoveToFolder,
  onMoveToVault,
  onCopy,
  onTrash,
  permanent = false,
  onRemoveAlert,
}: {
  record: OpenRecord
  folders: FolderNode[]
  onEdit: () => void
  onReload: () => Promise<void>
  onClone: () => void
  onMoveToFolder: (folderId: string | null) => void
  onMoveToVault: () => void
  /** "Copiar para…": into any vault, AWS included. */
  onCopy: () => void
  onTrash: () => void
  /** AWS vaults have no trash: deleting is for good, and the menu says so. */
  permanent?: boolean
  /** Take one alert off the record (a new version). */
  onRemoveAlert?: (index: number) => void
}) {
  const [tab, setTab] = useState<'campos' | 'arquivos' | 'historico' | 'atividade'>('campos')
  const [versions, setVersions] = useState<VersionEntry[]>([])
  const [files, setFiles] = useState<number | null>(null)
  const [viewing, setViewing] = useState<number | null>(null)
  const [copied, setCopied] = useState(0)
  const [copying, setCopying] = useState(false)
  const [sharing, setSharing] = useState(false)
  // AWS records: links still open, when the session that would sign a new one ends, and whether
  // the bucket switched sharing off.
  const [shares, setShares] = useState<SharedLink[]>([])
  const [sessionEndsAt, setSessionEndsAt] = useState<number | null>(null)
  const [shareOff, setShareOff] = useState(false)
  const aws = Boolean(record.parameter)

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

  const loadShares = () => {
    if (!aws) return
    void api
      .recordShares(record.id)
      .then((found) => {
        setShares(found.shares)
        setSessionEndsAt(found.session_ends_at ?? null)
        setShareOff(found.share_off ?? false)
      })
      .catch(() => setShares([]))
  }

  useEffect(loadShares, [record.id, aws])

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
            <Pill icon={Folder}>{folderPath(folders, record.folder_id) || t('Raiz do cofre')}</Pill>
            <Pill icon={Clock}>{t('alterado {when}', { when: when(record.updated_at) })}</Pill>
            {record.parameter && (
              <span title={t('O nome do parâmetro na AWS')} className="inline-flex max-w-full items-center truncate rounded-full border px-2 py-0.5 font-mono text-[11px] text-muted-foreground">
                {record.parameter}
              </span>
            )}
            {record.compromised_at && (
              <Pill icon={ShieldAlert} tone="rose">
                {t('comprometido desde {date}', { date: day(record.compromised_at) })}
              </Pill>
            )}
            {record.audit?.state === 'off' && (
              <Tooltip>
                <TooltipTrigger asChild>
                  <span tabIndex={0} className="inline-flex">
                    <Pill icon={ShieldOff} tone="amber">
                      {t('sem trilha na AWS')}
                    </Pill>
                  </span>
                </TooltipTrigger>
                <TooltipContent className="max-w-xs">
                  {t('O que se faz neste registro não fica registrado na AWS: {reason}. O Keyhold funciona normalmente. Quem administra a conta resolve isso.', { reason: record.audit.reason ?? '' })}
                </TooltipContent>
              </Tooltip>
            )}
            {shares.length > 0 && (
              <span className="inline-flex items-center gap-1.5">
                <Pill icon={Share2} tone="amber">
                  {shares.length === 1 ? t('1 link ativo') : t('{n} links ativos', { n: shares.length })} ·{' '}
                  {t('expira às {time}', {
                    time: new Date(Math.min(...shares.map((one) => one.expires_at)) * 1000).toLocaleTimeString(locale(), {
                      hour: '2-digit',
                      minute: '2-digit',
                    }),
                  })}
                </Pill>
                <Button
                  variant="outline"
                  size="sm"
                  className="h-6 rounded-full px-2.5 text-xs"
                  onClick={() =>
                    void run(async () => {
                      for (const one of shares) await api.revokeShare(one.id)
                      loadShares()
                    }, shares.length === 1 ? t('Link revogado') : t('Links revogados'))
                  }
                >
                  {t('Revogar')}
                </Button>
              </span>
            )}
            {record.rotation_due_at && (
              <Pill icon={CalendarClock} tone={overdue ? 'rose' : undefined}>
                {overdue ? t('troca vencida') : t('trocar até {date}', { date: day(record.rotation_due_at) })}
              </Pill>
            )}
          </div>
        </div>
      </div>

      {(payload.alerts?.length ?? 0) > 0 && (
        <div className="px-5 pt-4">
          <RecordAlerts alerts={payload.alerts ?? []} onRemove={record.plaintext ? undefined : onRemoveAlert} />
        </div>
      )}

      <div className="flex flex-wrap items-center gap-2 px-5 pt-4 pb-4">
        {primary ? (
          <Button
            disabled={copying}
            aria-busy={copying}
            onClick={() => {
              if (copying) return
              // A sealed secret is fetched inside the click (the binary notes the copy as it hands
              // it over); one the page already has is reported as before.
              const sealedAt = primary.sealed
              // A refusal from the binary (a trail that could not be written, a session that
              // ended) is kept apart: the clipboard only reports that its write failed.
              let refused: unknown = null
              if (sealedAt !== undefined) setCopying(true)
              const value =
                sealedAt === undefined
                  ? primary.value
                  : api
                      .secret(record.id, sealedAt, 'copy')
                      .then((secret) => (primary.kind === 'card' ? secret.value.replace(/\s+/g, '') : secret.value))
                      .catch((cause) => {
                        refused = cause
                        throw cause
                      })

              void copyForSeconds(value, CLIPBOARD_SECONDS, () => setCopied(0))
                .then(() => {
                  if (sealedAt === undefined) void api.reportAccess(record.id, 'copy', primary.label).catch(() => {})
                  setCopied(CLIPBOARD_SECONDS)
                  toast(t('{label} copiado', { label: primary.label }), t('A área de transferência é limpa em {n} segundos.', { n: CLIPBOARD_SECONDS }))
                })
                .catch(() =>
                  toast(
                    refused instanceof SlowError ? t('A conexão está lenta') : t('Não foi possível copiar'),
                    refused instanceof ApiError
                      ? refused.message
                      : t('O navegador bloqueou o acesso à área de transferência nesta página.'),
                    'error',
                  ),
                )
                .finally(() => setCopying(false))
            }}
          >
            {copying ? <Loader2 className="animate-spin" /> : <Copy />}
            {copied > 0 ? t('Copiado, limpa em {n}s', { n: copied }) : t('Copiar {label}', { label: primary.label.toLowerCase() })}
          </Button>
        ) : null}
        <Button variant={primary ? 'outline' : 'default'} onClick={onEdit} disabled={record.plaintext}>
          <Pencil />
          {t('Editar')}
        </Button>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              tooltip={false}
              variant="outline"
              size="icon-lg"
              aria-label={record.favourite ? t('Remover dos favoritos') : t('Favoritar')}
              aria-pressed={record.favourite}
              onClick={() => void run(() => api.favouriteRecord(record.id, !record.favourite))}
            >
              <Star className={cn(record.favourite && 'fill-primary text-primary')} />
            </Button>
          </TooltipTrigger>
          <TooltipContent>{record.favourite ? t('Remover dos favoritos') : t('Favoritar')}</TooltipContent>
        </Tooltip>
        <span className="flex-1" />
        <MenuButton
          size="icon-lg"
          variant="outline"
          align="end"
          label={t('Mais ações')}
          entries={[
            { kind: 'item', label: t('Clonar'), icon: CopyPlus, onSelect: onClone },
            {
              kind: 'sub',
              label: t('Mover para outra pasta'),
              icon: FolderInput,
              items: [
                { label: t('Raiz do cofre'), onSelect: () => onMoveToFolder(null) },
                ...folders.map((folder) => ({
                  label: `${'  '.repeat(folder.depth)}${folder.name}`,
                  onSelect: () => onMoveToFolder(folder.id),
                })),
              ],
            },
            ...(permanent ? [] : [{ kind: 'item' as const, label: t('Mover para outro cofre'), icon: Vault, onSelect: onMoveToVault }]),
            { kind: 'item', label: t('Copiar para outro local'), icon: Copy, onSelect: onCopy },
            ...(shareOff || record.plaintext ? [] : [{ kind: 'item' as const, label: t('Compartilhar…'), icon: Share2, onSelect: () => setSharing(true) }]),
            {
              kind: 'item',
              label: record.compromised_at ? t('Remover marca de comprometido') : t('Marcar como comprometido'),
              icon: ShieldAlert,
              onSelect: () =>
                void run(
                  () => api.setCompromised(record.id, record.compromised_at === null),
                  record.compromised_at ? t('Marca de comprometido removida') : t('Marcado como comprometido'),
                ),
            },
            { kind: 'separator' },
            { kind: 'item', label: permanent ? t('Excluir permanentemente') : t('Excluir'), icon: Trash2, destructive: true, onSelect: onTrash },
          ]}
        />
      </div>

      {record.plaintext && (
        <p className="mx-5 mb-4 rounded-xl bg-amber-soft px-4 py-3 text-[13px] text-foreground">
          {t('Este parâmetro é do tipo String, sem criptografia. O Keyhold exibe o valor, mas não o altera: apenas SecureString é gravado, e mudar o tipo pode quebrar quem lê este parâmetro.')}
        </p>
      )}

      <div className="flex gap-1 border-b px-5">
        <Tab on={tab === 'campos'} count={payload.fields.length} onClick={() => setTab('campos')}>
          {t('Campos')}
        </Tab>
        <Tab on={tab === 'arquivos'} count={files ?? undefined} onClick={() => setTab('arquivos')}>
          {t('Arquivos')}
        </Tab>
        <Tab on={tab === 'historico'} count={versions.length} onClick={() => setTab('historico')}>
          {t('Histórico')}
        </Tab>
        <Tab on={tab === 'atividade'} onClick={() => setTab('atividade')}>
          {t('Atividade')}
        </Tab>
      </div>

      {tab === 'campos' && (
        <>
          <div>
            {payload.fields.map((field, index) => (
              <FieldRow
                key={`${record.id}-${record.version}-${index}`}
                field={field}
                index={index}
                recordId={record.id}
                badge={field === primary ? 'primary' : field === listed ? 'list' : undefined}
              />
            ))}
            {payload.fields.length === 0 && (
              <p className="px-5 py-6 text-sm text-faint">
                {t('Este registro não possui campos. Use Editar para adicionar campos.')}
              </p>
            )}
          </div>
          {payload.notes.trim() && (
            <div className="border-t px-5 py-4">
              <span className="text-[13px] text-muted-foreground">{t('Observação')}</span>
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
            <span className="font-medium">{t('Versão {n}', { n: record.version })}</span>
            <span className="text-faint">{t('atual, {when}', { when: when(record.updated_at) })}</span>
          </div>
          {versions.map((entry) => (
            <div key={entry.version} className="flex items-center gap-3 border-b px-5 py-3 text-sm last:border-b-0">
              <span className="font-medium">{t('Versão {n}', { n: entry.version })}</span>
              <span className="flex-1 text-faint">{t('substituída {when}', { when: when(entry.created_at) })}</span>
              <Button variant="outline" size="sm" onClick={() => setViewing(entry.version)}>
                <Eye />
                {t('Visualizar')}
              </Button>
            </div>
          ))}
          {versions.length === 0 && (
            <p className="px-5 py-6 text-sm text-faint">{t('Nenhuma versão anterior. A cada alteração salva, a versão anterior é exibida aqui.')}</p>
          )}
        </div>
      )}

      {tab === 'atividade' && <Activity recordId={record.id} />}

      {sharing && (
        <ShareDialog
          record={record}
          aws={aws}
          sessionEndsAt={sessionEndsAt}
          onClose={() => setSharing(false)}
          onShared={loadShares}
        />
      )}

      {viewing !== null && (
        <VersionDialog
          recordId={record.id}
          version={viewing}
          onCancel={() => setViewing(null)}
          onRestore={() => {
            const version = viewing
            setViewing(null)
            void run(
              () =>
                api.restoreVersion(record.id, version, permanent ? record.version : undefined).then((answer) => {
                  if (answer.warning) toast(t('Versão restaurada com um aviso'), answer.warning, 'error')
                }),
              t('Versão {n} restaurada', { n: version }),
            )
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
