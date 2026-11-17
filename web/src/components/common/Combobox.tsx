import { useState, type ReactNode } from 'react'
import { Check, ChevronsUpDown } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Command, CommandEmpty, CommandInput, CommandItem, CommandList } from '@/components/ui/command'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { t } from '@/i18n'
import { cn } from '@/lib/utils'

export interface ComboboxOption {
  value: string
  /** What the trigger shows once picked, and the first line in the list. */
  label: string
  /** A second, quieter line: an account id, a role. Searched too. */
  detail?: string
  /** Drawn before the text, in the list and in the trigger. */
  leading?: ReactNode
  /** Drawn after the text in the list: a badge, a state. */
  trailing?: ReactNode
  disabled?: boolean
}

/**
 * One choice among many, picked by typing.
 *
 * A native select lists everything and makes somebody scroll; an AWS login reaches dozens of
 * accounts. Here the name, the id or the role is typed and the list narrows to it, and the same
 * control serves the short lists too, so every choice in Keyhold looks and works the same.
 */
export function Combobox({
  options,
  value,
  onChange,
  label,
  placeholder = t('Selecione…'),
  searchPlaceholder = t('Buscar'),
  empty = t('Nada encontrado.'),
  tall = false,
  className,
}: {
  options: ComboboxOption[]
  value: string
  onChange: (value: string) => void
  /** The control's accessible name. */
  label: string
  placeholder?: string
  searchPlaceholder?: string
  empty?: string
  /** Room for a second line in the trigger. */
  tall?: boolean
  className?: string
}) {
  const [open, setOpen] = useState(false)
  const current = options.find((option) => option.value === value)

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant="outline"
          role="combobox"
          aria-expanded={open}
          aria-label={label}
          className={cn('w-full justify-between gap-2 font-normal', tall && 'h-auto min-h-9 py-2', className)}
        >
          {current ? (
            <span className="flex min-w-0 flex-1 items-center gap-2.5 text-left">
              {current.leading}
              <span className="min-w-0 flex-1">
                <span className="block truncate">{current.label}</span>
                {tall && current.detail && <span className="block truncate text-xs text-faint">{current.detail}</span>}
              </span>
            </span>
          ) : (
            <span className="truncate text-muted-foreground">{placeholder}</span>
          )}
          <ChevronsUpDown className="size-4 shrink-0 opacity-50" />
        </Button>
      </PopoverTrigger>

      <PopoverContent className="w-(--radix-popover-trigger-width) min-w-[260px] p-0" align="start">
        <Command>
          <CommandInput placeholder={searchPlaceholder} />
          <CommandList>
            <CommandEmpty>{empty}</CommandEmpty>
            {options.map((option) => (
              // The value keeps two options with the same label apart; label and detail are searched.
              <CommandItem
                key={option.value}
                value={`${option.label} ${option.detail ?? ''} ${option.value}`}
                disabled={option.disabled}
                onSelect={() => {
                  onChange(option.value)
                  setOpen(false)
                }}
              >
                {option.leading}
                <span className="min-w-0 flex-1">
                  <span className="block truncate">{option.label}</span>
                  {option.detail && <span className="block truncate text-xs text-faint">{option.detail}</span>}
                </span>
                {option.trailing}
                {option.value === value && <Check className="size-4 shrink-0" />}
              </CommandItem>
            ))}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}
