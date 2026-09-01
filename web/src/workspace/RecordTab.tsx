import { useEffect, useState } from 'react'
import {
  ExternalLink,
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
import { Separator } from '@/components/ui/separator'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'

import { api, ApiError, type FolderNode, type OpenRecord, type VersionEntry } from '../api'
import { Input } from '@/components/ui/input'

import { Attachments } from './Attachments'
import { RecordEditor } from './RecordEditor'
import { SecretLine } from './SecretLine'
import { VersionDialog } from './VersionDialog'
import { kindOf } from './kinds'

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
  onClosed: (id: string) => void
}

export function RecordTab({ record, folders, onReload, onClosed }: RecordTabProps) {
  const [editing, setEditing] = useState(false)
  const [propertiesOpen, setPropertiesOpen] = useState(true)
  const [draft, setDraft] = useState(record.payload)
  const [versions, setVersions] = useState<VersionEntry[]>([])
  const [viewing, setViewing] = useState<number | null>(null)
  const [error, setError] = useState('')

  useEffect(() => {
    setDraft(record.payload)
    setEditing(false)
    void api
      .versions(record.id)
      .then((found) => setVersions(found.versions))
      .catch(() => setVersions([]))
  }, [record.id, record.version, record.payload])

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
  const kind = kindOf(record.kind)
  const KindIcon = kind.icon

  return (
    <div
      className={cn(
        'grid h-full min-h-0',
        // The panel is a drawer, not a margin: open it is wide enough to read a folder name and a
        // date without wrapping, and closed it takes no room at all.
        propertiesOpen ? 'grid-cols-[minmax(0,1fr)_22rem]' : 'grid-cols-[minmax(0,1fr)_0]',
      )}
    >
      <ScrollArea className="h-full min-w-0">
        <div className="space-y-4 p-6">
          <header className="space-y-1">
            <h2 className="flex min-w-0 items-center gap-2 text-lg font-medium">
              <KindIcon className="size-4 shrink-0 text-muted-foreground" />
              <span className="truncate">{record.payload.title}</span>
              {record.favourite && <Star className="size-4 shrink-0 fill-current text-primary" />}

              <span className="flex-1" />

              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={propertiesOpen ? 'Esconder propriedades' : 'Mostrar propriedades'}
                    aria-expanded={propertiesOpen}
                    onClick={() => setPropertiesOpen(!propertiesOpen)}
                  >
                    {propertiesOpen ? <PanelRightClose /> : <PanelRightOpen />}
                  </Button>
                </TooltipTrigger>
                <TooltipContent>
                  {propertiesOpen ? 'Esconder propriedades' : 'Mostrar propriedades'}
                </TooltipContent>
              </Tooltip>
            </h2>

            {link ? (
              <a
                href={link}
                target="_blank"
                rel="noreferrer noopener"
                className="inline-flex items-center gap-1 text-sm text-primary hover:underline"
              >
                {record.payload.url}
                <ExternalLink className="size-3" />
              </a>
            ) : (
              record.payload.url && (
                <p className="text-sm text-muted-foreground">{record.payload.url}</p>
              )
            )}
          </header>

          {record.compromised_at && (
            <Badge variant="destructive" className="gap-1">
              <ShieldAlert className="size-3" />
              Comprometido desde {record.compromised_at.slice(0, 10)}
            </Badge>
          )}

          <div className="rounded-lg border px-4">
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
              <p className="py-3 text-sm text-muted-foreground">Este registro não tem campos.</p>
            )}
          </div>

          {record.payload.notes && (
            <p className="rounded-lg border bg-muted/30 p-4 text-sm whitespace-pre-wrap">
              {record.payload.notes}
            </p>
          )}

          {error && <p className="text-sm text-destructive">{error}</p>}

          <div className="flex flex-wrap items-center gap-2">
            <Button size="sm" onClick={() => setEditing(true)}>
              <Pencil />
              Editar
            </Button>

            <Button
              size="sm"
              variant="outline"
              onClick={() => void run(() => api.favouriteRecord(record.id, !record.favourite))}
            >
              <Star />
              {record.favourite ? 'Tirar dos favoritos' : 'Favoritar'}
            </Button>

            <Button
              size="sm"
              variant="outline"
              onClick={() =>
                void run(() => api.setCompromised(record.id, record.compromised_at === null))
              }
            >
              <ShieldAlert />
              {record.compromised_at ? 'Não está mais comprometido' : 'Marcar comprometido'}
            </Button>

            <Button
              size="sm"
              variant="destructive"
              onClick={() => void api.archiveRecord(record.id, true).then(() => onClosed(record.id))}
            >
              <Trash2 />
              Excluir
            </Button>
          </div>
        </div>
      </ScrollArea>

      {/*
        * Always rendered, never unmounted: the panel's border is what separates the two halves of
        * this screen, and a border that disappears with its contents leaves the divider stopping
        * partway down the page. Closed, it is a zero-width column that still draws its edge.
        */}
      <ScrollArea className={cn('h-full min-w-0 border-l', !propertiesOpen && 'overflow-hidden')}>
        <aside className={cn('space-y-4 p-4', !propertiesOpen && 'invisible')}>
          <h3 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            Propriedades
          </h3>

          <dl className="space-y-3 text-sm">
            <div className="space-y-1">
              <dt className="text-muted-foreground">Tipo</dt>
              <dd>{kind.label}</dd>
            </div>

            <div className="space-y-1">
              <dt className="text-muted-foreground">Pasta</dt>
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

            <div className="space-y-1">
              <dt className="text-muted-foreground">Versão</dt>
              <dd>{record.version}</dd>
            </div>

            <div className="space-y-1">
              <dt className="text-muted-foreground">Alterado</dt>
              <dd>{record.updated_at.replace('T', ' ').replace('Z', '')}</dd>
            </div>

            {/*
              * A date the person picks for themselves, which the health lens then reports on when
              * it passes. Not a policy and not a reminder that fires — just the answer to "when did
              * I decide this one should change?", kept next to the record it is about.
              */}
            <div className="space-y-1">
              <dt className="text-muted-foreground">Trocar até</dt>
              <dd className="flex items-center gap-1">
                <Input
                  type="date"
                  className="h-8"
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

          <Separator />

          <Attachments recordId={record.id} />

          {versions.length > 0 && (
            <>
              <Separator />

              <h3 className="flex items-center gap-1.5 text-xs font-medium tracking-wide text-muted-foreground uppercase">
                <History className="size-3" />
                Histórico
              </h3>

              <ul className="space-y-1 text-sm">
                {versions.map((entry) => (
                  <li key={entry.version}>
                    <button
                      type="button"
                      className="flex w-full items-center justify-between gap-2 rounded-md px-1 py-0.5 text-left text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                      onClick={() => setViewing(entry.version)}
                    >
                      <span>
                        v{entry.version} · {entry.created_at.slice(0, 10)}
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
