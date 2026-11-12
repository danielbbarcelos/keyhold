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

import { api, ApiError, type Payload } from '../api'
import { FieldRow } from './record/FieldRow'

/**
 * Reading an old version before deciding to restore it.
 *
 * Restoring used to be the only thing the history offered, which made it a choice between a date
 * and a guess. Seeing first is the point: the reason to open a version from three months ago is
 * usually to check whether it holds the password you are actually looking for.
 *
 * The values arrive masked, like everywhere else — an old password is still a password.
 */
interface VersionDialogProps {
  recordId: string
  version: number
  onRestore: () => void
  onCancel: () => void
}

export function VersionDialog({ recordId, version, onRestore, onCancel }: VersionDialogProps) {
  const [payload, setPayload] = useState<Payload | null>(null)
  const [error, setError] = useState('')

  useEffect(() => {
    void api
      .readVersion(recordId, version)
      .then((answer) => setPayload(answer.payload))
      .catch((cause) =>
        setError(cause instanceof ApiError ? cause.message : 'Não foi possível abrir esta versão.'),
      )
  }, [recordId, version])

  return (
    <AlertDialog open onOpenChange={(open) => !open && onCancel()}>
      <AlertDialogContent className="sm:max-w-lg">
        <AlertDialogHeader>
          <AlertDialogTitle>Versão {version}</AlertDialogTitle>
          <AlertDialogDescription>
            Ao restaurar, o conteúdo atual é mantido como mais uma versão no histórico, e esta passa a
            ser a atual.
          </AlertDialogDescription>
        </AlertDialogHeader>

        {error && <p className="text-sm text-destructive">{error}</p>}

        {payload && (
          <div className="overflow-hidden rounded-xl border bg-raised">
            <div className="border-b px-5 py-3 text-sm font-semibold">{payload.title}</div>
            {payload.fields.map((field, index) => (
              <FieldRow key={index} field={field} index={index} recordId={recordId} live={false} />
            ))}
            {payload.fields.length === 0 && (
              <p className="px-5 py-3 text-sm text-faint">Esta versão não possui campos.</p>
            )}
          </div>
        )}

        <AlertDialogFooter>
          <AlertDialogCancel>Fechar</AlertDialogCancel>
          <Button disabled={!payload} onClick={onRestore}>
            Restaurar esta versão
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
