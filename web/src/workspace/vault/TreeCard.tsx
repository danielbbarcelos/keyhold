import { useEffect, useMemo, useState } from 'react'
import { ChevronDown, ChevronRight, ChevronsDownUp, ChevronsUpDown, Folder, FolderOpen, List, ListTree, Search, Star, Trash2 } from 'lucide-react'

import { Input } from '@/components/ui/input'
import { Tip } from '@/components/ui/tip'
import { cn } from '@/lib/utils'

import type { FolderNode, RecordPreview } from '../../api'
import { colourOf } from '../colours'
import { RecordTile } from '../icons'
import { MenuButton, RowMenu, type MenuEntry } from '../menu'
import { linkProps, TabMenu } from '../nav'
import { Clip } from '../Splitter'
import { AttentionDot, type Attention } from './Attention'
import { folderPath } from './paths'

/**
 * The vault's contents, always on the left while you are inside it: folders and records together
 * as a tree, or every record as a flat list with its folder under the title.
 *
 * Favourites and Atenção are filters on top of the same card rather than places of their own, and
 * the trash is a fixed line at its foot. Records are dragged onto folders to move them.
 */
export type TreeMode = 'tree' | 'list'
export type TreeFilter = 'all' | 'favourites' | 'attention'

export type Selection = { kind: 'record' | 'folder'; id: string } | { kind: 'trash' } | null

