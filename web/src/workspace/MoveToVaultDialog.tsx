import { useEffect, useState } from 'react'

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
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'

import { api, ApiError, type FolderNode, type VaultSummary } from '../api'

/**
 * Moving records to another vault.
 *
 * This is the one move that is not filing. A record's key is wrapped under its vault's key, so
 * changing vault means the binary unwraps it, wraps it again on the other side, and re-seals the
 * payload and every past version — which is why it asks first and says what changes.
 *
 * The destination's folders are fetched when the vault is picked rather than held for every vault:
 * a folder list is a vault's shape, and there is no reason for this screen to hold the shape of a
 * vault nobody chose.
 */
export function MoveToVaultDialog({
  ids,
  what,
  vaults,
  fromVaultId,
  onDone,
  onCancel,
}: {
  ids: string[]
  what: string
  vaults: VaultSummary[]
  fromVaultId: string | null
  onDone: () => void
  onCancel: () => void
}) {
  // AWS vaults are not offered here: going between the local file and an AWS account is a copy
  // ("Copiar para…"), which asks about every name and offers to delete the original at the end.
  const elsewhere = vaults.filter((vault) => vault.id !== fromVaultId && !vault.archived && vault.kind !== 'aws')

  const [vaultId, setVaultId] = useState(elsewhere[0]?.id ?? '')
  const [folders, setFolders] = useState<FolderNode[]>([])
  const [folderId, setFolderId] = useState('root')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  useEffect(() => {
    if (!vaultId) {
      return
    }

    let current = true

    setFolderId('root')

    void api
      .folders(vaultId)
      .then((answer) => current && setFolders(answer.folders))
      .catch(() => current && setFolders([]))

    return () => {
      current = false
    }
  }, [vaultId])

  const target = elsewhere.find((vault) => vault.id === vaultId) ?? null

  async function move() {
    setBusy(true)

    try {
      // Sequential, like every other bulk action here: one SQLite file, and each of these is a
      // transaction that re-encrypts.
      for (const id of ids) {
        await api.moveRecord(id, folderId === 'root' ? null : folderId, vaultId)
      }

      onDone()
    } catch (cause) {
      setError(cause instanceof ApiError ? cause.message : 'Não foi possível mover. Tente novamente.')
    } finally {
      setBusy(false)
    }
  }

  return (
    <AlertDialog open onOpenChange={(open) => !open && onCancel()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Mover {what} para outro cofre</AlertDialogTitle>
          <AlertDialogDescription>
            O conteúdo é decifrado e cifrado novamente com a chave do cofre de destino, junto com o
            histórico e os anexos. Nada sai do arquivo do Keyhold. Para um cofre AWS, use “Copiar
            para outro local”.
          </AlertDialogDescription>
        </AlertDialogHeader>

        {elsewhere.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            Nenhum outro cofre disponível. Crie um em Ajustes.
          </p>
        ) : (
          <div className="space-y-4">
            <div className="space-y-2">
              <Label>Cofre</Label>
              <Select value={vaultId} onValueChange={setVaultId}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {elsewhere.map((vault) => (
                    <SelectItem key={vault.id} value={vault.id}>
                      {vault.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="space-y-2">
              <Label>Pasta</Label>
              <Select value={folderId} onValueChange={setFolderId}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="root">Raiz do cofre</SelectItem>
                  {folders.map((folder) => (
                    <SelectItem key={folder.id} value={folder.id}>
                      {'\u00a0\u00a0\u00a0'.repeat(folder.depth)}
                      {folder.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            {/*
              * Agent access is decided per vault, so this is the one side effect of the move that
              * is not about where a row appears — and the only one worth interrupting for.
              */}
            {target?.agent_access && (
              <p className="text-sm text-amber-600 dark:text-amber-500">
                Um agente local pode visualizar o cofre {target.name}.
              </p>
            )}

            {error && <p className="text-sm text-destructive">{error}</p>}
          </div>
        )}

        <AlertDialogFooter>
          <AlertDialogCancel>Cancelar</AlertDialogCancel>
          <Button disabled={busy || !vaultId} onClick={() => void move()}>
            Mover
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
