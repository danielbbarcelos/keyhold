import { useState } from 'react'

import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'

import { api, ApiError, type FolderNode } from '../api'
import { ColourPicker } from './ColourPicker'

/**
 * Renaming, moving and deleting a folder.
 *
 * These three existed in the binary from the day folders did, and were reachable only through the
 * API — which made the tree something you could build and never correct. A folder you cannot rename
 * is a typo you live with.
 */

export function RenameFolderDialog({
  folder,
  onDone,
  onCancel,
}: {
  folder: FolderNode
  onDone: () => void
  onCancel: () => void
}) {
  const [name, setName] = useState(folder.name)
  const [colour, setColour] = useState(folder.colour ?? 'default')
  const [error, setError] = useState('')

  return (
    <AlertDialog open onOpenChange={(open) => !open && onCancel()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Renomear a pasta</AlertDialogTitle>
          <AlertDialogDescription>
            O nome e a cor ficam cifrados como o resto do cofre.
          </AlertDialogDescription>
        </AlertDialogHeader>

        <div className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="folder-name">Nome</Label>
            <Input
              id="folder-name"
              value={name}
              autoFocus
              onChange={(event) => setName(event.target.value)}
            />
          </div>

          <ColourPicker value={colour} onChange={setColour} />

          {error && <p className="text-sm text-destructive">{error}</p>}
        </div>

        <AlertDialogFooter>
          <AlertDialogCancel>Cancelar</AlertDialogCancel>
          <Button
            disabled={!name.trim()}
            onClick={() =>
              void api
                .renameFolder(folder.id, name.trim(), colour)
                .then(onDone)
                .catch((cause) => setError(describe(cause)))
            }
          >
            Salvar
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}

/**
 * Moving a folder.
 *
 * The list of destinations leaves out the folder itself and everything inside it, because those are
 * the two moves that would turn the tree into a loop. The binary refuses them as well — this just
 * means the interface does not offer a choice that is going to be rejected.
 */
export function MoveFolderDialog({
  folder,
  folders,
  onDone,
  onCancel,
}: {
  folder: FolderNode
  folders: FolderNode[]
  onDone: () => void
  onCancel: () => void
}) {
  const [parentId, setParentId] = useState(folder.parent_id ?? 'root')
  const [error, setError] = useState('')

  const descendants = new Set<string>([folder.id])
  let grew = true

  while (grew) {
    grew = false

    for (const candidate of folders) {
      if (candidate.parent_id && descendants.has(candidate.parent_id) && !descendants.has(candidate.id)) {
        descendants.add(candidate.id)
        grew = true
      }
    }
  }

  return (
    <AlertDialog open onOpenChange={(open) => !open && onCancel()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Mover {folder.name}</AlertDialogTitle>
          <AlertDialogDescription>
            Mover uma pasta não mexe no que está dentro dela: a chave da pasta continua a mesma.
          </AlertDialogDescription>
        </AlertDialogHeader>

        <div className="space-y-2">
          <Label>Para dentro de</Label>
          <Select value={parentId} onValueChange={setParentId}>
            <SelectTrigger className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="root">Raiz do cofre</SelectItem>
              {folders
                .filter((candidate) => !descendants.has(candidate.id))
                .map((candidate) => (
                  <SelectItem key={candidate.id} value={candidate.id}>
                    {'— '.repeat(candidate.depth)}
                    {candidate.name}
                  </SelectItem>
                ))}
            </SelectContent>
          </Select>

          {error && <p className="text-sm text-destructive">{error}</p>}
        </div>

        <AlertDialogFooter>
          <AlertDialogCancel>Cancelar</AlertDialogCancel>
          <Button
            onClick={() =>
              void api
                .moveFolder(folder.id, parentId === 'root' ? null : parentId)
                .then(onDone)
                .catch((cause) => setError(describe(cause)))
            }
          >
            Mover
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}

/**
 * Deleting a folder.
 *
 * A folder is a place, not a container of secrets: deleting one leaves every record it held exactly
 * where it was, at the vault's root. Saying so is the whole job of this dialog — somebody about to
 * delete a folder with forty logins in it needs to know they are not deleting forty logins.
 */
export function DeleteFolderDialog({
  folder,
  records,
  onDone,
  onCancel,
}: {
  folder: FolderNode
  records: number
  onDone: () => void
  onCancel: () => void
}) {
  const [error, setError] = useState('')

  return (
    <AlertDialog open onOpenChange={(open) => !open && onCancel()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Excluir a pasta {folder.name}?</AlertDialogTitle>
          <AlertDialogDescription>
            {records > 0
              ? `Os ${records} ${records === 1 ? 'registro que está' : 'registros que estão'} nela não ${
                  records === 1 ? 'é apagado' : 'são apagados'
                }: ${records === 1 ? 'volta' : 'voltam'} para a raiz do cofre.`
              : 'A pasta está vazia.'}
          </AlertDialogDescription>
        </AlertDialogHeader>

        {error && <p className="text-sm text-destructive">{error}</p>}

        <AlertDialogFooter>
          <AlertDialogCancel>Cancelar</AlertDialogCancel>
          <Button
            variant="destructive"
            onClick={() =>
              void api
                .deleteFolder(folder.id)
                .then(onDone)
                .catch((cause) => setError(describe(cause)))
            }
          >
            Excluir a pasta
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}

function describe(cause: unknown): string {
  return cause instanceof ApiError ? cause.message : 'Não foi possível falar com o keyhold.'
}
