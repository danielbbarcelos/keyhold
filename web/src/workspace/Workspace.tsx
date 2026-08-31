import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  AlertTriangle,
  Folder,
  FolderPlus,
  Lock,
  MoreHorizontal,
  Plus,
  Search,
  ScrollText,
  Settings2,
  Star,
  Trash2,
  Vault,
  X,
} from 'lucide-react'

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
import { Badge } from '@/components/ui/badge'
import { Checkbox } from '@/components/ui/checkbox'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { ScrollArea } from '@/components/ui/scroll-area'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'

import {
  api,
  ApiError,
  type FolderNode,
  type OpenRecord,
  type Payload,
  type RecordPreview,
  type Status,
  type VaultHealth,
  type VaultSummary,
} from '../api'
import { clearNow } from '../clipboard'
import { CommandPalette } from './CommandPalette'
import { NameDialog } from './NameDialog'
import { RecordDialog } from './RecordDialog'
import { emptyPayload } from './RecordEditor'
import { RecordTab } from './RecordTab'
import { BulkBar } from './BulkBar'
import { colourOf } from './colours'
import {
  DeleteFolderDialog,
  MoveFolderDialog,
  RenameFolderDialog,
} from './FolderDialogs'
import { AgentApproval } from './AgentApproval'
import { AuditPanel } from './AuditPanel'
import { HealthPanel } from './HealthPanel'
import { Settings } from './Settings'
import { TrashPanel } from './TrashPanel'
import { kindIcon } from './kinds'

/**
 * The workspace: a rail of lenses, a tree, tabs, and a properties panel.
 *
 * The layout is an editor's rather than a form's, because a vault has exactly that shape — a tree
 * on the left, the thing you opened in the middle, what is true about it on the right. Two
 * consequences worth stating:
 *
 *  - **A record opens as a tab**, so two of them can be read side by side. That is not a flourish:
 *    rotating a credential means having the old and the new in view at once.
 *  - **The rail changes what the list shows, never where you are.** There is one screen. A locked
 *    vault has nowhere to navigate to, and a route table would describe the shape of what is inside.
 */

type Lens = 'all' | 'favourites' | 'attention' | 'trash' | 'trail' | 'settings'

const LENSES: Array<{ id: Lens; label: string; icon: typeof Vault }> = [
  { id: 'all', label: 'Tudo', icon: Vault },
  { id: 'favourites', label: 'Favoritos', icon: Star },
  { id: 'attention', label: 'Atenção', icon: AlertTriangle },
  { id: 'trash', label: 'Lixeira', icon: Trash2 },
  { id: 'trail', label: 'Trilha', icon: ScrollText },
  { id: 'settings', label: 'Ajustes', icon: Settings2 },
]

interface WorkspaceProps {
  status: Status
  onChanged: () => void
}

