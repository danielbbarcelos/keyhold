import { MoreHorizontal } from 'lucide-react'

import { Button } from '@/components/ui/button'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from '@/components/ui/context-menu'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { cn } from '@/lib/utils'

/**
 * One description of a row's actions, drawn two ways.
 *
 * A row needs both: the `⋯` button is the affordance somebody discovers, and the right-click is
 * the one they reach for afterwards without looking. Writing the list twice — once in the dropdown
 * family, once in the context family — is how the two quietly drift apart, and a menu that offers
 * different things depending on how it was opened is worse than either alone.
 *
 * So the actions are data, and these two renderers are the only place either component family
 * appears.
 */
export interface SubEntry {
  label: string
  /** A Tailwind background class, drawn as a dot before the label. */
  swatch?: string
  onSelect: () => void
}

export type MenuEntry =
  | { kind: 'item'; label: string; destructive?: boolean; onSelect: () => void }
  | { kind: 'separator' }
  | { kind: 'sub'; label: string; items: SubEntry[] }

function Swatch({ colour }: { colour?: string }) {
  if (!colour) {
    return null
  }

  return <span className={cn('size-2.5 shrink-0 rounded-full', colour)} aria-hidden="true" />
}

function asDropdown(entry: MenuEntry, at: number) {
  if (entry.kind === 'separator') {
    return <DropdownMenuSeparator key={at} />
  }

  if (entry.kind === 'sub') {
    return (
      <DropdownMenuSub key={at}>
        <DropdownMenuSubTrigger>{entry.label}</DropdownMenuSubTrigger>
        <DropdownMenuSubContent>
          {entry.items.map((item, index) => (
            <DropdownMenuItem key={index} onSelect={item.onSelect}>
              <Swatch colour={item.swatch} />
              {item.label}
            </DropdownMenuItem>
          ))}
        </DropdownMenuSubContent>
      </DropdownMenuSub>
    )
  }

  return (
    <DropdownMenuItem
      key={at}
      variant={entry.destructive ? 'destructive' : 'default'}
      onSelect={entry.onSelect}
    >
      {entry.label}
    </DropdownMenuItem>
  )
}

function asContext(entry: MenuEntry, at: number) {
  if (entry.kind === 'separator') {
    return <ContextMenuSeparator key={at} />
  }

  if (entry.kind === 'sub') {
    return (
      <ContextMenuSub key={at}>
        <ContextMenuSubTrigger>{entry.label}</ContextMenuSubTrigger>
        <ContextMenuSubContent>
          {entry.items.map((item, index) => (
            <ContextMenuItem key={index} onSelect={item.onSelect}>
              <Swatch colour={item.swatch} />
              {item.label}
            </ContextMenuItem>
          ))}
        </ContextMenuSubContent>
      </ContextMenuSub>
    )
  }

  return (
    <ContextMenuItem
      key={at}
      variant={entry.destructive ? 'destructive' : 'default'}
      onSelect={entry.onSelect}
    >
      {entry.label}
    </ContextMenuItem>
  )
}

/** The `⋯` button, for finding out that a row has actions at all. */
export function MenuButton({ entries, label }: { entries: MenuEntry[]; label: string }) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon-xs" aria-label={label}>
          <MoreHorizontal />
        </Button>
      </DropdownMenuTrigger>

      <DropdownMenuContent align="start" className="w-52">
        {entries.map(asDropdown)}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/**
 * The same actions on right-click.
 *
 * `asChild` on purpose: the row keeps being the row — its drag handlers, its classes, its layout —
 * and gains the gesture rather than an extra element wrapped around it.
 */
export function RowMenu({
  entries,
  children,
}: {
  entries: MenuEntry[]
  children: React.ReactNode
}) {
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>{children}</ContextMenuTrigger>
      <ContextMenuContent className="w-52">{entries.map(asContext)}</ContextMenuContent>
    </ContextMenu>
  )
}
