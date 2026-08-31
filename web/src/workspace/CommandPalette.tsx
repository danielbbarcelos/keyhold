import {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command'

import type { RecordPreview } from '../api'
import { kindIcon } from './kinds'

/**
 * Ctrl+P: go to a record by name.
 *
 * It searches the titles the list already has — which, since the list opens each record's name, is
 * every record in the open vault. Field values are deliberately not searched: matching on a secret
 * would mean an interface that tells you a password by accepting a guess at it.
 *
 * The `<Command>` inside `<CommandDialog>` is not redundant: this version of the dialog places its
 * children straight into the dialog's content, so the list and the input would otherwise render
 * outside the context that holds their shared state — which fails at runtime rather than visibly.
 */
interface CommandPaletteProps {
  records: RecordPreview[]
  open: boolean
  onOpenChange: (open: boolean) => void
  onPick: (id: string) => void
}

export function CommandPalette({ records, open, onOpenChange, onPick }: CommandPaletteProps) {
  return (
    <CommandDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Ir para um registro"
      description="Busca pelo título e pelo usuário dos registros deste cofre."
    >
      <Command>
        <CommandInput placeholder="Ir para um registro…" />
        <CommandList>
          <CommandEmpty>Nada encontrado.</CommandEmpty>

          {records.map((record) => {
            const Icon = kindIcon(record.kind)

            return (
              <CommandItem
                key={record.id}
                value={`${record.title} ${record.username}`}
                onSelect={() => onPick(record.id)}
              >
                <Icon />
                <span>{record.title}</span>
                {record.username && (
                  <span className="ml-auto text-xs text-muted-foreground">{record.username}</span>
                )}
              </CommandItem>
            )
          })}
        </CommandList>
      </Command>
    </CommandDialog>
  )
}
