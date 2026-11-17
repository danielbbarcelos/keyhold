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
import { t } from '@/i18n'

import { api, ApiError, type FolderNode, type VaultSummary } from '../api'
import { FolderCombobox } from './FolderCombobox'

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
      setError(cause instanceof ApiError ? cause.message : t('Não foi possível mover. Tente novamente.'))
    } finally {
      setBusy(false)
    }
  }

  return (
    <AlertDialog open onOpenChange={(open) => !open && onCancel()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t('Mover {what} para outro cofre', { what })}</AlertDialogTitle>
          <AlertDialogDescription>
            {t('O conteúdo é decifrado e cifrado novamente com a chave do cofre de destino, junto com o histórico e os anexos. Nada sai do arquivo do Keyhold. Para um cofre AWS, use “Copiar para outro local”.')}
          </AlertDialogDescription>
        </AlertDialogHeader>

        {elsewhere.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {t('Nenhum outro cofre disponível. Crie um em Ajustes.')}
          </p>
        ) : (
          <div className="space-y-4">
            <div className="space-y-2">
              <Label>{t('Cofre')}</Label>
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
              <Label>{t('Pasta')}</Label>
              <FolderCombobox
                folders={folders}
                value={folderId === 'root' ? null : folderId}
                onChange={(id) => setFolderId(id ?? 'root')}
              />
            </div>

            {/*
              * Agent access is decided per vault, so this is the one side effect of the move that
              * is not about where a row appears — and the only one worth interrupting for.
              */}
            {target?.agent_access && (
              <p className="text-sm text-amber-600 dark:text-amber-500">
                {t('Um agente local pode visualizar o cofre {name}.', { name: target.name })}
              </p>
            )}

            {error && <p className="text-sm text-destructive">{error}</p>}
          </div>
        )}

        <AlertDialogFooter>
          <AlertDialogCancel>{t('Cancelar')}</AlertDialogCancel>
          <Button disabled={busy || !vaultId} onClick={() => void move()}>
            {t('Mover')}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