export function Workspace({ status, onChanged }: WorkspaceProps) {
  const [vaults, setVaults] = useState<VaultSummary[]>([])
  const [vaultId, setVaultId] = useState<string | null>(null)
  const [folders, setFolders] = useState<FolderNode[]>([])
  const [records, setRecords] = useState<RecordPreview[]>([])
  const [health, setHealth] = useState<VaultHealth | null>(null)
  const [trash, setTrash] = useState<RecordPreview[]>([])
  const [lens, setLens] = useState<Lens>('all')
  const [folderId, setFolderId] = useState<string | null>(null)
  const [query, setQuery] = useState('')
  const [tabs, setTabs] = useState<OpenRecord[]>([])
  const [activeTab, setActiveTab] = useState<string | null>(null)
  const [creating, setCreating] = useState<{ kind: string; payload: Payload } | null>(null)
  const [paletteOpen, setPaletteOpen] = useState(false)
  const [asking, setAsking] = useState<
    { what: 'vault' } | { what: 'folder'; parentId: string | null } | null
  >(null)
  const [lockAsked, setLockAsked] = useState(false)
  const [folderAction, setFolderAction] = useState<
    { what: 'rename' | 'move' | 'delete'; folder: FolderNode } | null
  >(null)
  const [selected, setSelected] = useState<string[]>([])
  const [dragging, setDragging] = useState<string | null>(null)
  const [dropTarget, setDropTarget] = useState<string | null | undefined>(undefined)
  const [error, setError] = useState('')

  const refreshVaults = useCallback(async () => {
    try {
      const found = await api.vaults()
      setVaults(found.vaults)

      // Pick a vault when there is none, and pick a new one when the selected vault stopped
      // existing — deleted, or put away. Leaving the id pointing at nothing would show an empty
      // list that looks like an empty vault.
      setVaultId((current) => {
        const stillThere = found.vaults.some(
          (vault) => vault.id === current && !vault.archived,
        )

        if (current && stillThere) {
          return current
        }

        return found.vaults.find((vault) => !vault.archived)?.id ?? null
      })
    } catch (cause) {
      setError(describe(cause))
    }
  }, [])

  const refreshContents = useCallback(async () => {
    if (!vaultId) {
      return
    }

    try {
      const [tree, list] = await Promise.all([api.folders(vaultId), api.records(vaultId)])

      setFolders(tree.folders)
      setRecords(list.records)

      // The two lenses that ask the binary a different question. Fetched only when they are on
      // screen: health opens every record in the vault to judge it, which is not something to do
      // on every refresh of a list nobody is looking at.
      if (lens === 'attention') {
        setHealth(null)
        setHealth((await api.health(vaultId)).health)
      }

      if (lens === 'trash') {
        setTrash((await api.trash(vaultId)).records)
      }
    } catch (cause) {
      setError(describe(cause))
    }
  }, [vaultId, lens])

  useEffect(() => void refreshVaults(), [refreshVaults])
  useEffect(() => void refreshContents(), [refreshContents])

  // Tabs belong to the vault that was open. When that changes — including because the vault was
  // deleted from the settings screen — they are records this screen can no longer reach.
  useEffect(() => {
    setTabs([])
    setActiveTab(null)
    setFolderId(null)
    setSelected([])
  }, [vaultId])

  // A selection is about the rows in front of you. Changing what the list shows makes it a
  // selection of things you can no longer see, which is how bulk actions hit the wrong records.
  useEffect(() => setSelected([]), [lens, folderId])

  // Locking is not a screen change — it takes the clipboard back too.
  useEffect(() => {
    if (!status.lock.unlocked) {
      clearNow()
      setTabs([])
    }
  }, [status.lock.unlocked])

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'p') {
        event.preventDefault()
        setPaletteOpen((open) => !open)
      }
    }

    window.addEventListener('keydown', onKey)

    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const visible = useMemo(() => {
    const term = query.trim().toLowerCase()

    return records.filter((record) => {
      if (record.archived) return false
      if (lens === 'favourites' && !record.favourite) return false
      if (folderId && record.folder_id !== folderId) return false
      if (term && !searchable(record).includes(term)) return false

      return true
    })
  }, [records, lens, folderId, query])

  // Two lenses replace the list entirely rather than filtering it: one is a report, the other is a
  // different set of records with different things to do to them.
  const wholeLens = lens === 'attention' || lens === 'trash' || lens === 'trail'

  async function openRecord(id: string) {
    if (tabs.some((tab) => tab.id === id)) {
      setActiveTab(id)

      return
    }

    try {
      const opened = await api.openRecord(id)
      setTabs((current) => [...current, opened.record])
      setActiveTab(id)
      setCreating(null)
    } catch (cause) {
      setError(describe(cause))
    }
  }

  async function saveNew() {
    if (!vaultId || !creating) {
      return
    }

    try {
      const made = await api.createRecord(vaultId, creating.kind, folderId, creating.payload)
      setCreating(null)
      await refreshContents()
      await openRecord(made.record.id)
    } catch (cause) {
      setError(describe(cause))
    }
  }

  const activeVault = vaults.find((vault) => vault.id === vaultId) ?? null

  return (
    <div className="grid h-screen grid-cols-[3.5rem_16rem_minmax(0,1fr)] overflow-hidden bg-background">
      {/* ── the rail ─────────────────────────────────────────────────────────────────────────── */}
      <nav className="flex flex-col items-center gap-1 border-r bg-sidebar py-3">
        {LENSES.map((entry) => {
          const Icon = entry.icon

          return (
            <Tooltip key={entry.id}>
              <TooltipTrigger asChild>
                <Button
                  variant={entry.id === lens ? 'secondary' : 'ghost'}
                  size="icon"
                  aria-label={entry.label}
                  aria-current={entry.id === lens}
                  onClick={() => {
                    setLens(entry.id)
                    setFolderId(null)
                  }}
                >
                  <Icon />
                </Button>
              </TooltipTrigger>
              <TooltipContent side="right">{entry.label}</TooltipContent>
            </Tooltip>
          )
        })}

        <span className="flex-1" />

        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              aria-label="Trancar agora"
              onClick={() => setLockAsked(true)}
            >
              <Lock />
            </Button>
          </TooltipTrigger>
          <TooltipContent side="right">Trancar agora</TooltipContent>
        </Tooltip>
      </nav>

      {/* ── the explorer ─────────────────────────────────────────────────────────────────────── */}
      {lens === 'settings' ? (
        <div className="col-span-2 min-w-0">
          <Header status={status} onChanged={onChanged} title="Ajustes" />
          <ScrollArea className="h-[calc(100vh-3.5rem)]">
            <Settings
              status={status}
              vaults={vaults}
              onChanged={onChanged}
              onVaultsChanged={refreshVaults}
            />
          </ScrollArea>
        </div>
      ) : (
        <>
          <aside className="flex min-w-0 flex-col border-r bg-sidebar">
            <div className="flex h-14 items-center gap-1.5 border-b px-3">
              <Select
                value={vaultId ?? ''}
                onValueChange={(value) => {
                  setVaultId(value)
                  setFolderId(null)
                  setTabs([])
                  setActiveTab(null)
                }}
              >
                <SelectTrigger className="h-8 flex-1" aria-label="Cofre">
                  <SelectValue placeholder="Cofre" />
                </SelectTrigger>
                <SelectContent>
                  {vaults
                    .filter((vault) => !vault.archived)
                    .map((vault) => (
                      <SelectItem key={vault.id} value={vault.id}>
                        {vault.name}
                      </SelectItem>
                    ))}
                </SelectContent>
              </Select>

              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label="Novo cofre"
                    onClick={() => setAsking({ what: 'vault' })}
                  >
                    <Plus />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>Novo cofre</TooltipContent>
              </Tooltip>
            </div>

            <ScrollArea className="flex-1">
              <div className="space-y-0.5 p-2">
                <TreeRow
                  selected={folderId === null}
                  onClick={() => setFolderId(null)}
                  dropping={dropTarget === null}
                  onDragOver={(event) => {
                    event.preventDefault()
                    setDropTarget(null)
                  }}
                  onDragLeave={() => setDropTarget(undefined)}
                  onDrop={(event) => {
                    event.preventDefault()
                    setDropTarget(undefined)

                    const id = draggedRecord(event)

                    if (id) {
                      void api.moveRecord(id, null).then(refreshContents)
                    }
                  }}
                >
                  Todos os registros
                </TreeRow>

                {folders.map((folder) => (
                  <TreeRow
                    key={folder.id}
                    depth={folder.depth}
                    selected={folder.id === folderId}
                    colour={folder.colour}
                    onClick={() => setFolderId(folder.id)}
                    dropping={dropTarget === folder.id}
                    onDragOver={(event) => {
                      event.preventDefault()
                      setDropTarget(folder.id)
                    }}
                    onDragLeave={() => setDropTarget(undefined)}
                    onDrop={(event) => {
                      event.preventDefault()
                      setDropTarget(undefined)

                      const id = draggedRecord(event)

                      if (id) {
                        // Dragging one of several selected records moves the whole selection: the
                        // rows are highlighted, so treating them as one thing is what the screen
                        // already promised.
                        const moving = selected.includes(id) ? selected : [id]

                        void Promise.all(
                          moving.map((one) => api.moveRecord(one, folder.id)),
                        ).then(() => {
                          setSelected([])
                          void refreshContents()
                        })
                      }
                    }}
                    menu={
                      <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                          <Button
                            variant="ghost"
                            size="icon-xs"
                            aria-label={`Ações da pasta ${folder.name}`}
                          >
                            <MoreHorizontal />
                          </Button>
                        </DropdownMenuTrigger>

                        <DropdownMenuContent align="start">
                          <DropdownMenuItem
                            onSelect={() => setFolderAction({ what: 'rename', folder })}
                          >
                            Renomear
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            onSelect={() => setFolderAction({ what: 'move', folder })}
                          >
                            Mover
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            onSelect={() => setAsking({ what: 'folder', parentId: folder.id })}
                          >
                            Nova pasta dentro
                          </DropdownMenuItem>
                          <DropdownMenuSeparator />
                          <DropdownMenuItem
                            variant="destructive"
                            onSelect={() => setFolderAction({ what: 'delete', folder })}
                          >
                            Excluir
                          </DropdownMenuItem>
                        </DropdownMenuContent>
                      </DropdownMenu>
                    }
                  >
                    {folder.name}
                  </TreeRow>
                ))}
              </div>
            </ScrollArea>

            <div className="border-t p-2">
              <Button
                variant="ghost"
                size="sm"
                className="w-full justify-start"
                disabled={!vaultId}
                onClick={() => setAsking({ what: 'folder', parentId: folderId })}
              >
                <FolderPlus />
                Nova pasta
              </Button>
            </div>
          </aside>

          {/* ── the middle ─────────────────────────────────────────────────────────────────── */}
          <section className="flex min-w-0 flex-col">
            <Header status={status} onChanged={onChanged} title={wholeLens ? titleFor(lens) : undefined}>
              {selected.length > 0 ? (
                <BulkBar
                  selected={selected}
                  folders={folders}
                  onClear={() => setSelected([])}
                  onChanged={refreshContents}
                />
              ) : (
              <>
              {!wholeLens && (
              <div className="relative max-w-sm flex-1">
                <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
                <Input
                  className="h-8 pl-8"
                  placeholder="Buscar…   Ctrl+P"
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                />
              </div>
              )}

              {!wholeLens && (
                <Button
                  size="sm"
                  disabled={!vaultId}
                  onClick={() => {
                    setCreating({ kind: 'login', payload: emptyPayload('login') })
                    setActiveTab(null)
                  }}
                >
                  <Plus />
                  Novo registro
                </Button>
              )}
              </>
              )}
            </Header>

            {wholeLens ? (
              <ScrollArea className="min-h-0 flex-1">
                {lens === 'trail' ? (
                  <AuditPanel
                    onOpenRecord={(recordId) => {
                      setLens('all')
                      void openRecord(recordId)
                    }}
                  />
                ) : lens === 'attention' ? (
                  <HealthPanel
                    health={health}
                    onOpen={(recordId) => {
                      setLens('all')
                      void openRecord(recordId)
                    }}
                  />
                ) : (
                  vaultId && (
                    <TrashPanel
                      records={trash}
                      retentionDays={status.retention_days}
                      vaultId={vaultId}
                      onChanged={refreshContents}
                    />
                  )
                )}
              </ScrollArea>
            ) : (
            <div className="grid min-h-0 flex-1 grid-cols-[17rem_minmax(0,1fr)]">
              <ScrollArea className="border-r">
                <ul className="space-y-0.5 p-2">
                  {visible.map((record) => {
                    const Icon = kindIcon(record.kind)
                    const attention = needsAttention(record)
                    const picked = selected.includes(record.id)

                    return (
                      <li key={record.id}>
                        <div
                          draggable
                          onDragStart={(event) => {
                            recordDragData(event, record.id)
                            setDragging(record.id)
                          }}
                          onDragEnd={() => {
                            setDragging(null)
                            setDropTarget(undefined)
                          }}
                          className={cn(
                            'group flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm transition-colors',
                            record.id === activeTab
                              ? 'bg-accent text-accent-foreground'
                              : 'hover:bg-muted',
                            dragging === record.id && 'opacity-50',
                          )}
                        >
                          {/*
                            * The checkbox takes the icon's place on hover, rather than sitting in a
                            * column of its own: a permanent column of checkboxes turns a list you
                            * read into a form you fill in.
                            */}
                          <span className="relative size-4 shrink-0">
                            <Icon
                              className={cn(
                                'absolute inset-0 size-4 text-muted-foreground transition-opacity',
                                (picked || selected.length > 0) && 'opacity-0',
                                'group-hover:opacity-0',
                              )}
                            />
                            <Checkbox
                              checked={picked}
                              aria-label={`Selecionar ${record.title}`}
                              className={cn(
                                'absolute inset-0 opacity-0 transition-opacity group-hover:opacity-100',
                                (picked || selected.length > 0) && 'opacity-100',
                              )}
                              onCheckedChange={(on) =>
                                setSelected((current) =>
                                  on
                                    ? [...current, record.id]
                                    : current.filter((one) => one !== record.id),
                                )
                              }
                            />
                          </span>

                          <button
                            type="button"
                            onClick={() => void openRecord(record.id)}
                            className="min-w-0 flex-1 truncate text-left"
                          >
                            {record.title}
                          </button>

                          {record.favourite && (
                            <Star className="size-3 shrink-0 fill-current text-primary" />
                          )}
                          {attention && (
                            <AlertTriangle
                              className={cn(
                                'size-3 shrink-0',
                                record.compromised_at ? 'text-destructive' : 'text-muted-foreground',
                              )}
                            />
                          )}
                        </div>
                      </li>
                    )
                  })}

                  {visible.length === 0 && (
                    <li className="px-2 py-6 text-center text-sm text-muted-foreground">
                      Nada aqui.
                    </li>
                  )}
                </ul>
              </ScrollArea>

              <div className="flex min-w-0 flex-col">
                {tabs.length > 0 && (
                  <div className="flex h-10 shrink-0 items-stretch gap-px overflow-x-auto border-b px-2">
                    {tabs.map((tab) => (
                      <div
                        key={tab.id}
                        className={cn(
                          'flex items-center gap-1.5 rounded-t-md px-3 text-sm',
                          tab.id === activeTab
                            ? 'bg-muted text-foreground'
                            : 'text-muted-foreground hover:bg-muted/50',
                        )}
                      >
                        <button
                          type="button"
                          className="max-w-40 truncate"
                          onClick={() => setActiveTab(tab.id)}
                        >
                          {tab.payload.title}
                        </button>
                        <Button
                          variant="ghost"
                          size="icon-xs"
                          aria-label="Fechar"
                          onClick={() => {
                            setTabs((current) => current.filter((one) => one.id !== tab.id))

                            if (tab.id === activeTab) {
                              setActiveTab(null)
                            }
                          }}
                        >
                          <X />
                        </Button>
                      </div>
                    ))}
                  </div>
                )}

                {activeTab ? (
                  <div className="min-h-0 flex-1">
                    <RecordTab
                      key={activeTab}
                      record={tabs.find((tab) => tab.id === activeTab)!}
                      folders={folders}
                      onReload={async (id) => {
                        const fresh = await api.openRecord(id)
                        setTabs((current) =>
                          current.map((tab) => (tab.id === id ? fresh.record : tab)),
                        )
                        await refreshContents()
                      }}
                      onClosed={(id) => {
                        setTabs((current) => current.filter((tab) => tab.id !== id))
                        setActiveTab(null)
                        void refreshContents()
                      }}
                    />
                  </div>
                ) : (
                  <div className="grid flex-1 place-items-center p-8 text-center">
                    <p className="max-w-xs text-sm text-muted-foreground">
                      {activeVault
                        ? 'Escolha um registro à esquerda, ou crie um. Ctrl+P abre a busca.'
                        : 'Crie um cofre para começar.'}
                    </p>
                  </div>
                )}
              </div>
            </div>
            )}
          </section>
        </>
      )}

      {/*
        * Locking asks first, because it is not the small action its icon suggests: the key is
        * dropped, every open tab closes, and anything half-written goes with it. The auto-lock does
        * the same thing without asking — but that one you configured, and this one is a click away
        * from the button that opens the vault's settings.
        */}
      <AlertDialog open={lockAsked} onOpenChange={setLockAsked}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Trancar o cofre agora?</AlertDialogTitle>
            <AlertDialogDescription>
              A chave é apagada da memória e as abas abertas se fecham. Para voltar, você digita a
              senha-mestra de novo.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancelar</AlertDialogCancel>
            <AlertDialogAction onClick={() => void api.lock().then(onChanged)}>
              Trancar
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {creating && (
        <RecordDialog
          open
          kind={creating.kind}
          payload={creating.payload}
          // Functional updates, and not for style: choosing a kind sets the kind *and* replaces the
          // starter fields, in two calls. Spreading the captured object twice makes the second
          // write discard the first — which showed up as a record type that refused to change.
          onKindChange={(kind) =>
            setCreating((current) => (current ? { ...current, kind } : current))
          }
          onChange={(payload) =>
            setCreating((current) => (current ? { ...current, payload } : current))
          }
          onSave={() => void saveNew()}
          onCancel={() => setCreating(null)}
          error={error}
        />
      )}

      {folderAction?.what === 'rename' && (
        <RenameFolderDialog
          folder={folderAction.folder}
          onDone={() => {
            setFolderAction(null)
            void refreshContents()
          }}
          onCancel={() => setFolderAction(null)}
        />
      )}

      {folderAction?.what === 'move' && (
        <MoveFolderDialog
          folder={folderAction.folder}
          folders={folders}
          onDone={() => {
            setFolderAction(null)
            void refreshContents()
          }}
          onCancel={() => setFolderAction(null)}
        />
      )}

      {folderAction?.what === 'delete' && (
        <DeleteFolderDialog
          folder={folderAction.folder}
          records={records.filter((record) => record.folder_id === folderAction.folder.id).length}
          onDone={() => {
            setFolderAction(null)

            if (folderId === folderAction.folder.id) {
              setFolderId(null)
            }

            void refreshContents()
          }}
          onCancel={() => setFolderAction(null)}
        />
      )}

      {asking?.what === 'vault' && (
        <NameDialog
          title="Novo cofre"
          hint="Um cofre é um agrupamento: pode ficar trancado enquanto outro está aberto, e é a unidade que um agente pode ou não enxergar."
          onConfirm={(name) => {
            setAsking(null)
            void api.createVault(name).then(refreshVaults)
          }}
          onCancel={() => setAsking(null)}
        />
      )}

      {asking?.what === 'folder' && vaultId && (
        <NameDialog
          title="Nova pasta"
          hint={asking.parentId ? 'Dentro da pasta selecionada.' : 'Na raiz do cofre.'}
          withColour
          onConfirm={(name, colour) => {
            const parentId = asking.parentId
            setAsking(null)
            void api.createFolder(vaultId, name, parentId, colour).then(refreshContents)
          }}
          onCancel={() => setAsking(null)}
        />
      )}

      {/*
        * The approval dialog lives above every lens: an agent asks whenever it asks, and a question
        * that only appeared on one screen would be a question nobody answers.
        */}
      <AgentApproval onAnswered={refreshContents} />

      <CommandPalette
        records={records.filter((record) => !record.archived)}
        open={paletteOpen}
        onOpenChange={setPaletteOpen}
        onPick={(id) => {
          setPaletteOpen(false)
          void openRecord(id)
        }}
      />
    </div>
  )
}

