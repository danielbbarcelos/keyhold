import { useState } from 'react'
import { Folder, FolderOpen, FolderPlus, Star, Vault } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

import type { FolderNode, RecordPreview } from '../../api'
import { colourOf } from '../colours'
import { RecordTile } from '../icons'
import { MenuButton, type MenuEntry } from '../menu'
import { Pill } from '../record/RecordView'
import { folderPath, when } from './paths'

/** A folder opened from the tree: what is in it, and what can be done to it. */
export function FolderView({
  folder,
  folders,
  records,
  vaultName,
  entries,
  onRecord,
  onFolder,
  onSubfolder,
}: {
  folder: FolderNode
  folders: FolderNode[]
  records: RecordPreview[]
  vaultName: string
  entries: MenuEntry[]
  onRecord: (id: string) => void
  onFolder: (id: string) => void
  onSubfolder: () => void
}) {
  const [tab, setTab] = useState<'registros' | 'subpastas'>('registros')
  const inside = records.filter((record) => record.folder_id === folder.id).sort((a, b) => a.title.localeCompare(b.title, 'pt-BR'))
  const children = folders.filter((child) => child.parent_id === folder.id)
  const parent = folderPath(folders, folder.parent_id)

  return (
    <div className="flex h-full min-h-0 flex-col overflow-auto rounded-[14px] border bg-card">
      <div className="flex items-start gap-4 px-5 pt-5 pb-4">
        <span className={cn('grid size-12 shrink-0 place-items-center rounded-xl', colourOf(folder.colour).tile)}>
          <FolderOpen className="size-[22px]" />
        </span>
        <div className="min-w-0 flex-1">
          <h2 className="truncate text-xl font-semibold tracking-tight">{folder.name}</h2>
          <div className="mt-1.5 flex flex-wrap gap-1.5">
            <Pill icon={Vault}>{parent ? `${vaultName}, em ${parent}` : `${vaultName}, na raiz`}</Pill>
            <Pill>
              {inside.length} {inside.length === 1 ? 'registro' : 'registros'}
            </Pill>
            <Pill>
              {children.length === 0 ? 'sem subpastas' : `${children.length} ${children.length === 1 ? 'subpasta' : 'subpastas'}`}
            </Pill>
          </div>
        </div>
        <Button variant="outline" size="sm" onClick={onSubfolder}>
          <FolderPlus />
          Subpasta
        </Button>
        <MenuButton entries={entries} label={`Ações da pasta ${folder.name}`} variant="outline" size="icon" align="end" />
      </div>

      <div className="flex gap-1 border-b px-5">
        {(
          [
            ['registros', 'Registros', inside.length],
            ['subpastas', 'Subpastas', children.length],
          ] as const
        ).map(([id, label, count]) => (
          <button
            key={id}
            type="button"
            onClick={() => setTab(id)}
            className={cn(
              '-mb-px flex h-10 items-center gap-1.5 border-b-2 px-2.5 text-sm font-medium',
              tab === id ? 'border-primary text-foreground' : 'border-transparent text-muted-foreground hover:text-foreground',
            )}
          >
            {label}
            <span className="text-xs text-faint">{count}</span>
          </button>
        ))}
      </div>

      {tab === 'registros' ? (
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b text-left text-xs text-faint">
              <th className="px-5 py-2.5 font-normal">Registro</th>
              <th className="px-3 py-2.5 font-normal">Alterado</th>
              <th className="px-5 py-2.5" />
            </tr>
          </thead>
          <tbody>
            {inside.map((record) => (
              <tr key={record.id} className="border-b last:border-b-0 hover:bg-hover">
                <td className="px-5 py-2.5">
                  <button type="button" onClick={() => onRecord(record.id)} className="flex items-center gap-3 text-left">
                    <RecordTile icon={record.icon} kind={record.kind} />
                    <span className="min-w-0">
                      <b className="flex items-center gap-1.5 font-medium">
                        {record.title}
                        {record.favourite && <Star className="size-3 fill-primary text-primary" />}
                      </b>
                      {record.subtitle && <span className="block text-xs text-faint">{record.subtitle}</span>}
                    </span>
                  </button>
                </td>
                <td className="px-3 py-2.5 text-muted-foreground">{when(record.updated_at)}</td>
                <td className="px-5 py-2.5 text-right">
                  <Button variant="outline" size="sm" onClick={() => onRecord(record.id)}>
                    Abrir
                  </Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : (
        <div className="p-2">
          {children.map((child) => (
            <button
              key={child.id}
              type="button"
              onClick={() => onFolder(child.id)}
              className="flex h-11 w-full items-center gap-3 rounded-lg px-3 text-left text-sm hover:bg-hover"
            >
              <Folder className={cn('size-4', colourOf(child.colour).text)} />
              <span className="flex-1 font-medium">{child.name}</span>
              <span className="text-xs text-faint">
                {records.filter((record) => record.folder_id === child.id).length} registros
              </span>
            </button>
          ))}
        </div>
      )}

      {tab === 'registros' && inside.length === 0 && (
        <p className="px-5 py-6 text-sm text-faint">Nenhum registro nesta pasta. Arraste um da árvore para cá, ou crie um novo.</p>
      )}
      {tab === 'subpastas' && children.length === 0 && (
        <p className="px-5 py-4 text-sm text-faint">Nenhuma subpasta. "Subpasta", acima, cria uma.</p>
      )}
    </div>
  )
}
