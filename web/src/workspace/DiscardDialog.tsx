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

/**
 * "Descartar as alterações?" before leaving something half written.
 *
 * A record being edited may hold a password that was just generated and exists nowhere else; a
 * click on the tree or the rail must not be the thing that throws it away.
 */
export function DiscardDialog({
  open,
  what = 'as alterações',
  onKeep,
  onDiscard,
}: {
  open: boolean
  what?: string
  onKeep: () => void
  onDiscard: () => void
}) {
  return (
    <AlertDialog open={open} onOpenChange={(isOpen) => !isOpen && onKeep()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Descartar {what}</AlertDialogTitle>
          <AlertDialogDescription>
            O que não foi salvo será perdido, inclusive uma senha recém-gerada. Esta ação não pode ser desfeita.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Continuar editando</AlertDialogCancel>
          <AlertDialogAction className="bg-destructive text-white hover:bg-destructive/90" onClick={onDiscard}>
            Descartar
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
