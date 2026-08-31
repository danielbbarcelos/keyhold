import { useState } from 'react'
import { RotateCcw, Trash2 } from 'lucide-react'

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

import { api, type RecordPreview } from '../api'
import { kindIcon } from './kinds'

/**
 * The trash.
 *
 * Deleting cannot mean destroying on the first click: nothing here can bring a secret back — not a
 * backup, not the master password, not us. So a deleted record waits, and the only two ways out of
 * this list are restoring it or a confirmation that says plainly what is about to happen.
 */
interface TrashPanelProps {
  records: RecordPreview[]
  retentionDays: number
  onChanged: () => void
  vaultId: string
}

export function TrashPanel({ records, retentionDays, onChanged, vaultId }: TrashPanelProps) {
  const [confirming, setConfirming] = useState<{ what: 'one'; id: string; title: string } | { what: 'all' } | null>(
    null,
  )

  return (
    <div className="space-y-3 p-6">
      <div className="flex flex-wrap items-center gap-3">
        <p className="flex-1 text-sm text-muted-foreground">
          {retentionDays > 0
            ? `O que é excluído fica aqui por ${retentionDays} dias e some no primeiro destranque depois disso.`
            : 'A limpeza automática está desligada: o que é excluído fica aqui até você mandar apagar.'}
        </p>

        {records.length > 0 && (
          <Button variant="destructive" size="sm" onClick={() => setConfirming({ what: 'all' })}>
            <Trash2 />
            Esvaziar agora
          </Button>
        )}
      </div>

      {records.length === 0 ? (
        <p className="py-8 text-center text-sm text-muted-foreground">A lixeira está vazia.</p>
      ) : (
        <ul className="space-y-1">
          {records.map((record) => {
            const Icon = kindIcon(record.kind)

            return (
              <li
                key={record.id}
                className="flex items-center gap-3 rounded-lg border px-3 py-2 text-sm"
              >
                <Icon className="size-4 shrink-0 text-muted-foreground" />
                <span className="min-w-0 flex-1 truncate">{record.title}</span>

                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => void api.archiveRecord(record.id, false).then(onChanged)}
                >
                  <RotateCcw />
                  Restaurar
                </Button>

                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label="Apagar definitivamente"
                  onClick={() =>
                    setConfirming({ what: 'one', id: record.id, title: record.title })
                  }
                >
                  <Trash2 />
                </Button>
              </li>
            )
          })}
        </ul>
      )}

      <AlertDialog open={confirming !== null} onOpenChange={(open) => !open && setConfirming(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {confirming?.what === 'all'
                ? 'Apagar tudo o que está na lixeira?'
                : 'Apagar definitivamente?'}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {confirming?.what === 'one' && <strong>{confirming.title}</strong>}
              {confirming?.what === 'one' ? ' — i' : 'I'}sto não tem volta. O conteúdo cifrado e
              todas as versões antigas deixam de existir, e nem a senha-mestra traz de volta.
            </AlertDialogDescription>
          </AlertDialogHeader>

          <AlertDialogFooter>
            <AlertDialogCancel>Cancelar</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                const action =
                  confirming?.what === 'all'
                    ? api.emptyTrash(vaultId)
                    : api.destroyRecord((confirming as { id: string }).id)

                void action.then(() => {
                  setConfirming(null)
                  onChanged()
                })
              }}
            >
              Apagar
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
