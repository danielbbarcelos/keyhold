import { useEffect, useState } from 'react'
import {
  Check,
  Copy,
  CopyPlus,
  EllipsisVertical,
  ExternalLink,
  FolderInput,
  History,
  PanelRightClose,
  PanelRightOpen,
  Pencil,
  ShieldAlert,
  Star,
  Trash2,
  X,
} from 'lucide-react'

import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { ScrollArea } from '@/components/ui/scroll-area'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'

import { api, ApiError, type FolderNode, type OpenRecord, type VersionEntry } from '../api'
import { Input } from '@/components/ui/input'
import { copyForSeconds } from '../clipboard'

import { Attachments } from './Attachments'
import { RecordEditor } from './RecordEditor'
import { SecretLine } from './SecretLine'
import { VersionDialog } from './VersionDialog'
import { kindOf, kindTone } from './kinds'

/**
 * One open record: what it holds, and what is true about it.
 *
 * Reading and editing are separate states rather than a form that is always live. A vault is read
 * far more often than it is written, and a page of editable inputs invites a stray keystroke into a
 * value nobody can recover.
 */
interface RecordTabProps {
  record: OpenRecord
  folders: FolderNode[]
  onReload: (id: string) => Promise<void>
  onClone: (id: string) => void
  onMoveToVault: () => void
  onClosed: (id: string) => void
}

/** Below this width the details panel would squeeze the record itself, so it starts closed. */
const WIDE_ENOUGH = '(min-width: 1280px)'

