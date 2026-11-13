import { useState } from 'react'
import { Check, ChevronsUpDown, Folder, FolderRoot } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Command, CommandEmpty, CommandInput, CommandItem, CommandList } from '@/components/ui/command'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { cn } from '@/lib/utils'

import type { FolderNode } from '../api'

/**
 * Where something goes: the vault's root or one of its folders, picked by typing.
 *
 * A vault grows to dozens of folders, and a plain list of them is a list somebody scrolls. Here the
 * name is typed, and what is searched is the whole path, so "prod" finds every folder under
 * "Produção" as well as the ones called that.
 */
export function FolderCombobox({
  folders,
  value,
  onChange,
  rootLabel = 'Raiz do cofre',
  label = 'Pasta',
  className,
}: {
  folders: FolderNode[]
  /** `null` is the root. */
  value: string | null
  onChange: (folderId: string | null) => void
  rootLabel?: string
  label?: string
  className?: string
}) {
  const [open, setOpen] = useState(false)
  const paths = pathsOf(folders)
  const current = value === null ? rootLabel : (paths.get(value) ?? rootLabel)

  const pick = (id: string | null) => {
    onChange(id)
    setOpen(false)
  }

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant="outline"
          role="combobox"
          aria-expanded={open}
          aria-label={label}
          className={cn('w-full justify-between font-normal', className)}
        >
          <span className="truncate">{current}</span>
          <ChevronsUpDown className="size-4 shrink-0 opacity-50" />
        </Button>
      </PopoverTrigger>

      <PopoverContent className="w-(--radix-popover-trigger-width) min-w-[260px] p-0" align="start">
        <Command>
          <CommandInput placeholder="Buscar pasta" />
          <CommandList>
            <CommandEmpty>Nenhuma pasta com esse nome.</CommandEmpty>
            <CommandItem value={rootLabel} onSelect={() => pick(null)}>
              <FolderRoot className="size-4 text-muted-foreground" />
              <span className="flex-1 truncate">{rootLabel}</span>
              {value === null && <Check className="size-4" />}
            </CommandItem>
            {folders.map((folder) => (
              // The id keeps two folders with the same path text apart; the path is what is searched.
              <CommandItem
                key={folder.id}
                value={`${paths.get(folder.id)} ${folder.id}`}
                keywords={[folder.name]}
                onSelect={() => pick(folder.id)}
              >
                <span style={{ width: folder.depth * 14 }} className="shrink-0" />
                <Folder className="size-4 shrink-0 text-muted-foreground" />
                <span className="flex-1 truncate">{folder.name}</span>
                {value === folder.id && <Check className="size-4" />}
              </CommandItem>
            ))}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}

/** Every folder's whole path ("Clientes / Acme / Produção"), by id. */
function pathsOf(folders: FolderNode[]): Map<string, string> {
  const byId = new Map(folders.map((folder) => [folder.id, folder]))
  const paths = new Map<string, string>()

  for (const folder of folders) {
    const parts: string[] = []
    let at: FolderNode | undefined = folder
    let guard = 0

    while (at && guard++ < 64) {
      parts.unshift(at.name)
      at = at.parent_id ? byId.get(at.parent_id) : undefined
    }

    paths.set(folder.id, parts.join(' / '))
  }

  return paths
}
