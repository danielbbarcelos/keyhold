import { useState } from 'react'
import { Eye, EyeOff } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'

/**
 * A password field you can look at.
 *
 * Every place keyhold asks somebody to type or check a secret needs the same two things: dots by
 * default, and a way to see what was actually typed. Without the second one a generated password
 * cannot be copied off the screen, a typo in a master password is invisible until it fails, and the
 * only way to check a value is to save it and reveal it afterwards.
 *
 * This is a composition of the registry's `Input` and `Button` rather than a modified `Input`: the
 * components in `components/ui` stay exactly as they were generated, so a future `shadcn add` never
 * has to be reconciled with local edits.
 */
interface PasswordInputProps extends Omit<React.ComponentProps<typeof Input>, 'type'> {
  /** Start visible — used where the value was just generated and is meant to be copied. */
  initiallyVisible?: boolean
  /** One more control inside the field, before the eye (the generator, in the editor). */
  trailing?: React.ReactNode
}

export function PasswordInput({
  className,
  initiallyVisible = false,
  trailing,
  ...props
}: PasswordInputProps) {
  const [visible, setVisible] = useState(initiallyVisible)

  return (
    <div className="relative flex-1">
      <Input
        {...props}
        type={visible ? 'text' : 'password'}
        className={cn(trailing ? 'pr-[4.5rem]' : 'pr-10', className)}
      />

      {trailing && <span className="absolute top-1/2 right-9 -translate-y-1/2">{trailing}</span>}

      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        aria-label={visible ? 'Esconder' : 'Mostrar'}
        aria-pressed={visible}
        // Sits inside the field rather than beside it, so a row of fields keeps one column of
        // inputs — a button that widens the row would break the editor's grid.
        className="absolute top-1/2 right-1 -translate-y-1/2 text-muted-foreground"
        onClick={() => setVisible(!visible)}
      >
        {visible ? <EyeOff /> : <Eye />}
      </Button>
    </div>
  )
}
