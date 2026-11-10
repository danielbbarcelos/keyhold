import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  AlertTriangle,
  ChevronDown,
  Cpu,
  Folder,
  Layers,
  Lock,
  Plus,
  Search,
  ScrollText,
  Settings2,
  SquarePen,
  Star,
  Trash2,
  Vault,
  X,
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'

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
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'

import { Logo } from '@/components/common/Logo'

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
import { COLOURS, colourOf } from './colours'
import { MenuButton, RowMenu, type MenuEntry } from './menu'
import {
  DeleteFolderDialog,
  MoveFolderDialog,
  RenameFolderDialog,
} from './FolderDialogs'
import { MoveToVaultDialog } from './MoveToVaultDialog'
import { EVERY_VAULT, VaultPicker } from './VaultPicker'
import { AgentApproval } from './AgentApproval'
import { AuditPanel } from './AuditPanel'
import { InstancesPanel } from './InstancesPanel'
import { HealthPanel } from './HealthPanel'
import { Settings } from './Settings'
import { TrashPanel } from './TrashPanel'
import { kindIcon, kindTone, subtitleOf } from './kinds'

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

type Lens = 'all' | 'favourites' | 'attention' | 'trash' | 'trail' | 'instances' | 'settings'

const LENSES: Array<{ id: Lens; label: string; icon: typeof Vault }> = [
  { id: 'all', label: 'Tudo', icon: Vault },
  { id: 'favourites', label: 'Favoritos', icon: Star },
  { id: 'attention', label: 'Atenção', icon: AlertTriangle },
  { id: 'trash', label: 'Lixeira', icon: Trash2 },
  { id: 'trail', label: 'Trilha', icon: ScrollText },
  { id: 'instances', label: 'Instâncias', icon: Cpu },
  { id: 'settings', label: 'Ajustes', icon: Settings2 },
]

