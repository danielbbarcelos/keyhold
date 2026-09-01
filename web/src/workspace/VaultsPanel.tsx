import { useEffect, useState } from 'react'
import { Archive, ArchiveRestore, Pencil, Trash2, Vault } from 'lucide-react'

import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { cn } from '@/lib/utils'

import { api, ApiError, type VaultSummary } from '../api'
import { ColourPicker } from './ColourPicker'
import { colourOf } from './colours'

/**
 * Renaming, putting away and deleting a vault.
 *
 * The three sit together because they are the same decision at three strengths, and the interface
 * should make that ladder visible: renaming changes nothing, archiving hides a vault and is undone
 * with one click, and deleting is the largest irreversible action in keyhold.
 *
 * The delete confirmation asks for the vault's name rather than an "are you sure". A yes/no box is
 * answered from muscle memory; a name has to be read off the screen and typed. It also states a
 * number — "isto apaga 47 registros" — because a count is something somebody can weigh, and "não
 * tem volta" is a sentence everybody has learned to click past.
 */
interface VaultsPanelProps {
  vaults: VaultSummary[]
  onChanged: () => void
}

export function VaultsPanel({ vaults, onChanged }: VaultsPanelProps) {
  const [renaming, setRenaming] = useState<VaultSummary | null>(null)
  const [deleting, setDeleting] = useState<VaultSummary | null>(null)

  return (
    <Card className="py-6">
      <CardHeader className="px-6">
        <CardTitle className="flex items-center gap-2">
          <Vault className="size-4" />
          Cofres
        </CardTitle>
        <CardDescription>
          Guardar um cofre o esconde da lista e é reversível. Excluir não é: leva junto tudo que
          está dentro, incluindo o histórico e a lixeira.
        </CardDescription>
      </CardHeader>

      <CardContent className="space-y-3 px-6">
        {vaults.map((vault) => (
          <div key={vault.id} className="flex items-center justify-between gap-3">
            <span className="flex min-w-0 items-center gap-2">
              <Vault className={cn('size-3.5 shrink-0', colourOf(vault.colour).text)} />
              <span className="truncate text-sm">{vault.name}</span>
              {vault.archived && <Badge variant="secondary">guardado</Badge>}
            </span>

            <span className="flex shrink-0 items-center gap-1">
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label="Renomear"
                onClick={() => setRenaming(vault)}
              >
                <Pencil />
              </Button>

              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={vault.archived ? 'Trazer de volta' : 'Guardar'}
                onClick={() => void api.archiveVault(vault.id, !vault.archived).then(onChanged)}
              >
                {vault.archived ? <ArchiveRestore /> : <Archive />}
              </Button>

              <Button
                variant="ghost"
                size="icon-sm"
                aria-label="Excluir o cofre"
                onClick={() => setDeleting(vault)}
              >
                <Trash2 />
              </Button>
            </span>
          </div>
        ))}
      </CardContent>

      {renaming && (
        <RenameDialog
          vault={renaming}
          onDone={() => {
            setRenaming(null)
            onChanged()
          }}
          onCancel={() => setRenaming(null)}
        />
      )}

      {deleting && (
        <DeleteDialog
          vault={deleting}
          onDone={() => {
            setDeleting(null)
            onChanged()
          }}
          onCancel={() => setDeleting(null)}
        />
      )}
    </Card>
  )
}

function RenameDialog({
  vault,
  onDone,
  onCancel,
}: {
  vault: VaultSummary
  onDone: () => void
  onCancel: () => void
}) {
  const [name, setName] = useState(vault.name)
  const [colour, setColour] = useState(vault.colour)
  const [error, setError] = useState('')

  return (
    <AlertDialog open onOpenChange={(open) => !open && onCancel()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Renomear o cofre</AlertDialogTitle>
          <AlertDialogDescription>
            O nome fica cifrado como o resto: quem ler o arquivo não vê nem isto.
          </AlertDialogDescription>
        </AlertDialogHeader>

        <div className="space-y-4">
          <Input value={name} autoFocus onChange={(event) => setName(event.target.value)} />
          <ColourPicker value={colour} onChange={setColour} />
          {error && <p className="text-sm text-destructive">{error}</p>}
        </div>

        <AlertDialogFooter>
          <AlertDialogCancel>Cancelar</AlertDialogCancel>
          <Button
            disabled={!name.trim()}
            onClick={() =>
              void api
                .updateVault(vault.id, {
                  name: name.trim(),
                  description: vault.description,
                  colour,
                })
                .then(onDone)
                .catch((cause) => setError(describe(cause)))
            }
          >
            Renomear
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}

function DeleteDialog({
  vault,
  onDone,
  onCancel,
}: {
  vault: VaultSummary
  onDone: () => void
  onCancel: () => void
}) {
  const [typed, setTyped] = useState('')
  const [error, setError] = useState('')
  const [counts, setCounts] = useState<{
    records: number
    archived_records: number
    folders: number
  } | null>(null)

  // Ask what is inside before offering to destroy it. The dialog opens with the question already
  // in flight, so the number is on screen by the time somebody has finished reading the title.
  useEffect(() => {
    void api
      .vaultContents(vault.id)
      .then((answer) => setCounts(answer.contents))
      .catch(() => setCounts(null))
  }, [vault.id])

  const total = counts ? counts.records + counts.archived_records : null

  return (
    <AlertDialog open onOpenChange={(open) => !open && onCancel()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Excluir o cofre {vault.name}?</AlertDialogTitle>
          <AlertDialogDescription>
            {total === null
              ? 'Conferindo o que há dentro…'
              : total === 0
                ? 'Este cofre está vazio.'
                : `Isto apaga ${total} ${total === 1 ? 'registro' : 'registros'}${
                    counts && counts.archived_records > 0
                      ? ` (${counts.archived_records} na lixeira)`
                      : ''
                  }${
                    counts && counts.folders > 0
                      ? ` e ${counts.folders} ${counts.folders === 1 ? 'pasta' : 'pastas'}`
                      : ''
                  }, com todo o histórico. Nem a senha-mestra traz de volta.`}
          </AlertDialogDescription>
        </AlertDialogHeader>

        <div className="space-y-2">
          <Label htmlFor="confirm-name">
            Digite <span className="text-foreground">{vault.name}</span> para confirmar
          </Label>
          <Input
            id="confirm-name"
            value={typed}
            autoFocus
            autoComplete="off"
            onChange={(event) => setTyped(event.target.value)}
          />
          {error && <p className="text-sm text-destructive">{error}</p>}
        </div>

        <AlertDialogFooter>
          <AlertDialogCancel>Cancelar</AlertDialogCancel>
          <Button
            variant="destructive"
            disabled={typed.trim() !== vault.name.trim()}
            onClick={() =>
              void api
                .deleteVault(vault.id, typed.trim())
                .then(onDone)
                .catch((cause) => setError(describe(cause)))
            }
          >
            Excluir para sempre
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}

function describe(cause: unknown): string {
  return cause instanceof ApiError ? cause.message : 'Não foi possível falar com o Keyhold.'
}
