import { useState } from 'react'

import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'

import { ColourPicker } from './ColourPicker'

/**
 * Asking for a name, in the page rather than through `window.prompt`.
 *
 * `prompt` is not a styling problem — it is a correctness one. Browsers suppress it in situations
 * nobody can predict (a background tab, a page that already showed one, a setting somebody turned
 * on), and a suppressed prompt returns `null`, which looks exactly like "cancelled". An action that
 * silently does nothing is the worst failure an interface can have.
 */
interface NameDialogProps {
  title: string
  hint?: string
  initial?: string
  confirmLabel?: string
  /** Offer a colour alongside the name. Absent means this thing has no colour to pick. */
  withColour?: boolean
  onConfirm: (name: string, colour: string) => void
  onCancel: () => void
}

export function NameDialog({
  title,
  hint,
  initial = '',
  confirmLabel = 'Criar',
  withColour = false,
  onConfirm,
  onCancel,
}: NameDialogProps) {
  const [name, setName] = useState(initial)
  const [colour, setColour] = useState('default')

  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="sm:max-w-md">
        <form
          onSubmit={(event) => {
            event.preventDefault()

            if (name.trim()) {
              onConfirm(name.trim(), colour)
            }
          }}
        >
          <DialogHeader>
            <DialogTitle>{title}</DialogTitle>
            {hint && <DialogDescription>{hint}</DialogDescription>}
          </DialogHeader>

          <div className="space-y-4 py-4">
            <Input value={name} autoFocus onChange={(event) => setName(event.target.value)} />
            {withColour && <ColourPicker value={colour} onChange={setColour} />}
          </div>

          <DialogFooter>
            <Button type="button" variant="ghost" onClick={onCancel}>
              Cancelar
            </Button>
            <Button type="submit" disabled={!name.trim()}>
              {confirmLabel}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
