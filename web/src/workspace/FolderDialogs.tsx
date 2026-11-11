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
import { Checkbox } from '@/components/ui/checkbox'
import { awsPathOf, AwsNameField, nameProblem, suggestName, titleProblem } from './awsName'
import { ColourPicker } from './ColourPicker'
import { toast } from './toast'

/**
 * Renaming, moving and deleting a folder.
 *
 * These three existed in the binary from the day folders did, and were reachable only through the
 * API — which made the tree something you could build and never correct. A folder you cannot rename
 * is a typo you live with.
 */

export function RenameFolderDialog({
  folder,
  aws,
  onDone,
  onCancel,
}: {
  folder: FolderNode
  /** An AWS folder: its name lives on its marker, and its path moves only when asked. */
  aws?: { renameDefault: boolean }
  /** With the folder as it is now: an AWS folder whose path moved has a new id. */
  onDone: (folder?: FolderNode) => void
  onCancel: () => void
}) {
  const [name, setName] = useState(folder.name)
  const [colour, setColour] = useState(folder.colour ?? 'default')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [move, setMove] = useState(aws?.renameDefault ?? false)
  const [leaf, setLeaf] = useState<string | null>(null)

  const path = awsPathOf(folder.id) ?? ''
  const parent = path.split('/').slice(0, -1).join('/') || null
  const changed = name.trim() !== folder.name || leaf !== null
  const segment = aws && move && changed ? leaf ?? suggestName(name, 'pasta') : null
  const wrong = aws ? titleProblem(name, 'O nome da pasta') ?? (segment !== null ? nameProblem(parent, segment) : null) : null

  return (
    <AlertDialog open onOpenChange={(open) => !open && onCancel()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Renomear a pasta</AlertDialogTitle>
          <AlertDialogDescription>
            {aws
              ? 'O nome aparece no Keyhold como você escrever. Na AWS, a pasta é parte do nome de cada parâmetro dentro dela.'
              : 'O nome e a cor ficam cifrados como o resto do cofre.'}
          </AlertDialogDescription>
        </AlertDialogHeader>

        <div className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="folder-name">Nome</Label>
            <Input
              id="folder-name"
              value={name}
              autoFocus
              aria-invalid={aws ? titleProblem(name) !== null : undefined}
              onChange={(event) => setName(event.target.value)}
            />
          </div>

          {aws ? (
            changed && (
              <div className="space-y-2">
                <label className="flex items-center gap-2 text-[13px]">
                  <Checkbox checked={move} onCheckedChange={(on) => setMove(on === true)} />
                  Mudar o nome na AWS também
                </label>
                {move ? (
                  <>
                    <AwsNameField parent={parent} from={name} value={leaf} onChange={setLeaf} fallback="pasta" />
                    <p className="text-xs text-amber">
                      Cada parâmetro dentro de {path} vai para o caminho novo e perde o histórico. Quem lê pelo
                      caminho antigo para de achar. Se algum tiver anexos, só o nome no Keyhold muda.
                    </p>
                  </>
                ) : (
                  <p className="text-xs text-faint">
                    Na AWS continua <code className="font-mono">{path}</code>. Só o nome no Keyhold muda.
                  </p>
                )}
              </div>
            )
          ) : (
            <ColourPicker value={colour} onChange={setColour} />
          )}

          {(error || (aws && wrong)) && <p className="text-sm text-destructive">{error || wrong}</p>}
        </div>

        <AlertDialogFooter>
          <AlertDialogCancel>Cancelar</AlertDialogCancel>
          <Button
            disabled={!name.trim() || busy || wrong !== null}
            onClick={() => {
              setBusy(true)
              void api
                .renameFolder(folder.id, name.trim(), aws ? null : colour, segment ?? undefined)
                .then((answer) => {
                  if (answer.warning) toast('Renomeada, com um aviso', answer.warning, 'error')
                  onDone(answer.folder)
                })
                .catch((cause) => setError(describe(cause)))
                .finally(() => setBusy(false))
            }}
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
  return cause instanceof ApiError ? cause.message : 'Não foi possível falar com o Keyhold.'
}
