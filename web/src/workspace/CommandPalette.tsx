import {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command'

import type { RecordPreview, VaultSummary } from '../api'
import { RecordTile } from './icons'

/**
 * Ctrl+P: go to a record by name, in any vault.
 *
 * It searches what the lists already show (title, the line under it, the address), never field
 * values: matching on a secret would mean an interface that tells you a password by accepting a
 * guess at it.
 *
 * The `<Command>` inside `<CommandDialog>` is not redundant: this version of the dialog places its
 * children straight into the dialog's content, so the list and the input would otherwise render
 * outside the context that holds their shared state.
 */
interface CommandPaletteProps {
  records: RecordPreview[]
  vaults: VaultSummary[]
  open: boolean
  onOpenChange: (open: boolean) => void
  onPick: (record: RecordPreview) => void
}

export function CommandPalette({ records, vaults, open, onOpenChange, onPick }: CommandPaletteProps) {
  const vaultName = (id: string) => vaults.find((vault) => vault.id === id)?.name ?? ''

  return (
    <CommandDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Buscar em todos os registros"
      description="Busca pelo título, pela linha de identificação e pelo endereço."
    >
      <Command>
        <CommandInput placeholder="Buscar em todos os registros…" />
        <CommandList>
          <CommandEmpty>Nada encontrado.</CommandEmpty>

          {records.map((record) => (
            <CommandItem
              key={record.id}
              value={`${record.title} ${record.subtitle} ${record.url} ${vaultName(record.vault_id)} ${record.id}`}
              onSelect={() => onPick(record)}
            >
              <RecordTile icon={record.icon} kind={record.kind} size="xs" />
              <span className="min-w-0 flex-1 truncate">
                {record.title}
                {record.subtitle && <span className="ml-2 text-xs text-muted-foreground">{record.subtitle}</span>}
              </span>
              <span className="ml-auto text-xs text-faint">{vaultName(record.vault_id)}</span>
            </CommandItem>
          ))}
        </CommandList>
      </Command>
    </CommandDialog>
  )
}