export function RecordTab({
  record,
  folders,
  onReload,
  onClone,
  onMoveToVault,
  onClosed,
}: RecordTabProps) {
  const [editing, setEditing] = useState(false)
  const [propertiesOpen, setPropertiesOpen] = useState(() => window.matchMedia(WIDE_ENOUGH).matches)
  const [copied, setCopied] = useState(0)
  const [draft, setDraft] = useState(record.payload)
  const [versions, setVersions] = useState<VersionEntry[]>([])
  const [viewing, setViewing] = useState<number | null>(null)
  const [error, setError] = useState('')
  const [confirmingDelete, setConfirmingDelete] = useState(false)

  useEffect(() => {
    setDraft(record.payload)
    setEditing(false)
    void api
      .versions(record.id)
      .then((found) => setVersions(found.versions))
      .catch(() => setVersions([]))
  }, [record.id, record.version, record.payload])

  // Follow the window: narrowing it closes the panel, widening it opens it again.
  useEffect(() => {
    const query = window.matchMedia(WIDE_ENOUGH)
    const follow = () => setPropertiesOpen(query.matches)

    query.addEventListener('change', follow)

    return () => query.removeEventListener('change', follow)
  }, [])

  useEffect(() => {
    if (copied <= 0) {
      return
    }

    const tick = window.setInterval(() => setCopied((left) => Math.max(0, left - 1)), 1000)

    return () => window.clearInterval(tick)
  }, [copied])

  async function run(action: () => Promise<unknown>) {
    try {
      await action()
      await onReload(record.id)
      setError('')
    } catch (cause) {
      setError(cause instanceof ApiError ? cause.message : 'Não foi possível falar com o Keyhold.')
    }
  }

  if (editing) {
    return (
      <RecordEditor
        heading={
          <header className="flex items-center gap-4">
            <span className={cn('grid size-12 shrink-0 place-items-center rounded-xl', kindTone(record.kind))}>
              <KindIconOf kind={record.kind} />
            </span>
            <div className="min-w-0">
              <h2 className="truncate text-[1.375rem] leading-7 font-bold tracking-tight">
                Editar {record.payload.title}
              </h2>
              <p className="text-sm text-muted-foreground">{kindOf(record.kind).label}</p>
            </div>
          </header>
        }
        kind={record.kind}
        payload={draft}
        onChange={setDraft}
        onSave={() => void run(() => api.saveRecord(record.id, draft))}
        onCancel={() => {
          setDraft(record.payload)
          setEditing(false)
        }}
        error={error}
      />
    )
  }

  const link = safeHref(record.payload.url)
  // The value behind the main button: the first password, or failing that the first hidden field.
  const secret =
    record.payload.fields.find((field) => field.kind === 'password' && field.value) ??
    record.payload.fields.find((field) => field.kind === 'hidden' && field.value)
  const kind = kindOf(record.kind)
  const KindIcon = kind.icon

  return (
    <div
      className={cn(
        'grid h-full min-h-0',
        // The panel is a drawer, not a margin: open it is wide enough to read a folder name and a
        // date without wrapping, and closed it takes no room at all.
        propertiesOpen ? 'grid-cols-[minmax(0,1fr)_21rem]' : 'grid-cols-[minmax(0,1fr)_0]',
      )}
    >
      <ScrollArea className="h-full min-w-0">
        <div className="max-w-3xl px-8 pt-7 pb-10">
          <header className="flex items-start gap-4">
            <span className={cn('grid size-12 shrink-0 place-items-center rounded-xl', kindTone(record.kind))}>
              <KindIcon className="size-6" />
            </span>

            <div className="min-w-0 flex-1">
              <h2 className="flex min-w-0 items-center gap-2 text-[1.375rem] leading-7 font-bold tracking-tight">
                <span className="truncate">{record.payload.title}</span>
                {record.favourite && <Star className="size-4 shrink-0 fill-current text-primary" />}
              </h2>

              {link ? (
                <a
                  href={link}
                  target="_blank"
                  rel="noreferrer noopener"
                  className="mt-1 inline-flex max-w-full items-center gap-1.5 text-sm text-link hover:underline"
                >
                  <span className="truncate">{record.payload.url}</span>
                  <ExternalLink className="size-3.5 shrink-0" />
                </a>
              ) : (
                record.payload.url && (
                  <p className="mt-1 truncate text-sm text-muted-foreground">{record.payload.url}</p>
                )
              )}
            </div>

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={propertiesOpen ? 'Esconder detalhes' : 'Mostrar detalhes'}
                  aria-expanded={propertiesOpen}
                  onClick={() => setPropertiesOpen(!propertiesOpen)}
                >
                  {propertiesOpen ? <PanelRightClose /> : <PanelRightOpen />}
                </Button>
              </TooltipTrigger>
              <TooltipContent>{propertiesOpen ? 'Esconder detalhes' : 'Mostrar detalhes'}</TooltipContent>
            </Tooltip>
          </header>

          {record.compromised_at && (
            <Badge variant="destructive" className="mt-4 gap-1">
              <ShieldAlert className="size-3" />
              Comprometido desde {record.compromised_at.slice(0, 10)}
            </Badge>
          )}

          {/*
            * One action in colour: copying the password is what a vault is opened for, nine times in
            * ten. Editing and favouriting sit beside it; everything rarer, and everything that
            * removes, goes behind the menu, away from a hurried click.
            */}
          <div className="mt-5 mb-6 flex flex-wrap items-center gap-2">
            {secret ? (
              <Button
                onClick={() => {
                  void api.reportAccess(record.id, 'copy', secret.label).catch(() => {})
                  void copyForSeconds(secret.value, CLIPBOARD_SECONDS, () => setCopied(0)).then(() =>
                    setCopied(CLIPBOARD_SECONDS),
                  )
                }}
              >
                {copied > 0 ? <Check /> : <Copy />}
                {copied > 0 ? `Copiado, limpa em ${copied}s` : `Copiar ${secret.label.toLowerCase()}`}
              </Button>
            ) : (
              <Button onClick={() => setEditing(true)}>
                <Pencil />
                Editar
              </Button>
            )}

            {secret && (
              <Button variant="outline" onClick={() => setEditing(true)}>
                <Pencil />
                Editar
              </Button>
            )}

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="outline"
                  size="icon"
                  className="size-9"
                  aria-label={record.favourite ? 'Tirar dos favoritos' : 'Favoritar'}
                  aria-pressed={record.favourite}
                  onClick={() => void run(() => api.favouriteRecord(record.id, !record.favourite))}
                >
                  <Star className={record.favourite ? 'fill-current text-primary' : undefined} />
                </Button>
              </TooltipTrigger>
              <TooltipContent>{record.favourite ? 'Tirar dos favoritos' : 'Favoritar'}</TooltipContent>
            </Tooltip>

            <span className="flex-1" />

            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="outline" size="icon" className="size-9" aria-label="Mais ações">
                  <EllipsisVertical />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-64">
                <DropdownMenuItem onSelect={() => onClone(record.id)}>
                  <CopyPlus />
                  Clonar
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={onMoveToVault}>
                  <FolderInput />
                  Mover para outro cofre
                </DropdownMenuItem>
                {versions.length > 0 && (
                  <DropdownMenuItem onSelect={() => setViewing(versions[0].version)}>
                    <History />
                    Versões anteriores
                  </DropdownMenuItem>
                )}
                <DropdownMenuItem
                  onSelect={() =>
                    void run(() => api.setCompromised(record.id, record.compromised_at === null))
                  }
                >
                  <ShieldAlert />
                  {record.compromised_at ? 'Não está mais comprometido' : 'Marcar como comprometido'}
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem variant="destructive" onSelect={() => setConfirmingDelete(true)}>
                  <Trash2 />
                  Mandar para a lixeira
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>

          <div className="overflow-hidden rounded-xl border bg-surface">
            {record.payload.username && (
              <SecretLine label="Usuário" value={record.payload.username} masked={false} />
            )}

            {record.payload.fields.map((field, index) => (
              <SecretLine
                key={index}
                label={field.label || 'Sem rótulo'}
                value={field.value}
                masked={field.kind === 'password' || field.kind === 'hidden'}
                recordId={record.id}
              />
            ))}

            {record.payload.username === '' && record.payload.fields.length === 0 && (
              <p className="px-4 py-3 text-sm text-muted-foreground">Este registro não tem campos.</p>
            )}
          </div>

          {record.payload.notes && (
            <>
              <h3 className="mt-7 mb-2 text-sm font-bold">Notas</h3>
              <p className="rounded-xl border bg-surface px-4 py-3.5 text-sm whitespace-pre-wrap">
                {record.payload.notes}
              </p>
            </>
          )}

          {error && <p className="mt-4 text-sm text-destructive">{error}</p>}
        </div>
      </ScrollArea>

      {/*
        * Always rendered, never unmounted: the panel's border is what separates the two halves of
        * this screen, and a border that disappears with its contents leaves the divider stopping
        * partway down the page. Closed, it is a zero-width column that still draws its edge.
        */}
      <ScrollArea className={cn('h-full min-w-0 border-l', !propertiesOpen && 'overflow-hidden')}>
        <aside className={cn('space-y-5 px-5 py-6', !propertiesOpen && 'invisible')}>
          <h3 className="text-sm font-bold">Detalhes</h3>

          <dl className="space-y-4 text-sm">
            <div>
              <dt className="mb-1.5 text-xs font-semibold text-muted-foreground">Tipo</dt>
              <dd>{kind.label}</dd>
            </div>

            <div>
              <dt className="mb-1.5 text-xs font-semibold text-muted-foreground">Pasta</dt>
              <dd>
                <Select
                  value={record.folder_id ?? 'root'}
                  onValueChange={(value) =>
                    void run(() => api.moveRecord(record.id, value === 'root' ? null : value))
                  }
                >
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="root">Raiz do cofre</SelectItem>
                    {folders.map((folder) => (
                      <SelectItem key={folder.id} value={folder.id}>
                        {folder.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </dd>
            </div>

            <div>
              <dt className="mb-1.5 text-xs font-semibold text-muted-foreground">Versão</dt>
              <dd>
                {record.version}, alterado em {record.updated_at.slice(0, 16).replace('T', ' às ')}
              </dd>
            </div>

            {/*
              * A date the person picks for themselves, which the health lens then reports on when
              * it passes. Not a policy and not a reminder that fires — just the answer to "when did
              * I decide this one should change?", kept next to the record it is about.
              */}
            <div>
              <dt className="mb-1.5 text-xs font-semibold text-muted-foreground">Trocar até</dt>
              <dd className="flex items-center gap-1">
                <Input
                  type="date"
                  value={record.rotation_due_at?.slice(0, 10) ?? ''}
                  onChange={(event) =>
                    void run(() =>
                      api.setRotationDue(
                        record.id,
                        event.target.value ? `${event.target.value}T00:00:00Z` : null,
                      ),
                    )
                  }
                />
                {record.rotation_due_at && (
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label="Tirar a data"
                    onClick={() => void run(() => api.setRotationDue(record.id, null))}
                  >
                    <X />
                  </Button>
                )}
              </dd>
            </div>
          </dl>

          <Attachments recordId={record.id} />

          {versions.length > 0 && (
            <>
              <h3 className="flex items-center gap-1.5 pt-2 text-sm font-bold">
                <History className="size-4 text-muted-foreground" />
                Histórico
              </h3>

              <ul className="space-y-1 text-sm">
                {versions.map((entry) => (
                  <li key={entry.version}>
                    <button
                      type="button"
                      className="flex w-full items-center justify-between gap-2 rounded-md px-2 py-1.5 text-left text-muted-foreground transition-colors hover:bg-hover hover:text-foreground"
                      onClick={() => setViewing(entry.version)}
                    >
                      <span>
                        Versão {entry.version}, {entry.created_at.slice(0, 10)}
                      </span>
                      <span className="text-xs">ver</span>
                    </button>
                  </li>
                ))}
              </ul>
            </>
          )}
        </aside>
      </ScrollArea>

      <AlertDialog open={confirmingDelete} onOpenChange={setConfirmingDelete}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Mandar “{record.payload.title}” para a lixeira?</AlertDialogTitle>
            <AlertDialogDescription>
              Vai para a lixeira, e de lá dá para restaurar enquanto o prazo não vencer.
            </AlertDialogDescription>
          </AlertDialogHeader>

          <AlertDialogFooter>
            <AlertDialogCancel>Cancelar</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                setConfirmingDelete(false)
                void api.archiveRecord(record.id, true).then(() => onClosed(record.id))
              }}
            >
              Mandar para a lixeira
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {viewing !== null && (
        <VersionDialog
          recordId={record.id}
          version={viewing}
          onRestore={() => {
            const version = viewing
            setViewing(null)
            void run(() => api.restoreVersion(record.id, version))
          }}
          onCancel={() => setViewing(null)}
        />
      )}
    </div>
  )
}

const CLIPBOARD_SECONDS = 30

function KindIconOf({ kind }: { kind: string }) {
  const Icon = kindOf(kind).icon

  return <Icon className="size-6" />
}

/**
 * Only ever link to http(s).
 *
 * The address comes out of the encrypted payload, which means it is whatever was typed. A
 * `javascript:` URL in an href is script execution on the one page that holds decrypted secrets in
 * memory — so the protocol is checked, and anything else is shown as text instead.
 */
function safeHref(url: string): string | null {
  // An empty address resolves against the page's own origin and comes back as a perfectly valid
  // link to keyhold itself — which rendered as a stray link icon under the title of every record
  // that has no address at all.
  if (!url.trim()) {
    return null
  }

  try {
    const parsed = new URL(url, window.location.origin)

    return parsed.protocol === 'http:' || parsed.protocol === 'https:' ? parsed.href : null
  } catch {
    return null
  }
}