export function TreeCard({
  vaultId,
  folders,
  records,
  attention,
  onEditRecord,
  onClearCompromised,
  selection,
  mode,
  filter,
  trashCount,
  showTrash = true,
  onMode,
  onFilter,
  onRecord,
  onFolder,
  onTrash,
  onDrop,
  canMoveFolders = true,
  pickedEntries,
  folderEntries,
  recordEntries,
  rootEntries,
}: {
  folders: FolderNode[]
  records: RecordPreview[]
  attention: Attention
  /** The attention popover's "Editar". */
  onEditRecord: (id: string) => void
  /** The attention popover's "Não está mais comprometido". */
  onClearCompromised: (id: string) => void
  selection: Selection
  mode: TreeMode
  filter: TreeFilter
  trashCount: number
  showTrash?: boolean
  onMode: (mode: TreeMode) => void
  onFilter: (filter: TreeFilter) => void
  /** For Ctrl click and the middle button, which open the row in a new tab. */
  vaultId: string
  onRecord: (id: string) => void
  onFolder: (id: string) => void
  onTrash: () => void
  /** Records (`r:<id>`) and folders (`f:<id>`) dropped on a folder, or on the root (`null`). */
  onDrop: (items: string[], folderId: string | null) => void
  /** Folders can be dragged too (not in AWS, where a folder is part of every name under it). */
  canMoveFolders?: boolean
  /** What the "Ações" menu offers for the picked items; `clear` empties the selection afterwards. */
  pickedEntries: (items: string[], clear: () => void) => MenuEntry[]
  /** Right-click on the empty space of the tree: what can be made at the root. */
  rootEntries: MenuEntry[]
  folderEntries: (folder: FolderNode) => MenuEntry[]
  recordEntries: (record: RecordPreview) => MenuEntry[]
}) {
  const [query, setQuery] = useState('')
  const [closed, setClosed] = useState<Set<string>>(new Set())
  const allClosed = folders.length > 0 && folders.every((folder) => closed.has(folder.id))
  const [dropping, setDropping] = useState<string | null | undefined>(undefined)
  // Items picked with Ctrl/⌘ or Shift click, to drag together: `r:<id>` and `f:<id>`.
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [anchor, setAnchor] = useState<string | null>(null)

  // Everything inside a picked folder goes with it: its subfolders and records are shown as
  // picked too, a shade lighter.
  const within = useMemo(() => {
    const inside = new Set<string>()
    const add = (parentId: string) => {
      for (const child of folders.filter((folder) => folder.parent_id === parentId)) {
        inside.add(`f:${child.id}`)
        add(child.id)
      }
    }
    for (const item of picked) if (item.startsWith('f:')) add(item.slice(2))
    const folderIds = new Set([...picked, ...inside].filter((item) => item.startsWith('f:')).map((item) => item.slice(2)))
    for (const record of records) if (record.folder_id && folderIds.has(record.folder_id)) inside.add(`r:${record.id}`)
    return inside
  }, [picked, folders, records])

  useEffect(() => {
    if (picked.size === 0) return
    const clear = (event: KeyboardEvent) => event.key === 'Escape' && setPicked(new Set())
    window.addEventListener('keydown', clear)
    return () => window.removeEventListener('keydown', clear)
  }, [picked.size])

  const term = query.trim().toLowerCase()
  const narrowed = filter !== 'all' || term !== ''

  const shown = useMemo(
    () =>
      records.filter((record) => {
        if (filter === 'favourites' && !record.favourite) return false
        if (filter === 'attention' && !attention.has(record.id)) return false
        if (term && !`${record.title} ${record.subtitle} ${record.url}`.toLowerCase().includes(term)) return false

        return true
      }),
    [records, filter, attention, term],
  )

  const counts = {
    all: records.length,
    favourites: records.filter((record) => record.favourite).length,
    attention: records.filter((record) => attention.has(record.id)).length,
  }

  /** While narrowing, only the folders that lead to something shown stay in the tree. */
  const leadsSomewhere = (folderId: string): boolean =>
    shown.some((record) => record.folder_id === folderId) ||
    folders.some((child) => child.parent_id === folderId && leadsSomewhere(child.id))

  const countIn = (folderId: string): number =>
    records.filter((record) => record.folder_id === folderId).length +
    folders.filter((child) => child.parent_id === folderId).reduce((total, child) => total + countIn(child.id), 0)

  const sortRecords = (list: RecordPreview[]) => [...list].sort((a, b) => a.title.localeCompare(b.title, 'pt-BR'))

  /** Every row in the order it is drawn, for Shift click. */
  function order(): string[] {
    if (mode === 'list') return sortRecords(shown).map((record) => `r:${record.id}`)

    const out: string[] = []
    const walk = (folder: FolderNode) => {
      if (narrowed && !leadsSomewhere(folder.id)) return
      out.push(`f:${folder.id}`)
      if (!narrowed && closed.has(folder.id)) return
      folders.filter((child) => child.parent_id === folder.id).forEach(walk)
      sortRecords(shown.filter((record) => record.folder_id === folder.id)).forEach((record) => out.push(`r:${record.id}`))
    }
    folders.filter((folder) => folder.parent_id === null).forEach(walk)
    sortRecords(shown.filter((record) => record.folder_id === null)).forEach((record) => out.push(`r:${record.id}`))
    return out
  }

  /**
   * A click on a row: Ctrl/⌘ picks or unpicks it, Shift picks everything from the last pick to it.
   * `true` when the click was a pick, so the row does not also open.
   */
  function pick(key: string, event: React.MouseEvent): boolean {
    if (event.ctrlKey || event.metaKey) {
      setPicked((was) => {
        const next = new Set(was)
        if (next.has(key)) next.delete(key)
        else next.add(key)
        return next
      })
      setAnchor(key)
      return true
    }

    if (event.shiftKey) {
      const all = order()
      const from = all.indexOf(anchor ?? key)
      const to = all.indexOf(key)
      if (from >= 0 && to >= 0) {
        const [a, b] = from < to ? [from, to] : [to, from]
        setPicked(new Set(all.slice(a, b + 1)))
      }
      return true
    }

    if (picked.size) setPicked(new Set())
    setAnchor(key)
    return false
  }

  /** What a drag carries: the picked items when the row is one of them, the row alone otherwise. */
  function startDrag(key: string, event: React.DragEvent) {
    const items = picked.has(key) ? [...picked] : [key]
    event.dataTransfer.setData('text/keyhold-items', JSON.stringify(items))
    event.dataTransfer.effectAllowed = 'move'

    if (items.length > 1) {
      const badge = document.createElement('div')
      badge.textContent = `${items.length} itens`
      badge.style.cssText =
        'position:fixed;top:-100px;left:0;padding:6px 12px;border-radius:999px;font:600 13px system-ui;background:var(--primary);color:var(--primary-foreground)'
      document.body.appendChild(badge)
      event.dataTransfer.setDragImage(badge, 10, 10)
      window.setTimeout(() => badge.remove(), 0)
    }
  }

  function dropProps(folderId: string | null) {
    return {
      onDragOver: (event: React.DragEvent) => {
        if (!event.dataTransfer.types.includes('text/keyhold-items')) return
        event.preventDefault()
        event.stopPropagation()
        setDropping(folderId)
      },
      onDragLeave: () => setDropping(undefined),
      onDrop: (event: React.DragEvent) => {
        event.preventDefault()
        event.stopPropagation()
        setDropping(undefined)
        try {
          const items = JSON.parse(event.dataTransfer.getData('text/keyhold-items')) as string[]
          if (items.length) {
            onDrop(items, folderId)
            setPicked(new Set())
          }
        } catch {
          /* not ours */
        }
      },
    }
  }

  function recordRow(record: RecordPreview, depth: number, flat = false) {
    const on = selection?.kind === 'record' && selection.id === record.id
    const key = `r:${record.id}`
    const link = linkProps({ place: 'vault', vault: vaultId, record: record.id }, () => onRecord(record.id))

    return (
      <RowMenu key={record.id} entries={recordEntries(record)}>
        <div
          role="button"
          tabIndex={0}
          draggable
          aria-selected={picked.has(key)}
          onDragStart={(event) => startDrag(key, event)}
          {...dropProps(record.folder_id)}
          onClick={(event) => !pick(key, event) && onRecord(record.id)}
          onAuxClick={link.onAuxClick}
          onMouseDown={(event) => {
            if (event.shiftKey) event.preventDefault()
            link.onMouseDown(event)
          }}
          onKeyDown={(event) => event.key === 'Enter' && onRecord(record.id)}
          style={flat ? undefined : { paddingLeft: 8 + depth * 22 + (depth > 0 ? 0 : 0) }}
          className={cn(
            'group flex cursor-pointer items-center gap-2 rounded-lg pr-2 outline-none hover:bg-hover focus-visible:ring-2 focus-visible:ring-ring/50',
            flat ? 'h-14 px-2.5' : 'h-[34px]',
            on && 'bg-primary-soft hover:bg-primary-soft',
            picked.has(key) && 'bg-primary-soft/70 ring-1 ring-primary/50 ring-inset',
            !picked.has(key) && within.has(key) && 'bg-primary-soft/35',
          )}
        >
          {!flat && <span className="w-4 shrink-0" />}
          <RecordTile icon={record.icon} kind={record.kind} size={flat ? 'sm' : 'xs'} />
          <span className="min-w-0 flex-1">
            <Clip className={cn('text-sm', flat && 'font-medium')}>{record.title}</Clip>
            {flat && (
              <span className="flex items-center gap-1 truncate text-xs text-faint">
                <Folder className="size-3 shrink-0" />
                <span className="truncate">
                  {folderPath(folders, record.folder_id) || 'Raiz'}
                  {record.subtitle && `, ${record.subtitle}`}
                </span>
              </span>
            )}
          </span>
          {attention.has(record.id) && (
            <AttentionDot
              concern={attention.get(record.id)!}
              onEdit={() => onEditRecord(record.id)}
              onClearCompromised={() => onClearCompromised(record.id)}
            />
          )}
          {record.favourite && <Star className="size-[13px] shrink-0 fill-primary text-primary" />}
        </div>
      </RowMenu>
    )
  }

  function folderBranch(folder: FolderNode): React.ReactNode {
    if (narrowed && !leadsSomewhere(folder.id)) return null

    const open = narrowed || !closed.has(folder.id)
    const on = selection?.kind === 'folder' && selection.id === folder.id
    const children = folders.filter((child) => child.parent_id === folder.id)
    const inside = sortRecords(shown.filter((record) => record.folder_id === folder.id))
    const Icon = open ? FolderOpen : Folder
    const key = `f:${folder.id}`
    const link = linkProps({ place: 'vault', vault: vaultId, folder: folder.id }, () => onFolder(folder.id))

    return (
      <div key={folder.id}>
        <RowMenu entries={folderEntries(folder)}>
          <div
            {...dropProps(folder.id)}
            draggable={canMoveFolders}
            onDragStart={canMoveFolders ? (event) => startDrag(key, event) : undefined}
            aria-selected={picked.has(key)}
            style={{ paddingLeft: 4 + folder.depth * 22 }}
            className={cn(
              'group flex h-[34px] items-center gap-1.5 rounded-lg pr-1.5 hover:bg-hover',
              on && 'bg-primary-soft hover:bg-primary-soft',
              picked.has(key) && 'bg-primary-soft/70 ring-1 ring-primary/50 ring-inset',
              !picked.has(key) && within.has(key) && 'bg-primary-soft/35',
              dropping === folder.id && 'ring-2 ring-primary',
            )}
          >
<Tip label={open ? 'Recolher' : 'Expandir'}>
            <button
              type="button"
              aria-label={open ? 'Recolher' : 'Expandir'}
              onClick={() =>
                setClosed((current) => {
                  const next = new Set(current)
                  if (next.has(folder.id)) next.delete(folder.id)
                  else next.add(folder.id)
                  return next
                })
              }
              className="grid size-5 place-items-center rounded text-faint hover:text-foreground"
            >
              {open ? <ChevronDown className="size-3.5" /> : <ChevronRight className="size-3.5" />}
            </button></Tip>
            <button
              type="button"
              onClick={(event) => !pick(key, event) && onFolder(folder.id)}
              onAuxClick={link.onAuxClick}
              onMouseDown={(event) => {
                if (event.shiftKey) event.preventDefault()
                link.onMouseDown(event)
              }}
              className="flex min-w-0 flex-1 items-center gap-2 text-left text-sm font-medium"
            >
              <Icon className={cn('size-4 shrink-0', folder.colour && folder.colour !== 'default' ? colourOf(folder.colour).text : 'text-faint')} />
              <Clip>{folder.name}</Clip>
            </button>
            <span className="text-xs text-faint group-hover:hidden group-has-aria-expanded:hidden">{countIn(folder.id)}</span>
            <span className="hidden group-hover:block group-has-aria-expanded:block">
              <MenuButton entries={folderEntries(folder)} label={`Ações da pasta ${folder.name}`} />
            </span>
          </div>
        </RowMenu>
        {open && (
          <>
            {children.map(folderBranch)}
            {inside.map((record) => recordRow(record, folder.depth + 1))}
          </>
        )}
      </div>
    )
  }

  const roots = folders.filter((folder) => folder.parent_id === null)
  const loose = sortRecords(shown.filter((record) => record.folder_id === null))

  return (
    <aside className="flex min-h-0 min-w-0 flex-col rounded-[14px] border bg-card">
      <div className="flex items-center gap-2 px-3 pt-3 pb-2.5">
        <div className="relative min-w-0 flex-1">
          <Search className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-faint" />
          <Input
            className="h-8 pl-8"
            placeholder="Filtrar neste cofre"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
        </div>
        <div className="flex rounded-lg border bg-raised p-0.5">
          <ModeButton on={mode === 'tree'} label="Árvore" onClick={() => onMode('tree')}>
            <ListTree className="size-[15px]" />
          </ModeButton>
          <ModeButton on={mode === 'list'} label="Lista" onClick={() => onMode('list')}>
            <List className="size-[15px]" />
          </ModeButton>
        </div>
        {mode === 'tree' && folders.length > 0 && (
<Tip label={allClosed ? 'Expandir todas as pastas' : 'Recolher todas as pastas'}>
          <button
            type="button"
            aria-label={allClosed ? 'Expandir todas as pastas' : 'Recolher todas as pastas'}
            onClick={() => setClosed(allClosed ? new Set() : new Set(folders.map((folder) => folder.id)))}
            className="grid h-8 w-8 shrink-0 place-items-center rounded-lg border bg-raised text-faint hover:text-foreground"
          >
            {allClosed ? <ChevronsUpDown className="size-[15px]" /> : <ChevronsDownUp className="size-[15px]" />}
          </button></Tip>
        )}
      </div>

      <div className="flex flex-wrap gap-1.5 border-b px-3 pb-2.5">
        <FilterChip on={filter === 'all'} count={counts.all} onClick={() => onFilter('all')}>
          Tudo
        </FilterChip>
        <FilterChip on={filter === 'favourites'} count={counts.favourites} onClick={() => onFilter('favourites')}>
          Favoritos
        </FilterChip>
        <FilterChip on={filter === 'attention'} count={counts.attention} onClick={() => onFilter('attention')}>
          Atenção
        </FilterChip>

      </div>

      {picked.size > 0 && (
        <div className="flex items-center gap-2 border-b bg-primary-soft/50 px-3 py-2 text-xs">
          <b className="min-w-0 flex-1 truncate font-semibold">
            {picked.size} {picked.size === 1 ? 'selecionado' : 'selecionados'}
          </b>
          <MenuButton
            text="Ações"
            label="Ações para os itens selecionados"
            variant="outline"
            align="end"
            entries={pickedEntries([...picked], () => setPicked(new Set()))}
          />
          <button type="button" className="shrink-0 text-primary-text hover:underline" onClick={() => setPicked(new Set())}>
            Limpar
          </button>
        </div>
      )}

      <div
        className={cn('flex min-h-0 flex-1 flex-col overflow-y-auto px-1.5 py-2', dropping === null && 'bg-primary-soft/40')}
        {...dropProps(null)}
      >
        {mode === 'tree' ? (
          <>
            {roots.map(folderBranch)}
            {loose.map((record) => recordRow(record, 0))}
          </>
        ) : (
          <>
            <div className="flex justify-between px-2.5 pt-1 pb-1.5 text-xs text-faint">
              <span>{shown.length} {shown.length === 1 ? 'registro' : 'registros'}</span>
              <span>A a Z</span>
            </div>
            {sortRecords(shown).map((record) => recordRow(record, 0, true))}
          </>
        )}

        {shown.length === 0 && (
          <p className="px-3 py-6 text-center text-[13px] text-faint">
            {records.length === 0
              ? 'Este cofre ainda está vazio. "Novo registro", no topo, cria o primeiro.'
              : filter === 'favourites'
                ? 'Nenhum favorito aqui. A estrela de um registro o traz para cá.'
                : filter === 'attention'
                  ? 'Nada pede atenção neste cofre.'
                  : 'Nada com esse nome.'}
          </p>
        )}
        {/* The empty space under the rows is the vault's root: right-click there to make something in it. */}
        <RowMenu entries={rootEntries}>
          <div className="min-h-10 flex-1" aria-hidden="true" />
        </RowMenu>
      </div>

      {showTrash ? (
      <TabMenu to={{ place: 'vault', vault: vaultId, trash: true }}>
      <button
        type="button"
        {...linkProps({ place: 'vault', vault: vaultId, trash: true }, onTrash)}
        className={cn(
          'flex h-11 shrink-0 items-center gap-2.5 rounded-b-[14px] border-t px-4 text-sm text-muted-foreground hover:bg-hover hover:text-foreground',
          selection?.kind === 'trash' && 'bg-primary-soft text-foreground hover:bg-primary-soft',
        )}
      >
        <Trash2 className="size-4" />
        Lixeira
        <span className="ml-auto text-xs text-faint">{trashCount}</span>
      </button>
      </TabMenu>
      ) : (
        <p className="flex h-11 shrink-0 items-center gap-2 border-t px-4 text-xs text-faint">
          Na AWS não há lixeira: apagar é de vez.
        </p>
      )}
    </aside>
  )
}

function ModeButton({ on, label, onClick, children }: { on: boolean; label: string; onClick: () => void; children: React.ReactNode }) {
  return (
<Tip label={label}>
    <button
      type="button"
      aria-label={label}
      aria-pressed={on}
      onClick={onClick}
      className={cn('grid h-[26px] w-[30px] place-items-center rounded-md text-faint hover:text-foreground', on && 'bg-card text-foreground shadow-sm')}
    >
      {children}
    </button></Tip>
  )
}

function FilterChip({ on, count, onClick, children }: { on: boolean; count: number; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      aria-pressed={on}
      onClick={onClick}
      className={cn(
        'inline-flex h-[26px] items-center gap-1.5 rounded-full border px-2.5 text-xs font-medium',
        on ? 'border-transparent bg-primary-soft text-primary-text' : 'text-muted-foreground hover:bg-hover',
      )}
    >
      {children}
      <span className={on ? 'opacity-70' : 'text-faint'}>{count}</span>
    </button>
  )
}
