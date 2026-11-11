import { useMemo, useState } from 'react'
import { ChevronDown, ChevronRight, ChevronsDownUp, ChevronsUpDown, Folder, FolderOpen, List, ListTree, Search, Star, Trash2 } from 'lucide-react'

import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'

import type { FolderNode, RecordPreview } from '../../api'
import { colourOf } from '../colours'
import { RecordTile } from '../icons'
import { MenuButton, RowMenu, type MenuEntry } from '../menu'
import { linkProps, TabMenu } from '../nav'
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
  folderEntries,
  recordEntries,
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
  onDrop: (recordId: string, folderId: string | null) => void
  folderEntries: (folder: FolderNode) => MenuEntry[]
  recordEntries: (record: RecordPreview) => MenuEntry[]
}) {
  const [query, setQuery] = useState('')
  const [closed, setClosed] = useState<Set<string>>(new Set())
  const allClosed = folders.length > 0 && folders.every((folder) => closed.has(folder.id))
  const [dropping, setDropping] = useState<string | null | undefined>(undefined)

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

  function dropProps(folderId: string | null) {
    return {
      onDragOver: (event: React.DragEvent) => {
        if (!event.dataTransfer.types.includes('text/keyhold-record')) return
        event.preventDefault()
        setDropping(folderId)
      },
      onDragLeave: () => setDropping(undefined),
      onDrop: (event: React.DragEvent) => {
        event.preventDefault()
        setDropping(undefined)
        const id = event.dataTransfer.getData('text/keyhold-record')
        if (id) onDrop(id, folderId)
      },
    }
  }

  function recordRow(record: RecordPreview, depth: number, flat = false) {
    const on = selection?.kind === 'record' && selection.id === record.id

    return (
      <RowMenu key={record.id} entries={recordEntries(record)}>
        <div
          role="button"
          tabIndex={0}
          draggable
          onDragStart={(event) => {
            event.dataTransfer.setData('text/keyhold-record', record.id)
            event.dataTransfer.effectAllowed = 'move'
          }}
          {...linkProps({ place: 'vault', vault: vaultId, record: record.id }, () => onRecord(record.id))}
          onKeyDown={(event) => event.key === 'Enter' && onRecord(record.id)}
          style={flat ? undefined : { paddingLeft: 8 + depth * 22 + (depth > 0 ? 0 : 0) }}
          className={cn(
            'group flex cursor-pointer items-center gap-2 rounded-lg pr-2 outline-none hover:bg-hover focus-visible:ring-2 focus-visible:ring-ring/50',
            flat ? 'h-14 px-2.5' : 'h-[34px]',
            on && 'bg-primary-soft hover:bg-primary-soft',
          )}
        >
          {!flat && <span className="w-4 shrink-0" />}
          <RecordTile icon={record.icon} kind={record.kind} size={flat ? 'sm' : 'xs'} />
          <span className="min-w-0 flex-1">
            <span className={cn('block truncate text-sm', flat && 'font-medium')}>{record.title}</span>
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

    return (
      <div key={folder.id}>
        <RowMenu entries={folderEntries(folder)}>
          <div
            {...dropProps(folder.id)}
            style={{ paddingLeft: 4 + folder.depth * 22 }}
            className={cn(
              'group flex h-[34px] items-center gap-1.5 rounded-lg pr-1.5 hover:bg-hover',
              on && 'bg-primary-soft hover:bg-primary-soft',
              dropping === folder.id && 'ring-2 ring-primary',
            )}
          >
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
            </button>
            <button
              type="button"
              {...linkProps({ place: 'vault', vault: vaultId, folder: folder.id }, () => onFolder(folder.id))}
              className="flex min-w-0 flex-1 items-center gap-2 text-left text-sm font-medium"
            >
              <Icon className={cn('size-4 shrink-0', folder.colour && folder.colour !== 'default' ? colourOf(folder.colour).text : 'text-faint')} />
              <span className="truncate">{folder.name}</span>
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
    <aside className="flex min-h-0 flex-col rounded-[14px] border bg-card">
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
          <button
            type="button"
            title={allClosed ? 'Expandir todas as pastas' : 'Recolher todas as pastas'}
            aria-label={allClosed ? 'Expandir todas as pastas' : 'Recolher todas as pastas'}
            onClick={() => setClosed(allClosed ? new Set() : new Set(folders.map((folder) => folder.id)))}
            className="grid h-8 w-8 shrink-0 place-items-center rounded-lg border bg-raised text-faint hover:text-foreground"
          >
            {allClosed ? <ChevronsUpDown className="size-[15px]" /> : <ChevronsDownUp className="size-[15px]" />}
          </button>
        )}
      </div>

      <div className="flex gap-1.5 border-b px-3 pb-2.5">
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

      <div
        className={cn('min-h-0 flex-1 overflow-y-auto px-1.5 py-2', dropping === null && 'bg-primary-soft/40')}
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
    <button
      type="button"
      title={label}
      aria-label={label}
      aria-pressed={on}
      onClick={onClick}
      className={cn('grid h-[26px] w-[30px] place-items-center rounded-md text-faint hover:text-foreground', on && 'bg-card text-foreground shadow-sm')}
    >
      {children}
    </button>
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
