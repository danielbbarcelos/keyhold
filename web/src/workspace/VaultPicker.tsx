import { useState } from 'react'
import { ChevronsUpDown, Vault } from 'lucide-react'

import { Button } from '@/components/ui/button'
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from '@/components/ui/command'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { cn } from '@/lib/utils'

import type { VaultSummary } from '../api'
import { colourOf } from './colours'

/**
 * What "every vault at once" is called on the wire.
 *
 * A sentinel rather than `null`, because `null` already means "no vault chosen yet" — the state
 * this screen is in before the list arrives, and the one it falls back to when the chosen vault
 * stops existing. Those two are not the same thing and must not share a value.
 */
export const EVERY_VAULT = '*'

/**
 * Choosing which vault the screen is showing.
 *
 * A combobox rather than a select for two reasons. It types: a dropdown you have to read through
 * is fine at four vaults and useless at forty, and the whole point of vaults is that there end up
 * being many. And it holds an entry a select could not — "todos os cofres", which is not one of
 * the options but a way of looking at all of them, and so is separated from the list rather than
 * sitting at the top of it.
 */
export function VaultPicker({
  vaults,
  value,
  onPick,
}: {
  vaults: VaultSummary[]
  value: string | null
  onPick: (id: string) => void
}) {
  const [open, setOpen] = useState(false)

  const open_ones = vaults.filter((vault) => !vault.archived)
  const chosen = open_ones.find((vault) => vault.id === value) ?? null

  const label =
    value === EVERY_VAULT ? 'Todos os cofres' : (chosen?.name ?? 'Cofre')

  function pick(id: string) {
    setOpen(false)
    onPick(id)
  }

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          role="combobox"
          aria-expanded={open}
          aria-label="Cofre"
          className="h-8 min-w-0 flex-1 justify-between px-2 font-normal"
        >
          <span className="flex min-w-0 items-center gap-2">
            {value === EVERY_VAULT ? (
              <Vault className="size-3.5 shrink-0 text-muted-foreground" />
            ) : (
              <span
                className={cn('size-2.5 shrink-0 rounded-full', colourOf(chosen?.colour).dot)}
                aria-hidden="true"
              />
            )}
            <span className="truncate">{label}</span>
          </span>
          <ChevronsUpDown className="size-3.5 shrink-0 opacity-50" />
        </Button>
      </PopoverTrigger>

      <PopoverContent className="w-(--radix-popover-trigger-width) min-w-56 p-0">
        <Command>
          <CommandInput placeholder="Buscar cofre…" />
          <CommandList>
            <CommandEmpty>Nenhum cofre com esse nome.</CommandEmpty>

            <CommandGroup>
              <CommandItem
                value="todos os cofres"
                data-checked={value === EVERY_VAULT || undefined}
                onSelect={() => pick(EVERY_VAULT)}
              >
                <Vault className="text-muted-foreground" />
                Todos os cofres
              </CommandItem>
            </CommandGroup>

            <CommandSeparator />

            <CommandGroup>
              {open_ones.map((vault) => (
                <CommandItem
                  key={vault.id}
                  // The id is the value so two vaults may share a name; the name is what is
                  // searched.
                  value={vault.id}
                  keywords={[vault.name]}
                  data-checked={vault.id === value || undefined}
                  onSelect={() => pick(vault.id)}
                >
                  <span
                    className={cn('size-2.5 shrink-0 rounded-full', colourOf(vault.colour).dot)}
                    aria-hidden="true"
                  />
                  <span className="truncate">{vault.name}</span>
                </CommandItem>
              ))}
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}
