import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { ScrollArea } from '@/components/ui/scroll-area'

import type { Payload } from '../api'
import { RecordEditor } from './RecordEditor'

/**
 * Writing a new record, in a dialog rather than in the middle of the workspace.
 *
 * It used to live where a record's tab goes, and that was wrong in a way that only shows up in use:
 * switching lens or clicking another record left the half-written form sitting there, unreachable
 * and unsaved, behind whatever came next. A dialog says what is actually true — this is one thing,
 * started deliberately, and finished or discarded before anything else happens.
 */
interface RecordDialogProps {
  open: boolean
  /** A copy of an existing record, rather than a blank one. */
  cloning?: boolean
  kind: string
  payload: Payload
  onKindChange: (kind: string) => void
  onChange: (payload: Payload) => void
  onSave: () => void
  onCancel: () => void
  busy?: boolean
  error?: string
}

export function RecordDialog({
  open,
  cloning,
  kind,
  payload,
  onKindChange,
  onChange,
  onSave,
  onCancel,
  busy,
  error,
}: RecordDialogProps) {
  return (
    <Dialog open={open} onOpenChange={(next) => !next && onCancel()}>
      <DialogContent className="max-h-[85vh] gap-0 overflow-hidden p-0 sm:max-w-2xl">
        <DialogHeader className="border-b px-6 py-4">
          <DialogTitle>{cloning ? 'Clonar registro' : 'Novo registro'}</DialogTitle>
          <DialogDescription>
            {cloning
              ? 'Uma cópia idêntica, para você ajustar antes de salvar. Ela nasce na mesma pasta do original — anexos não vêm junto.'
              : 'O conteúdo é selado antes de tocar o disco. Só o título e o usuário voltam na lista.'}
          </DialogDescription>
        </DialogHeader>

        <ScrollArea className="max-h-[calc(85vh-6rem)]">
          <RecordEditor
            kind={kind}
            payload={payload}
            onKindChange={onKindChange}
            onChange={onChange}
            onSave={onSave}
            onCancel={onCancel}
            busy={busy}
            error={error}
          />
        </ScrollArea>
      </DialogContent>
    </Dialog>
  )
}
