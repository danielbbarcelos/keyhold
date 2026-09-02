import { useState } from 'react'
import { FolderInput, Star, Trash2, X } from 'lucide-react'

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
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'

import { api, type FolderNode } from '../api'

/**
 * What you can do to several records at once.
 *
 * It appears only when something is selected, in the place the toolbar would be, because a bar of
 * bulk actions sitting there permanently is a bar that eventually gets clicked with the wrong thing
 * selected. Deleting asks first — one click here is worth many elsewhere.
 */
interface BulkBarProps {
  selected: string[]
  folders: FolderNode[]
  onClear: () => void
  onChanged: () => void
  onMoveToVault: () => void
}

export function BulkBar({ selected, folders, onClear, onChanged, onMoveToVault }: BulkBarProps) {
  const [confirmingDelete, setConfirmingDelete] = useState(false)

  async function each(action: (id: string) => Promise<unknown>) {
    // Sequential rather than in parallel: these all land in the same SQLite file, and a burst of
    // concurrent writes buys nothing on a list somebody selected by hand.
    for (const id of selected) {
      await action(id)
    }

    onClear()
    onChanged()
  }

  return (
    <>
      <span className="text-sm text-muted-foreground">
        {selected.length} {selected.length === 1 ? 'selecionado' : 'selecionados'}
      </span>

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="outline" size="sm">
            <FolderInput />
            Mover para
          </Button>
        </DropdownMenuTrigger>

        <DropdownMenuContent align="start">
          <DropdownMenuItem onSelect={() => void each((id) => api.moveRecord(id, null))}>
            Raiz do cofre
          </DropdownMenuItem>
          {folders.map((folder) => (
            <DropdownMenuItem
              key={folder.id}
              onSelect={() => void each((id) => api.moveRecord(id, folder.id))}
            >
              {'— '.repeat(folder.depth)}
              {folder.name}
            </DropdownMenuItem>
          ))}

          {/* Another vault is not another folder: it re-encrypts, so it asks in its own dialog. */}
          <DropdownMenuSeparator />
          <DropdownMenuItem onSelect={onMoveToVault}>Outro cofre…</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <Button
        variant="outline"
        size="sm"
        onClick={() => void each((id) => api.favouriteRecord(id, true))}
      >
        <Star />
        Favoritar
      </Button>

      <Button variant="destructive" size="sm" onClick={() => setConfirmingDelete(true)}>
        <Trash2 />
        Excluir
      </Button>

      <Button variant="ghost" size="icon-sm" aria-label="Limpar a seleção" onClick={onClear}>
        <X />
      </Button>

      <AlertDialog open={confirmingDelete} onOpenChange={setConfirmingDelete}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              Excluir {selected.length} {selected.length === 1 ? 'registro' : 'registros'}?
            </AlertDialogTitle>
            <AlertDialogDescription>
              Eles vão para a lixeira, e de lá dá para restaurar enquanto o prazo não vencer.
            </AlertDialogDescription>
          </AlertDialogHeader>

          <AlertDialogFooter>
            <AlertDialogCancel>Cancelar</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                setConfirmingDelete(false)
                void each((id) => api.archiveRecord(id, true))
              }}
            >
              Excluir
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
