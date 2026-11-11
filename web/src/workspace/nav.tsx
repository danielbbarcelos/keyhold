import { SquareArrowOutUpRight } from 'lucide-react'

import { tabAddress } from '../session'
import { RowMenu, type MenuEntry } from './menu'

/**
 * Where a tab can be: the same places the rail and the tree lead to, by id only.
 *
 * Ids rather than names on purpose: the fragment lands in the browser's history, and an opaque id
 * says nothing about what the vault holds. Nothing here is a route table: a tab that opens on a
 * place it cannot find (a vault since deleted, a record in the trash) simply shows Início.
 */
export type Destination =
  | { place: 'home' | 'trail' | 'instances' }
  | { place: 'settings'; section?: string }
  | { place: 'vault'; vault: string; record?: string; folder?: string; trash?: boolean }

function encode(to: Destination): URLSearchParams {
  const params = new URLSearchParams({ at: to.place })

  if (to.place === 'settings' && to.section) params.set('s', to.section)
  if (to.place === 'vault') {
    params.set('v', to.vault)
    if (to.record) params.set('r', to.record)
    if (to.folder) params.set('f', to.folder)
    if (to.trash) params.set('t', '1')
  }

  return params
}

/** Where this tab was asked to open, read once from its own address. */
export function destinationFromLocation(): Destination {
  const params = new URLSearchParams(window.location.hash.replace(/^#/, ''))
  const at = params.get('at')

  if (at === 'trail' || at === 'instances') return { place: at }
  if (at === 'settings') return { place: 'settings', section: params.get('s') ?? undefined }
  if (at === 'vault' && params.get('v')) {
    return {
      place: 'vault',
      vault: params.get('v')!,
      record: params.get('r') ?? undefined,
      folder: params.get('f') ?? undefined,
      trash: params.get('t') === '1',
    }
  }

  return { place: 'home' }
}

/** Keep this tab's address on where it is, so a reload comes back to the same place. */
export function rememberDestination(to: Destination): void {
  window.history.replaceState(null, '', tabAddress(encode(to)))
}

export function openInNewTab(to: Destination): void {
  window.open(tabAddress(encode(to)), '_blank', 'noopener')
}

export function newTabEntry(to: Destination): MenuEntry {
  return { kind: 'item', label: 'Abrir em nova guia', icon: SquareArrowOutUpRight, onSelect: () => openInNewTab(to) }
}

/**
 * Click handlers that behave like a link: a plain click goes there, Ctrl or ⌘ click and the
 * middle button open a new tab instead.
 */
export function linkProps(to: Destination, go: () => void) {
  return {
    onClick: (event: React.MouseEvent) => {
      if (event.ctrlKey || event.metaKey) {
        event.preventDefault()
        event.stopPropagation()
        openInNewTab(to)
        return
      }
      go()
    },
    onAuxClick: (event: React.MouseEvent) => {
      if (event.button === 1) {
        event.preventDefault()
        event.stopPropagation()
        openInNewTab(to)
      }
    },
    // Keeps the middle button from starting the browser's autoscroll on its way to auxclick.
    onMouseDown: (event: React.MouseEvent) => {
      if (event.button === 1) event.preventDefault()
    },
  }
}

/** A right-click menu with "Abrir em nova guia", for things that have no other actions. */
export function TabMenu({ to, children, more = [] }: { to: Destination; children: React.ReactNode; more?: MenuEntry[] }) {
  return <RowMenu entries={[newTabEntry(to), ...more]}>{children}</RowMenu>
}
