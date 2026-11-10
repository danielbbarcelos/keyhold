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
import { RecordTile } from './icons'
import { toastError } from './toast'
import { when } from './vault/paths'

/**
 * The trash.
 *
 * Deleting cannot mean destroying on the first click: nothing here can bring a secret back — not a
 * backup, not the master password, not us. So a deleted record waits, and the only two ways out of
 * this list are restoring it or a confirmation that says plainly what is about to happen.
 */
interface TrashPanelProps {
  title: string
  records: RecordPreview[]
  retentionDays: number
  onChanged: () => void
  /** `null` when the trash being shown is several vaults' at once, which nothing can empty in one
   * go — "esvaziar" would then mean a different set of records depending on which vault was open. */
  vaultId: string | null
}

export function TrashPanel({ title, records, retentionDays, onChanged, vaultId }: TrashPanelProps) {
  const [confirming, setConfirming] = useState<{ what: 'one'; id: string; title: string } | { what: 'all' } | null>(
    null,
  )

  const left = (archivedAt: string) => {
    if (retentionDays <= 0) return 'fica até você apagar'
    const gone = new Date(archivedAt.endsWith('Z') ? archivedAt : `${archivedAt}Z`)
    gone.setDate(gone.getDate() + retentionDays)
    const days = Math.max(0, Math.ceil((gone.getTime() - Date.now()) / 86_400_000))

    return days === 0 ? 'some no próximo destranque' : `some em ${days} ${days === 1 ? 'dia' : 'dias'}`
  }

  return (
    <div className="flex h-full min-h-0 flex-col overflow-auto rounded-[14px] border bg-card">
      <div className="flex items-center gap-3 border-b px-5 py-4">
        <div className="min-w-0 flex-1">
          <h3 className="text-[15px] font-semibold">{title}</h3>
          <p className="mt-0.5 text-[13px] text-muted-foreground">
            {retentionDays > 0
              ? `Registros excluídos esperam aqui por ${retentionDays} dias e somem no primeiro destranque depois disso.`
              : 'A limpeza automática está desligada: o que é excluído fica aqui até você mandar apagar.'}
          </p>
        </div>
        {records.length > 0 && vaultId && (
          <Button variant="outline" size="sm" onClick={() => setConfirming({ what: 'all' })}>
            <Trash2 />
            Esvaziar
          </Button>
        )}
      </div>

      {records.length === 0 ? (
        <p className="px-5 py-10 text-center text-sm text-faint">A lixeira está vazia.</p>
      ) : (
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b text-left text-xs text-faint">
              <th className="px-5 py-2.5 font-normal">Registro</th>
              <th className="px-3 py-2.5 font-normal">Excluído</th>
              <th className="px-3 py-2.5 font-normal">Prazo</th>
              <th className="px-5 py-2.5" />
            </tr>
          </thead>
          <tbody>
            {records.map((record) => (
              <tr key={record.id} className="border-b last:border-b-0">
                <td className="px-5 py-2.5">
                  <span className="flex items-center gap-3">
                    <RecordTile icon={record.icon} kind={record.kind} className="opacity-70" />
                    <span className="min-w-0">
                      <b className="block truncate font-medium">{record.title}</b>
                      {record.subtitle && <span className="block truncate text-xs text-faint">{record.subtitle}</span>}
                    </span>
                  </span>
                </td>
                <td className="px-3 py-2.5 text-muted-foreground">{when(record.archived_at ?? record.updated_at)}</td>
                <td className="px-3 py-2.5">
                  <span className="rounded-full bg-stone-soft px-2.5 py-0.5 text-xs text-muted-foreground">
                    {left(record.archived_at ?? record.updated_at)}
                  </span>
                </td>
                <td className="px-5 py-2.5">
                  <span className="flex items-center justify-end gap-1">
                    <Button variant="outline" size="sm" onClick={() =>
                        void api
                          .archiveRecord(record.id, false)
                          .then(onChanged)
                          .catch((cause) => toastError(cause, 'Não restaurei'))
                      }>
                      <RotateCcw />
                      Restaurar
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      aria-label="Apagar definitivamente"
                      onClick={() => setConfirming({ what: 'one', id: record.id, title: record.title })}
                    >
                      <Trash2 />
                    </Button>
                  </span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
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
              {confirming?.what === 'one' && <strong>{confirming.title}: </strong>}
              {confirming?.what === 'one' ? 'i' : 'I'}sto não tem volta. O conteúdo cifrado e
              todas as versões antigas deixam de existir, e nem a senha-mestra traz de volta.
            </AlertDialogDescription>
          </AlertDialogHeader>

          <AlertDialogFooter>
            <AlertDialogCancel>Cancelar</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                const action =
                  confirming?.what === 'all'
                    ? api.emptyTrash(vaultId as string)
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