/**
 * Moving records by dragging them onto a folder.
 *
 * Plain HTML drag and drop rather than a library: this is one gesture with one payload — a record's
 * id — and a dependency for it would be more code to audit than the feature is worth in a program
 * whose argument is that it depends on very little.
 */
function recordDragData(event: React.DragEvent, id: string) {
  event.dataTransfer.setData('text/keyhold-record', id)
  event.dataTransfer.effectAllowed = 'move'
}

function draggedRecord(event: React.DragEvent): string | null {
  return event.dataTransfer.getData('text/keyhold-record') || null
}

/**
 * The one bar across the top of the working area.
 *
 * It is a fixed height and the explorer's header matches it, which is the whole reason it exists as
 * a component: two headers that drift apart by a pixel are what makes a shell look unfinished.
 */
function Header({
  status,
  onChanged,
  title,
  children,
}: WorkspaceProps & { title?: string; children?: React.ReactNode }) {
  return (
    <header className="flex h-14 shrink-0 items-center gap-2 border-b px-3">
      {title && <h1 className="text-sm font-medium">{title}</h1>}
      {children}
      <span className="flex-1" />
      <Countdown status={status} onChanged={onChanged} />
    </header>
  )
}

function Countdown({ status, onChanged }: WorkspaceProps) {
  const seconds = status.lock.locks_in

  if (seconds === null) {
    return (
      <Badge variant="outline" className="gap-1 font-normal">
        Sem tranca automática
      </Badge>
    )
  }

  const label = seconds >= 60 ? `${Math.ceil(seconds / 60)} min` : `${seconds}s`

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          className="tabular-nums"
          onClick={() => void api.heartbeat().then(onChanged)}
        >
          <Lock className={seconds <= 30 ? 'text-destructive' : undefined} />
          {label}
        </Button>
      </TooltipTrigger>
      <TooltipContent>Tranca em {label}. Clique para adiar.</TooltipContent>
    </Tooltip>
  )
}

