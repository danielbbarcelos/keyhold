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
import { t } from '@/i18n'

/**
 * "Descartar as alterações?" before leaving something half written.
 *
 * A record being edited may hold a password that was just generated and exists nowhere else; a
 * click on the tree or the rail must not be the thing that throws it away.
 */
export function DiscardDialog({
  open,
  what,
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
          <AlertDialogTitle>{what ? t('Descartar {what}', { what }) : t('Descartar as alterações')}</AlertDialogTitle>
          <AlertDialogDescription>
            {t('O que não foi salvo será perdido, inclusive uma senha recém-gerada. Esta ação não pode ser desfeita.')}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{t('Continuar editando')}</AlertDialogCancel>
          <AlertDialogAction className="bg-destructive text-white hover:bg-destructive/90" onClick={onDiscard}>
            {t('Descartar')}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
