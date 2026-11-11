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

import { AwsNameField, nameProblem, suggestName, titleProblem } from './awsName'
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
  /** An AWS folder: its parent's path, for "Na AWS: …". `undefined` anywhere else. */
  awsParent?: string | null
  onConfirm: (name: string, colour: string, segment?: string) => void
  onCancel: () => void
}

export function NameDialog({
  title,
  hint,
  initial = '',
  confirmLabel = 'Criar',
  withColour = false,
  awsParent,
  onConfirm,
  onCancel,
}: NameDialogProps) {
  const [name, setName] = useState(initial)
  const [colour, setColour] = useState('default')
  const aws = awsParent !== undefined
  const [typed, setTyped] = useState<string | null>(null)
  const segment = typed ?? suggestName(name, 'pasta')
  const wrong = aws ? titleProblem(name, 'O nome da pasta') ?? (name.trim() ? nameProblem(awsParent ?? null, segment) : null) : null

  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="sm:max-w-md">
        <form
          onSubmit={(event) => {
            event.preventDefault()

            if (name.trim() && !wrong) {
              onConfirm(name.trim(), colour, aws ? segment : undefined)
            }
          }}
        >
          <DialogHeader>
            <DialogTitle>{title}</DialogTitle>
            {hint && <DialogDescription>{hint}</DialogDescription>}
          </DialogHeader>

          <div className="space-y-4 py-4">
            <Input
              value={name}
              autoFocus
              aria-invalid={aws ? titleProblem(name) !== null : undefined}
              onChange={(event) => setName(event.target.value)}
            />
            {aws && titleProblem(name) && <p className="-mt-2 text-xs text-destructive">{titleProblem(name, 'O nome da pasta')}</p>}
            {aws && name.trim() && !titleProblem(name) && (
              <div className="-mt-2">
                <AwsNameField parent={awsParent ?? null} from={name} value={typed} onChange={setTyped} fallback="pasta" />
              </div>
            )}
            {withColour && <ColourPicker value={colour} onChange={setColour} />}
          </div>

          <DialogFooter>
            <Button type="button" variant="ghost" onClick={onCancel}>
              Cancelar
            </Button>
            <Button type="submit" disabled={!name.trim() || wrong !== null}>
              {confirmLabel}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