/** A rail button: the lens it is on reads in the accent, the rest stay quiet. */
const RAIL_BUTTON =
  'size-10 rounded-[10px] text-sidebar-muted hover:bg-sidebar-hover hover:text-sidebar-foreground aria-[current=true]:bg-sidebar-accent aria-[current=true]:text-sidebar-accent-foreground [&_svg:not([class*=size-])]:size-5'

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
  // `folderId` is fixed when the dialog opens rather than read when it saves: a clone belongs
  // wherever its original is, which is not necessarily the folder the list happens to be showing.
  const [creating, setCreating] = useState<{
    kind: string
    payload: Payload
    folderId: string | null
    cloning?: boolean
  } | null>(null)
  const [paletteOpen, setPaletteOpen] = useState(false)
  const [asking, setAsking] = useState<
    { what: 'vault' } | { what: 'folder'; parentId: string | null } | null
  >(null)
  const [lockAsked, setLockAsked] = useState(false)
  const [folderAction, setFolderAction] = useState<
    { what: 'rename' | 'move' | 'delete'; folder: FolderNode } | null
  >(null)
  const [deleting, setDeleting] = useState<RecordPreview | null>(null)
  // `what` is the phrase the dialog puts in its title, so one record and a selection read alike.
  const [movingToVault, setMovingToVault] = useState<{ ids: string[]; what: string } | null>(null)
  const [selected, setSelected] = useState<string[]>([])
  const [foldersOpen, setFoldersOpen] = useState(true)
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

        if (current === EVERY_VAULT || (current && stillThere)) {
          return current
        }

        return found.vaults.find((vault) => !vault.archived)?.id ?? null
      })
    } catch (cause) {
      setError(describe(cause))
    }
  }, [])

  /**
   * What the screen is showing: one vault, or all of them at once.
   *
   * Across every vault there is no folder tree — a folder belongs to a vault, and a tree that mixed
   * "Clientes" from two of them would be a tree whose rows mean different things. The sidebar shows
   * the vaults instead, which is the only grouping that still holds.
   */
  const refreshContents = useCallback(async () => {
    if (!vaultId) {
      return
    }

    const across = vaultId === EVERY_VAULT
    const reach = across ? vaults.filter((vault) => !vault.archived).map((v) => v.id) : [vaultId]

    try {
      if (across) {
        const lists = await Promise.all(reach.map((id) => api.records(id)))

        setFolders([])
        setRecords(
          lists
            .flatMap((list) => list.records)
            .sort((a, b) => b.updated_at.localeCompare(a.updated_at)),
        )
      } else {
        const [tree, list] = await Promise.all([api.folders(vaultId), api.records(vaultId)])

        setFolders(tree.folders)
        setRecords(list.records)
      }

      // The two lenses that ask the binary a different question. Fetched only when they are on
      // screen: health opens every record in the vault to judge it, which is not something to do
      // on every refresh of a list nobody is looking at.
      if (lens === 'attention') {
        setHealth(null)

        const reports = await Promise.all(reach.map((id) => api.health(id)))

        setHealth({
          findings: reports.flatMap((report) => report.health.findings),
          examined: reports.reduce((total, report) => total + report.health.examined, 0),
        })
      }

      if (lens === 'trash') {
        const bins = await Promise.all(reach.map((id) => api.trash(id)))

        setTrash(bins.flatMap((bin) => bin.records))
      }
    } catch (cause) {
      setError(describe(cause))
    }
  }, [vaultId, vaults, lens])

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
  const wholeLens =
    lens === 'attention' || lens === 'trash' || lens === 'trail' || lens === 'instances'

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
      const made = await api.createRecord(vaultId, creating.kind, creating.folderId, creating.payload)
      setCreating(null)
      await refreshContents()
      await openRecord(made.record.id)
    } catch (cause) {
      setError(describe(cause))
    }
  }

  /**
   * A copy of a record, opened for editing before it exists.
   *
   * The whole record is fetched rather than copied from the row: the list carries a title and a
   * username, and a clone that quietly dropped every other field would be worse than no clone. The
   * copy starts identical — same kind, same fields in the same order, including the ones somebody
   * added by hand — and lands in the original's folder, not in whatever the list is showing.
   *
   * Attachments are not copied. They are files, not fields, and duplicating them silently is not
   * something to decide on somebody's behalf.
   */
  async function cloneRecord(id: string) {
    try {
      const opened = await api.openRecord(id)

      setCreating({
        kind: opened.record.kind,
        payload: opened.record.payload,
        folderId: opened.record.folder_id,
        cloning: true,
      })
    } catch (cause) {
      setError(describe(cause))
    }
  }

  /**
   * Send a record to the trash.
   *
   * Closing the tab is part of it: a record in the trash is not a record with a tab, and leaving
   * one open means a pane whose Save writes to something the list no longer shows.
   */
  async function deleteRecord(id: string) {
    try {
      await api.archiveRecord(id, true)

      setTabs((current) => current.filter((tab) => tab.id !== id))
      setActiveTab((current) => (current === id ? null : current))

      await refreshContents()
    } catch (cause) {
      setError(describe(cause))
    }
  }

  /**
   * What can be done to a folder.
   *
   * Colour is a submenu here rather than only a field in the rename dialog. It was reachable that
   * way and nobody found it: "renomear" does not read like "and also pick a colour", so the one
   * thing people wanted to change about a folder was hidden behind the one thing they did not.
   */
  const folderEntries = (folder: FolderNode): MenuEntry[] => [
    { kind: 'item', label: 'Renomear', onSelect: () => setFolderAction({ what: 'rename', folder }) },
    {
      kind: 'sub',
      label: 'Cor',
      items: COLOURS.map((colour) => ({
        label: colour.label,
        swatch: colour.dot,
        onSelect: () =>
          void api.renameFolder(folder.id, folder.name, colour.value).then(refreshContents),
      })),
    },
    { kind: 'item', label: 'Mover', onSelect: () => setFolderAction({ what: 'move', folder }) },
    {
      kind: 'item',
      label: 'Nova pasta dentro',
      onSelect: () => setAsking({ what: 'folder', parentId: folder.id }),
    },
    { kind: 'separator' },
    {
      kind: 'item',
      label: 'Excluir',
      destructive: true,
      onSelect: () => setFolderAction({ what: 'delete', folder }),
    },
  ]

  /**
   * What can be done to a record without opening it.
   *
   * Deliberately about the row it was opened on, never about the selection: the checkboxes have
   * their own bar, and a right-click that quietly acts on nine other rows is how the wrong nine get
   * moved.
   *
   * Deleting asks, even though it only goes to the trash. "Excluir" sits one row below "Mover
   * para" in a menu that opens under the pointer, and undoing a mis-click meant finding the lens,
   * the row and the restore — three steps to undo one that was never intended.
   */
  const recordEntries = (record: RecordPreview): MenuEntry[] => [
    { kind: 'item', label: 'Abrir', onSelect: () => void openRecord(record.id) },
    { kind: 'item', label: 'Clonar', onSelect: () => void cloneRecord(record.id) },
    {
      kind: 'item',
      label: record.favourite ? 'Tirar dos favoritos' : 'Favoritar',
      onSelect: () =>
        void api.favouriteRecord(record.id, !record.favourite).then(refreshContents),
    },
    {
      kind: 'sub',
      label: 'Mover para',
      items: [
        {
          label: 'Raiz do cofre',
          onSelect: () => void api.moveRecord(record.id, null).then(refreshContents),
        },
        ...folders.map((folder) => ({
          label: `${'— '.repeat(folder.depth)}${folder.name}`,
          onSelect: () => void api.moveRecord(record.id, folder.id).then(refreshContents),
        })),
      ],
    },
    {
      kind: 'item',
      label: 'Mover para outro cofre…',
      onSelect: () => setMovingToVault({ ids: [record.id], what: `“${record.title}”` }),
    },
    { kind: 'separator' },
    {
      kind: 'item',
      label: 'Excluir',
      destructive: true,
      onSelect: () => setDeleting(record),
    },
  ]

  const activeVault = vaults.find((vault) => vault.id === vaultId) ?? null
  const across = vaultId === EVERY_VAULT
  const openVaults = vaults.filter((vault) => !vault.archived)
  const nameOfVault = (id: string) => vaults.find((vault) => vault.id === id)?.name ?? ''
  const liveRecords = records.filter((record) => !record.archived)
  const listTitle = across
    ? 'Todos os cofres'
    : lens === 'favourites'
      ? 'Favoritos'
      : (folders.find((folder) => folder.id === folderId)?.name ?? 'Todos os registros')

  function startNewRecord() {
    setCreating({ kind: 'login', payload: emptyPayload('login'), folderId })
    setActiveTab(null)
  }

  return (
    <div className="grid h-screen grid-cols-[4rem_16.5rem_minmax(0,1fr)] grid-rows-[minmax(0,1fr)] overflow-hidden bg-background">
      {/* ── the rail ─────────────────────────────────────────────────────────────────────────── */}
      <nav className="flex flex-col items-center gap-1.5 bg-rail py-3">
        {/*
          * The logo goes home.
          *
          * It is the one mark on the screen that is not a control, which is exactly why people
          * click it when they are lost — so it does the thing they mean by it: the whole list, no
          * folder, no open record, nothing typed in the search box.
          */}
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className="mb-2 size-10 rounded-[10px] hover:bg-sidebar-hover"
              aria-label="Início"
              onClick={() => {
                setLens('all')
                setFolderId(null)
                setActiveTab(null)
                setCreating(null)
                setQuery('')
                setSelected([])
              }}
            >
              <Logo className="size-9 rounded-[10px]" />
            </Button>
          </TooltipTrigger>
          <TooltipContent side="right">Início</TooltipContent>
        </Tooltip>

        {LENSES.map((entry) => {
          const Icon = entry.icon

          return (
            <Tooltip key={entry.id}>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  className={RAIL_BUTTON}
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
              className={RAIL_BUTTON}
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
        <div className="col-span-2 flex min-h-0 min-w-0 flex-col">
          <Header status={status} onChanged={onChanged} title="Ajustes" />
          <Settings
            status={status}
            vaults={vaults}
            onChanged={onChanged}
            onVaultsChanged={refreshVaults}
          />
        </div>
      ) : (
        <>
          <aside className="flex min-w-0 flex-col border-r border-sidebar-border bg-sidebar text-sidebar-foreground">
            <div className="flex h-14 shrink-0 items-center gap-2 border-b border-sidebar-border pr-3 pl-2">
              <VaultPicker
                vaults={vaults}
                value={vaultId}
                onPick={(id) => {
                  setVaultId(id)
                  setFolderId(null)
                  setTabs([])
                  setActiveTab(null)
                }}
                onNew={() => setAsking({ what: 'vault' })}
              />

              {/* The most common thing done here gets the one coloured button in the sidebar. A new
                  vault is rarer, and lives at the bottom of the vault picker. */}
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    size="icon"
                    className="size-8 rounded-lg"
                    aria-label="Novo registro"
                    disabled={!vaultId || across}
                    onClick={startNewRecord}
                  >
                    <SquarePen />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>Novo registro</TooltipContent>
              </Tooltip>
            </div>

            <ScrollArea className="min-h-0 flex-1">
              <div className="space-y-px p-2 pt-3">
                {across ? (
                  openVaults.map((vault) => (
                    <TreeRow
                      key={vault.id}
                      selected={false}
                      colour={vault.colour}
                      onClick={() => setVaultId(vault.id)}
                    >
                      {vault.name}
                    </TreeRow>
                  ))
                ) : (
                <>
                <TreeRow
                  icon={Layers}
                  count={liveRecords.length}
                  selected={lens !== 'favourites' && folderId === null}
                  onClick={() => {
                    setLens('all')
                    setFolderId(null)
                  }}
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

                <TreeRow
                  icon={Star}
                  count={liveRecords.filter((record) => record.favourite).length}
                  selected={lens === 'favourites'}
                  onClick={() => {
                    setLens('favourites')
                    setFolderId(null)
                  }}
                >
                  Favoritos
                </TreeRow>

                <button
                  type="button"
                  aria-expanded={foldersOpen}
                  onClick={() => setFoldersOpen((open) => !open)}
                  className="mt-3 flex h-7 w-full items-center gap-1.5 rounded-md px-2 text-sm text-sidebar-muted hover:bg-sidebar-hover hover:text-sidebar-foreground"
                >
                  <ChevronDown className={cn('size-3.5 transition-transform', !foldersOpen && '-rotate-90')} />
                  Pastas
                </button>

                {foldersOpen &&
                  folders.map((folder) => (
                  <TreeRow
                    key={folder.id}
                    depth={folder.depth}
                    count={liveRecords.filter((record) => record.folder_id === folder.id).length}
                    selected={folder.id === folderId}
                    colour={folder.colour}
                    onClick={() => {
                      setLens('all')
                      setFolderId(folder.id)
                    }}
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
                    entries={folderEntries(folder)}
                    menuLabel={`Ações da pasta ${folder.name}`}
                  >
                    {folder.name}
                  </TreeRow>
                ))}

                {foldersOpen && (
                  <button
                    type="button"
                    disabled={!vaultId}
                    onClick={() => setAsking({ what: 'folder', parentId: folderId })}
                    className="flex h-[30px] w-full items-center gap-2.5 rounded-md pl-2 text-sm text-sidebar-muted hover:bg-sidebar-hover hover:text-sidebar-foreground"
                  >
                    <span className="grid size-5 place-items-center rounded-md bg-sidebar-hover">
                      <Plus className="size-3.5" />
                    </span>
                    Adicionar pasta
                  </button>
                )}
                </>
                )}
              </div>
            </ScrollArea>
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
                  onMoveToVault={() =>
                    setMovingToVault({
                      ids: selected,
                      what: `${selected.length} ${selected.length === 1 ? 'registro' : 'registros'}`,
                    })
                  }
                />
              ) : (
              <>
              {!wholeLens && (
              <div className="relative max-w-md flex-1">
                <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
                <Input
                  className="h-9 bg-surface pl-9 dark:bg-surface"
                  placeholder="Buscar…   Ctrl+P"
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                />
              </div>
              )}

              {!wholeLens && (
                <Button
                  size="sm"
                  disabled={!vaultId || across}
                  onClick={startNewRecord}
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
                ) : lens === 'instances' ? (
                  <InstancesPanel />
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
                      vaultId={across ? null : vaultId}
                      onChanged={refreshContents}
                    />
                  )
                )}
              </ScrollArea>
            ) : (
            <div className="grid min-h-0 flex-1 grid-cols-[18rem_minmax(0,1fr)] grid-rows-[minmax(0,1fr)]">
              <ScrollArea className="border-r">
                <div className="flex items-baseline justify-between gap-2 px-4 pt-4 pb-2">
                  <h2 className="truncate text-lg font-bold tracking-tight">{listTitle}</h2>
                  <span className="shrink-0 text-xs text-muted-foreground">
                    {visible.length} {visible.length === 1 ? 'registro' : 'registros'}
                  </span>
                </div>
                <ul className="space-y-0.5 px-2 pb-2">
                  {visible.map((record) => {
                    const Icon = kindIcon(record.kind)
                    const attention = needsAttention(record)
                    const picked = selected.includes(record.id)

                    return (
                      <li key={record.id}>
                        <RowMenu entries={recordEntries(record)}>
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
                            'group flex w-full items-center gap-3 rounded-lg px-2.5 py-2 text-sm transition-colors',
                            record.id === activeTab ? 'bg-accent text-accent-foreground' : 'hover:bg-hover',
                            dragging === record.id && 'opacity-50',
                          )}
                        >
                          {/*
                            * The checkbox takes the icon's place on hover, rather than sitting in a
                            * column of its own: a permanent column of checkboxes turns a list you
                            * read into a form you fill in.
                            */}
                          <span className="relative grid size-9 shrink-0 place-items-center">
                            <span
                              className={cn(
                                'absolute inset-0 grid place-items-center rounded-lg transition-opacity',
                                kindTone(record.kind),
                                (picked || selected.length > 0) && 'opacity-0',
                                'group-hover:opacity-0',
                              )}
                            >
                              <Icon className="size-[18px]" />
                            </span>
                            <Checkbox
                              checked={picked}
                              aria-label={`Selecionar ${record.title}`}
                              className={cn(
                                'relative size-4 opacity-0 transition-opacity group-hover:opacity-100',
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
                            className="min-w-0 flex-1 text-left"
                          >
                            <span className="block truncate font-semibold">{record.title}</span>
                            <span className="block truncate text-xs text-muted-foreground">
                              {subtitleOf(record)}
                            </span>
                          </button>

                          {/* Which vault a row is from only matters when the list mixes them. */}
                          {across && (
                            <span className="shrink-0 text-xs text-muted-foreground">
                              {nameOfVault(record.vault_id)}
                            </span>
                          )}

                          {record.favourite && (
                            <Star className="size-3.5 shrink-0 fill-current text-primary" />
                          )}
                          {attention && (
                            <AlertTriangle
                              className={cn(
                                'size-3 shrink-0',
                                record.compromised_at ? 'text-destructive' : 'text-muted-foreground',
                              )}
                            />
                          )}

                          {/* The same actions as the right-click, as something to find. A gesture
                              nobody is told about is a gesture nobody uses. */}
                          <span className="shrink-0 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100 has-aria-expanded:opacity-100">
                            <MenuButton
                              entries={recordEntries(record)}
                              label={`Ações de ${record.title}`}
                            />
                          </span>
                        </div>
                        </RowMenu>
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
                  <div className="flex h-11 shrink-0 items-stretch gap-1 overflow-x-auto overflow-y-hidden border-b px-3">
                    {tabs.map((tab) => (
                      <div
                        key={tab.id}
                        className={cn(
                          '-mb-px flex items-center gap-1.5 border-b-2 px-3 text-sm',
                          tab.id === activeTab
                            ? 'border-primary font-semibold text-foreground'
                            : 'border-transparent text-muted-foreground hover:text-foreground',
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
                      onClone={(id) => void cloneRecord(id)}
                      onMoveToVault={() => {
                        const open = tabs.find((tab) => tab.id === activeTab)!
                        setMovingToVault({ ids: [open.id], what: `“${open.payload.title}”` })
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
                      {activeVault || across
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
          cloning={creating.cloning}
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

      {deleting && (
        <AlertDialog open onOpenChange={(open) => !open && setDeleting(null)}>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>Excluir “{deleting.title}”?</AlertDialogTitle>
              <AlertDialogDescription>
                Vai para a lixeira, e de lá dá para restaurar enquanto o prazo não vencer.
              </AlertDialogDescription>
            </AlertDialogHeader>

            <AlertDialogFooter>
              <AlertDialogCancel>Cancelar</AlertDialogCancel>
              <AlertDialogAction
                onClick={() => {
                  const id = deleting.id
                  setDeleting(null)
                  void deleteRecord(id)
                }}
              >
                Excluir
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      )}

      {movingToVault && (
        <MoveToVaultDialog
          ids={movingToVault.ids}
          what={movingToVault.what}
          vaults={vaults}
          fromVaultId={across ? null : vaultId}
          onDone={() => {
            // Whatever moved is in another vault now, so any tab showing it is a tab onto a record
            // this screen no longer lists.
            const moved = new Set(movingToVault.ids)

            setTabs((current) => current.filter((tab) => !moved.has(tab.id)))
            setActiveTab((current) => (current && moved.has(current) ? null : current))
            setSelected([])
            setMovingToVault(null)

            void refreshContents()
          }}
          onCancel={() => setMovingToVault(null)}
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
    <header className="flex h-14 shrink-0 items-center gap-2.5 border-b px-4">
      {title && <h1 className="text-base font-semibold">{title}</h1>}
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

const INDENT = ['pl-2.5', 'pl-7', 'pl-11', 'pl-15', 'pl-[4.75rem]', 'pl-[5.75rem]']

function TreeRow({
  depth = 0,
  selected,
  colour,
  icon: Icon,
  count,
  dropping,
  onClick,
  onDragOver,
  onDragLeave,
  onDrop,
  entries,
  menuLabel,
  children,
}: {
  depth?: number
  selected: boolean
  colour?: string | null
  icon?: LucideIcon
  count?: number
  dropping?: boolean
  onClick: () => void
  onDragOver?: (event: React.DragEvent) => void
  onDragLeave?: () => void
  onDrop?: (event: React.DragEvent) => void
  entries?: MenuEntry[]
  menuLabel?: string
  children: React.ReactNode
}) {
  const row = (
    <div
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      className={cn(
        'group flex h-[30px] items-center rounded-md pr-1.5 transition-colors',
        dropping && 'ring-2 ring-ring',
        // Static classes rather than a computed padding: Tailwind only ships the classes it can see
        // in the source, and an interpolated one would simply not exist in the stylesheet.
        INDENT[Math.min(depth, INDENT.length - 1)],
        selected
          ? 'bg-sidebar-accent font-semibold text-sidebar-accent-foreground'
          : 'text-sidebar-muted hover:bg-sidebar-hover hover:text-sidebar-foreground',
      )}
    >
      <button
        type="button"
        onClick={onClick}
        className="flex min-w-0 flex-1 items-center gap-2.5 text-left text-sm"
      >
        {Icon ? (
          <Icon className="size-4 shrink-0" />
        ) : (
          colour !== undefined && <Folder className={cn('size-4 shrink-0', colourOf(colour).text)} />
        )}
        <span className="truncate">{children}</span>
      </button>

      {count !== undefined && (
        <span
          className={cn(
            'shrink-0 pl-2 text-xs tabular-nums opacity-75',
            // The count gives way to the menu on hover, so the row never grows a column.
            entries && 'group-hover:hidden group-has-aria-expanded:hidden',
          )}
        >
          {count}
        </span>
      )}

      {/* The menu appears on hover and stays put once opened, so it never moves out from under a
          pointer that is already on its way to it. */}
      {entries && (
        <span className="hidden shrink-0 group-hover:block group-has-aria-expanded:block focus-within:block">
          <MenuButton entries={entries} label={menuLabel ?? 'Ações'} />
        </span>
      )}
    </div>
  )

  return entries ? <RowMenu entries={entries}>{row}</RowMenu> : row
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
  return cause instanceof ApiError ? cause.message : 'Não foi possível falar com o Keyhold.'
}