const INDENT = ['pl-2', 'pl-6', 'pl-10', 'pl-14', 'pl-[4.5rem]', 'pl-[5.5rem]']

function TreeRow({
  depth = 0,
  selected,
  colour,
  dropping,
  onClick,
  onDragOver,
  onDragLeave,
  onDrop,
  menu,
  children,
}: {
  depth?: number
  selected: boolean
  colour?: string | null
  dropping?: boolean
  onClick: () => void
  onDragOver?: (event: React.DragEvent) => void
  onDragLeave?: () => void
  onDrop?: (event: React.DragEvent) => void
  menu?: React.ReactNode
  children: React.ReactNode
}) {
  return (
    <div
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      className={cn(
        'group flex items-center rounded-md pr-1 transition-colors',
        dropping && 'ring-2 ring-primary',
        // Static classes rather than a computed padding: Tailwind only ships the classes it can see
        // in the source, and an interpolated one would simply not exist in the stylesheet.
        INDENT[Math.min(depth, INDENT.length - 1)],
        selected ? 'bg-accent text-accent-foreground' : 'text-muted-foreground hover:bg-muted',
      )}
    >
      <button
        type="button"
        onClick={onClick}
        className="flex min-w-0 flex-1 items-center gap-2 py-1.5 text-left text-sm"
      >
        {colour !== undefined && (
          <Folder className={cn('size-3.5 shrink-0', colourOf(colour).text)} />
        )}
        <span className="truncate">{children}</span>
      </button>

      {/* The menu appears on hover and stays put once opened, so it never moves out from under a
          pointer that is already on its way to it. */}
      {menu && (
        <span className="shrink-0 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100 has-aria-expanded:opacity-100">
          {menu}
        </span>
      )}
    </div>
  )
}

function titleFor(lens: Lens): string {
  return LENSES.find((entry) => entry.id === lens)?.label ?? ''
}

/** What the toolbar's search looks at: the parts of a record that are not secret. */
function searchable(record: RecordPreview): string {
  return `${record.title} ${record.username} ${record.url}`.toLowerCase()
}

/**
 * The marker on a row in the list.
 *
 * Deliberately the two things the file itself knows — marked compromised, rotation date passed —
 * rather than the full health report: a list must not open every record in the vault to draw
 * itself. The attention lens is where the rest of the reasoning lives.
 */
function needsAttention(record: RecordPreview): boolean {
  if (record.compromised_at) {
    return true
  }

  return record.rotation_due_at !== null && new Date(record.rotation_due_at) <= new Date()
}

function describe(cause: unknown): string {
  return cause instanceof ApiError ? cause.message : 'Não foi possível falar com o keyhold.'
}
