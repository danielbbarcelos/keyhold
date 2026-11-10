import type { FolderNode } from '../../api'

/** The folders from the root down to `folderId`, inclusive. */
export function folderChain(folders: FolderNode[], folderId: string | null): FolderNode[] {
  const chain: FolderNode[] = []
  let at = folders.find((folder) => folder.id === folderId)

  while (at && chain.length < 32) {
    chain.unshift(at)
    at = folders.find((folder) => folder.id === at!.parent_id)
  }

  return chain
}

/** "Infra / Bancos", or an empty string for the vault's root. */
export function folderPath(folders: FolderNode[], folderId: string | null): string {
  return folderChain(folders, folderId)
    .map((folder) => folder.name)
    .join(' / ')
}

/** "hoje, 13:43", "ontem", "há 5 min", "12/10/2026". Dates from the binary are UTC. */
export function when(iso: string | null | undefined): string {
  if (!iso) {
    return ''
  }

  const at = new Date(iso.endsWith('Z') || iso.includes('+') ? iso : `${iso}Z`)
  const seconds = Math.round((Date.now() - at.getTime()) / 1000)
  const time = at.toLocaleTimeString('pt-BR', { hour: '2-digit', minute: '2-digit' })

  if (seconds < 60) return 'agora'
  if (seconds < 3600) return `há ${Math.floor(seconds / 60)} min`

  const today = new Date()
  const sameDay = (a: Date, b: Date) =>
    a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate()

  if (sameDay(at, today)) return `hoje, ${time}`

  const yesterday = new Date(today)
  yesterday.setDate(today.getDate() - 1)

  if (sameDay(at, yesterday)) return `ontem, ${time}`

  return at.toLocaleDateString('pt-BR')
}
