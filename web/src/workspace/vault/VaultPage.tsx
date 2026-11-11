import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  Copy,
  CopyPlus,
  FolderInput,
  FolderPlus,
  Lock,
  MoveRight,
  Palette,
  Pencil,
  Plus,
  Star,
  Trash2,
  Vault,
} from 'lucide-react'

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'

import {
  api,
  ApiError,
  type FolderNode,
  type OpenRecord,
  type Payload,
  type RecordPreview,
  type Status,
  type VaultSummary,
} from '../../api'
import { COLOURS } from '../colours'
import { emptyPayload } from '../fields'
import { DiscardDialog } from '../DiscardDialog'
import { DeleteFolderDialog, MoveFolderDialog, RenameFolderDialog } from '../FolderDialogs'
import type { MenuEntry } from '../menu'
import { MoveToVaultDialog } from '../MoveToVaultDialog'
import { CopyDialog, type CopySource } from '../CopyDialog'
import { NameDialog } from '../NameDialog'
import { newTabEntry } from '../nav'
import { RecordSkeleton, TreeSkeleton } from '../Skeletons'
import { Splitter } from '../Splitter'
import { RecordEdit, renameOf, type EditDraft } from '../record/RecordEdit'
import { awsPathOf, nameProblem, suggestName, titleProblem } from '../awsName'
import { RecordView } from '../record/RecordView'
import { Crumb, TopBar, VaultSwitcher } from '../shell'
import { toast, toastError } from '../toast'
import { TrashPanel } from '../TrashPanel'
import { FolderView } from './FolderView'
import { folderChain, folderPath } from './paths'
import { TreeCard, type Selection, type TreeFilter, type TreeMode } from './TreeCard'
import type { Attention, Concern } from './Attention'

/**
 * Inside a vault: the tree on the left, always, and on the right whatever was picked in it (a
 * record, its editor, a folder, the trash).
 */
export function VaultPage({
  status,
  vault,
  vaults,
  selection,
  onSelect,
  onVault,
  onNewVault,
  onManageVaults,
  onSearch,
  onSeen,
  onAccounts,
  onDirty,
}: {
  status: Status
  vault: VaultSummary
  vaults: VaultSummary[]
  selection: Selection
  onSelect: (selection: Selection) => void
  onVault: (id: string) => void
  onNewVault: () => void
  onManageVaults: () => void
  onSearch: () => void
  onSeen: (record: RecordPreview) => void
  /** Ajustes, at Contas AWS: where an expired AWS session is renewed. */
  onAccounts: () => void
  /** Tells the workspace a record is being edited with changes, so it asks before leaving. */
  onDirty: (dirty: boolean) => void
}) {
  const [folders, setFolders] = useState<FolderNode[]>([])
  const [records, setRecords] = useState<RecordPreview[]>([])
  const [trash, setTrash] = useState<RecordPreview[]>([])
  const [attention, setAttention] = useState<Attention>(new Map())
  // "Editar" from the attention popover: the record opens, then its editor.
  const [editWhenOpen, setEditWhenOpen] = useState<string | null>(null)
  const [mode, setMode] = useState<TreeMode>('tree')
  const [filter, setFilter] = useState<TreeFilter>('all')
  const [open, setOpen] = useState<OpenRecord | null>(null)
  const [editing, setEditing] = useState<EditDraft | null>(null)
  // A record being written that does not exist yet. Its draft is `editing`, on the same page an edit uses.
  const [creating, setCreating] = useState<{ cloning?: boolean } | null>(null)
  const [asking, setAsking] = useState<{ parentId: string | null } | null>(null)
  const [folderAction, setFolderAction] = useState<{ what: 'rename' | 'move' | 'delete'; folder: FolderNode } | null>(null)
  const [deleting, setDeleting] = useState<{ id: string; title: string } | null>(null)
  const [movingToVault, setMovingToVault] = useState<{ id: string; title: string } | null>(null)
  const [copying, setCopying] = useState<CopySource | null>(null)
  const [saving, setSaving] = useState(false)
  const [askingJson, setAskingJson] = useState(false)
  const [movingAws, setMovingAws] = useState<{ ids: string[]; folderId: string | null } | null>(null)
  const [unavailable, setUnavailable] = useState<{ message: string; code?: string } | null>(null)
  // False until the first read of this vault answers: until then the tree and the panel are skeletons
  // rather than an "empty vault" that is not empty.
  const [loaded, setLoaded] = useState(false)
  const aws = vault.kind === 'aws'
  const [origin, setOrigin] = useState('')
  const [leaving, setLeaving] = useState<(() => void) | null>(null)
  const dirty = editing !== null && JSON.stringify(editing) !== origin
  const editingRef = useRef(false)
  editingRef.current = editing !== null

  useEffect(() => onDirty(dirty), [dirty, onDirty])

  /** Run `go` now, or after "Descartar" when there are unsaved changes. */
  const guard = (go: () => void) => (dirty ? setLeaving(() => go) : go())
  const select = (next: Selection) =>
    guard(() => {
      setCreating(null)
      if (creating) setEditing(null)
      onSelect(next)
    })

  /** "Novo registro", "Registro novo aqui" and "Clonar": the editing page, for a record yet to be. */
  const startCreating = (payload: Payload, folderId: string | null, cloning = false) =>
    guard(() => {
      const draft: EditDraft = { payload, folderId, rotation: null, rename: false, leaf: null }
      setOrigin(JSON.stringify(draft))
      setEditing(draft)
      setCreating({ cloning })
    })

  const stopEditing = () => {
    setEditing(null)
    setCreating(null)
  }

  const startEditing = (draft: EditDraft) => {
    // In AWS the vault decides whether a new title starts by moving the name too.
    const ready = aws ? { ...draft, rename: vault.aws?.rename_with_title ?? false, leaf: null } : draft
    setOrigin(JSON.stringify(ready))
    setEditing(ready)
  }

  /** Why "Salvar" cannot go to AWS as it is: a title with "/", or a name AWS would refuse. */
  const editProblem = (() => {
    if (!aws || !editing) return null
    if (creating) {
      return (
        titleProblem(editing.payload.title) ??
        (editing.payload.title.trim()
          ? nameProblem(awsPathOf(editing.folderId), editing.leaf ?? suggestName(editing.payload.title))
          : null)
      )
    }
    if (!open) return null
    const leaf = renameOf(editing, open.payload.title)

    return titleProblem(editing.payload.title) ?? (leaf !== null ? nameProblem(awsPathOf(editing.folderId), leaf) : null)
  })()

  const refresh = useCallback(async () => {
    try {
      const [tree, list, bin] = await Promise.all([api.folders(vault.id), api.records(vault.id), api.trash(vault.id)])

      setFolders(tree.folders)
      setRecords(list.records.filter((record) => !record.archived))
      setTrash(bin.records)
      setUnavailable(null)
      setLoaded(true)

      // Atenção: what the file already knows (compromised, rotation passed) plus the health report,
      // which opens every record to judge weak and repeated passwords.
      const day = (iso: string) => iso.slice(0, 10).split('-').reverse().join('/')
      const flagged: Attention = new Map()
      for (const record of list.records) {
        const concern: Concern = { issues: [], reasons: [] }
        if (record.compromised_at !== null) {
          concern.issues.push('compromised')
          concern.reasons.push(`Marcado como comprometido em ${day(record.compromised_at)}.`)
        }
        if (record.rotation_due_at !== null && new Date(record.rotation_due_at) <= new Date()) {
          concern.issues.push('rotation')
          concern.reasons.push(`A data de troca, ${day(record.rotation_due_at)}, já passou.`)
        }
        if (concern.issues.length) flagged.set(record.id, concern)
      }
      setAttention(flagged)

      // The health report opens every record to judge the passwords; its words win where it has some.
      void api
        .health(vault.id)
        .then((report) => {
          const merged: Attention = new Map(flagged)
          for (const finding of report.health.findings) {
            merged.set(finding.record_id, { issues: finding.issues, reasons: finding.reasons })
          }
          setAttention(merged)
        })
        .catch(() => {})
    } catch (cause) {
      setLoaded(true)
      if (cause instanceof ApiError && cause.aws && editingRef.current) {
        // Never swap an open editor for an error page: the draft would have no way back.
        toastError(cause, 'O cofre na AWS não respondeu')
      } else if (cause instanceof ApiError && cause.aws) {
        setUnavailable({ message: cause.message, code: cause.aws })
      } else {
        toastError(cause, 'Não consegui ler o cofre')
      }
    }
  }, [vault.id])

  useEffect(() => {
    setOpen(null)
    setEditing(null)
    setFilter('all')
    setLoaded(false)
    void refresh()
  }, [refresh])

  // An AWS vault is shared with whoever else IAM lets in: read it again when the window comes
  // back, and every minute while it is in front of somebody.
  useEffect(() => {
    if (!aws) return

    const again = () => document.visibilityState === 'visible' && void refresh()
    const timer = window.setInterval(again, 60_000)
    window.addEventListener('focus', again)

    return () => {
      window.clearInterval(timer)
      window.removeEventListener('focus', again)
    }
  }, [aws, refresh])

  const openId = selection?.kind === 'record' ? selection.id : null

  const loadRecord = useCallback(async (id: string) => {
    try {
      const found = await api.openRecord(id)
      setOpen(found.record)
    } catch (cause) {
      toastError(cause, 'Esse registro não abriu')
      onSelect(null)
    }
  }, [onSelect])

  useEffect(() => {
    setEditing(null)

    if (openId) {
      void loadRecord(openId)
    } else {
      setOpen(null)
    }
  }, [openId, loadRecord])

  useEffect(() => {
    if (open && editWhenOpen === open.id && !editing) {
      setEditWhenOpen(null)
      startEditing({
        payload: structuredClone(open.payload),
        folderId: open.folder_id,
        rotation: open.rotation_due_at?.slice(0, 10) ?? null,
      })
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, editWhenOpen])

  useEffect(() => {
    if (open) {
      const preview = records.find((record) => record.id === open.id)
      if (preview) onSeen(preview)
    }
    // Only when a different record opens, not on every list refresh.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open?.id])

  const reloadOpen = async () => {
    if (open) {
      const fresh = await api.openRecord(open.id)
      setOpen(fresh.record)
    }
    await refresh()
  }

  /**
   * Save what changed, and only that: the payload when it changed (a new version), then the
   * folder, then the rotation date. In AWS a folder change renames the parameter, so the steps
   * after it use the new id. A failure after the payload is written says so rather than
   * "não salvei".
   */
  async function save(jsonAgreed = false) {
    if (!open || !editing) return

    const before = JSON.parse(origin) as EditDraft
    const payloadChanged = JSON.stringify(editing.payload) !== JSON.stringify(before.payload)

    // In AWS a lone password is stored as the plain value applications read. Anything more (an
    // alert, an icon, notes, another field) turns it into Keyhold's JSON, and those applications
    // stop reading it. That is said before it happens, not discovered after.
    if (aws && payloadChanged && !jsonAgreed && isPlainValue(before.payload) && !isPlainValue(editing.payload)) {
      setAskingJson(true)
      return
    }

    setSaving(true)
    let id = open.id
    let wrote = false

    try {
      if (payloadChanged) {
        const saved = await api.saveRecord(id, editing.payload, aws ? open.version : undefined)
        if (saved.warning) toast('Salvo, com um aviso', saved.warning, 'error')
        wrote = true
      }
      const newLeaf = aws ? renameOf(editing, before.payload.title) : null
      if (editing.folderId !== open.folder_id || newLeaf !== null) {
        const moved = await api.moveRecord(id, editing.folderId, undefined, newLeaf ?? undefined)
        if (moved.id) id = moved.id
        if (moved.warning) toast('Movido, com um aviso', moved.warning, 'error')
        wrote = true
      }
      if ((editing.rotation ?? null) !== (open.rotation_due_at?.slice(0, 10) ?? null)) {
        await api.setRotationDue(id, editing.rotation)
      }

      setEditing(null)
      if (id !== open.id) {
        await refresh()
        onSelect({ kind: 'record', id })
      } else {
        await reloadOpen()
      }
      toast('Registro salvo', editing.payload.title)
    } catch (cause) {
      if (cause instanceof ApiError && cause.aws === 'conflict') {
        // Somebody else saved in the meantime. Keep the draft, load their version underneath it,
        // and say plainly that saving again will replace what they wrote.
        try {
          const fresh = await api.openRecord(open.id)
          setOpen(fresh.record)
        } catch {
          /* the toast below still explains */
        }
        toastError(cause, 'Outra pessoa salvou antes. Salvar de novo substitui a versão dela.')
      } else if (wrote) {
        toastError(cause, 'Salvei em parte: o conteúdo foi gravado, o resto não')
        setEditing(null)
        if (id !== open.id) onSelect({ kind: 'record', id })
        else void reloadOpen()
      } else {
        toastError(cause, 'Não salvei')
      }
    } finally {
      setSaving(false)
    }
  }

  /** Create the record being written: empty fields are left out, as a new record starts lean. */
  async function create() {
    if (!editing) return
    setSaving(true)

    const payload = {
      ...editing.payload,
      fields: editing.payload.fields.filter(
        (field) => field.value.trim() !== '' || Object.values(field.extra ?? {}).some((part) => part.trim() !== ''),
      ),
    }

    try {
      const name = aws ? (editing.leaf ?? suggestName(payload.title)) : undefined
      const made = await api.createRecord(vault.id, editing.folderId, payload, name)
      if (editing.rotation) await api.setRotationDue(made.record.id, editing.rotation).catch(() => undefined)
      stopEditing()
      await refresh()
      onSelect({ kind: 'record', id: made.record.id })
      toast('Registro criado', payload.title)
    } catch (cause) {
      toastError(cause, 'Não criei o registro')
    } finally {
      setSaving(false)
    }
  }

  /** "×" on an alert: the record without it, as a new version. */
  async function removeAlert(index: number) {
    if (!open) return
    const payload = { ...open.payload, alerts: (open.payload.alerts ?? []).filter((_, at) => at !== index) }

    try {
      const saved = await api.saveRecord(open.id, payload, aws ? open.version : undefined)
      if (saved.warning) toast('Salvo, com um aviso', saved.warning, 'error')
      await reloadOpen()
      toast('Alerta retirado', open.payload.title)
    } catch (cause) {
      toastError(cause, 'Não tirei o alerta')
    }
  }

  async function clone(id: string) {
    try {
      const found = await api.openRecord(id)
      startCreating(
        { ...found.record.payload, title: `${found.record.payload.title} (cópia)` },
        found.record.folder_id,
        true,
      )
    } catch (cause) {
      toastError(cause)
    }
  }

  async function moveTo(id: string, folderId: string | null, confirmed = false) {
    // In Parameter Store a move is a new name: the old one, with its history, is deleted.
    if (aws && !confirmed) {
      setMovingAws({ ids: [id], folderId })

      return
    }

    try {
      const moved = await api.moveRecord(id, folderId)
      if (moved.warning) toast('Movido, com um aviso', moved.warning, 'error')
      await refresh()
      if (moved.id && moved.id !== id) {
        if (open?.id === id) onSelect({ kind: 'record', id: moved.id })
      } else if (open?.id === id) {
        await reloadOpen()
      }
      toast('Registro movido', folderPath(folders, folderId) || 'Raiz do cofre')
    } catch (cause) {
      toastError(cause, 'Não movi o registro')
    }
  }

  /** Whether `folderId` is `ancestor` or lies anywhere under it. */
  const isUnder = (folderId: string | null, ancestor: string): boolean => {
    let at = folderId
    while (at) {
      if (at === ancestor) return true
      at = folders.find((folder) => folder.id === at)?.parent_id ?? null
    }
    return false
  }

  /**
   * Several items dragged together onto a folder or the root. Records and folders already there are
   * left alone, and a folder never goes inside itself. In AWS one question covers every record (each
   * loses its history), and folders do not move: a folder there is part of every name under it.
   */
  async function moveMany(items: string[], folderId: string | null, confirmed = false) {
    const recordIds = items
      .filter((item) => item.startsWith('r:'))
      .map((item) => item.slice(2))
      .filter((id) => records.find((record) => record.id === id)?.folder_id !== folderId)
    let folderIds = items
      .filter((item) => item.startsWith('f:'))
      .map((item) => item.slice(2))
      .filter((id) => !isUnder(folderId, id) && folders.find((folder) => folder.id === id)?.parent_id !== folderId)

    if (aws && folderIds.length) {
      toast('Pastas não se movem na AWS', 'Use "Renomear" na pasta, com "Mudar o nome na AWS também".', 'error')
      folderIds = []
    }
    if (recordIds.length === 0 && folderIds.length === 0) return

    if (recordIds.length === 1 && folderIds.length === 0) {
      void moveTo(recordIds[0], folderId, confirmed)
      return
    }

    if (aws && !confirmed) {
      setMovingAws({ ids: recordIds, folderId })
      return
    }

    let failed = 0
    let moved = 0
    for (const id of recordIds) {
      try {
        const answer = await api.moveRecord(id, folderId)
        if (answer.warning) toast('Movido, com um aviso', answer.warning, 'error')
        moved++
      } catch {
        failed++
      }
    }
    for (const id of folderIds) {
      try {
        await api.moveFolder(id, folderId)
        moved++
      } catch {
        failed++
      }
    }

    // In AWS a moved record is a new name, so the one open may no longer exist under its id.
    if (aws && open && recordIds.includes(open.id)) onSelect(null)
    await refresh()

    const where = folderPath(folders, folderId) || 'Raiz do cofre'
    if (moved) toast(`${moved} ${moved === 1 ? 'item movido' : 'itens movidos'}`, where)
    if (failed) toast(`${failed} ${failed === 1 ? 'item não foi movido' : 'itens não foram movidos'}`, 'Tente de novo, um de cada vez, para ver o motivo.', 'error')
  }

  async function sendToTrash(id: string) {
    try {
      await api.archiveRecord(id, true)
      if (open?.id === id) onSelect(null)
      await refresh()
      toast(
        aws ? 'Apagado da AWS' : 'Foi para a lixeira',
        aws ? 'O parâmetro não existe mais na conta.' : 'Dá para restaurar de lá enquanto o prazo não vencer.',
      )
    } catch (cause) {
      toastError(cause)
    }
  }

  const folderTab = (folder: FolderNode): MenuEntry[] => [
    newTabEntry({ place: 'vault', vault: vault.id, folder: folder.id }),
    { kind: 'separator' },
  ]

  const folderEntries = (folder: FolderNode): MenuEntry[] => aws ? [...folderTab(folder), ...awsFolderEntries(folder)] : [
    ...folderTab(folder),
    {
      kind: 'item',
      label: 'Registro novo aqui',
      icon: Plus,
      onSelect: () => startCreating(emptyPayload(), folder.id),
    },
    { kind: 'item', label: 'Subpasta', icon: FolderPlus, onSelect: () => setAsking({ parentId: folder.id }) },
    { kind: 'item', label: 'Renomear', icon: Pencil, onSelect: () => setFolderAction({ what: 'rename', folder }) },
    {
      kind: 'sub',
      label: 'Cor',
      icon: Palette,
      items: COLOURS.map((colour) => ({
        label: colour.label,
        swatch: colour.dot,
        onSelect: () => void api.renameFolder(folder.id, folder.name, colour.value).then(refresh),
      })),
    },
    { kind: 'item', label: 'Mover', icon: MoveRight, onSelect: () => setFolderAction({ what: 'move', folder }) },
    { kind: 'item', label: 'Copiar para…', icon: Copy, onSelect: () => setCopying({ vaultId: vault.id, folderId: folder.id, what: `a pasta ${folder.name}` }) },
    { kind: 'separator' },
    { kind: 'item', label: 'Excluir', icon: Trash2, destructive: true, onSelect: () => setFolderAction({ what: 'delete', folder }) },
  ]

  /** In Parameter Store a folder is a path: it can hold new things and be removed when empty. */
  const awsFolderEntries = (folder: FolderNode): MenuEntry[] => [
    {
      kind: 'item',
      label: 'Registro novo aqui',
      icon: Plus,
      onSelect: () => startCreating(emptyPayload(), folder.id),
    },
    { kind: 'item', label: 'Subpasta', icon: FolderPlus, onSelect: () => setAsking({ parentId: folder.id }) },
    { kind: 'item', label: 'Renomear', icon: Pencil, onSelect: () => setFolderAction({ what: 'rename', folder }) },
    { kind: 'item', label: 'Copiar para…', icon: Copy, onSelect: () => setCopying({ vaultId: vault.id, folderId: folder.id, what: `a pasta ${folder.name}` }) },
    { kind: 'separator' },
    { kind: 'item', label: 'Excluir a pasta vazia', icon: Trash2, destructive: true, onSelect: () => setFolderAction({ what: 'delete', folder }) },
  ]

  const recordEntries = (record: RecordPreview): MenuEntry[] => [
    newTabEntry({ place: 'vault', vault: vault.id, record: record.id }),
    { kind: 'separator' },
    {
      kind: 'item',
      label: record.favourite ? 'Tirar dos favoritos' : 'Favoritar',
      icon: Star,
      onSelect: () => void api.favouriteRecord(record.id, !record.favourite).then(refresh),
    },
    { kind: 'item', label: 'Clonar', icon: CopyPlus, onSelect: () => void clone(record.id) },
    {
      kind: 'sub',
      label: 'Mover para outra pasta',
      icon: FolderInput,
      items: [
        { label: 'Raiz do cofre', onSelect: () => void moveTo(record.id, null) },
        ...folders.map((folder) => ({
          label: `${'  '.repeat(folder.depth)}${folder.name}`,
          onSelect: () => void moveTo(record.id, folder.id),
        })),
      ],
    },
    ...(aws
      ? []
      : [{ kind: 'item' as const, label: 'Mover para outro cofre', icon: Vault, onSelect: () => setMovingToVault({ id: record.id, title: record.title }) }]),
    {
      kind: 'item',
      label: 'Copiar para…',
      icon: Copy,
      onSelect: () => setCopying({ vaultId: vault.id, recordIds: [record.id], what: `“${record.title}”` }),
    },
    { kind: 'separator' },
    {
      kind: 'item',
      label: aws ? 'Apagar de vez' : 'Mandar para a lixeira',
      icon: Trash2,
      destructive: true,
      onSelect: () => setDeleting({ id: record.id, title: record.title }),
    },
  ]

  const currentFolder =
    selection?.kind === 'folder' ? folders.find((folder) => folder.id === selection.id) ?? null : null
  const chainFolderId = currentFolder?.id ?? (open && openId ? open.folder_id : null)
  const chain = useMemo(() => folderChain(folders, chainFolderId), [folders, chainFolderId])
  const newHere = currentFolder?.id ?? (open && openId ? open.folder_id : null)

  const bar = (
    <TopBar
      onSearch={editing ? undefined : onSearch}
      actions={
        editing ? (
          <>
            <span className="hidden items-center gap-1.5 text-xs text-faint lg:flex">
              <Lock className="size-3.5" />
              Selado antes de tocar o disco
            </span>
            <Button variant="outline" onClick={() => guard(stopEditing)}>
              Cancelar
            </Button>
            <Button
              disabled={saving || !editing.payload.title.trim() || editProblem !== null}
              title={editProblem ?? undefined}
              onClick={() => void (creating ? create() : save())}
            >
              {creating ? 'Criar registro' : 'Salvar alterações'}
            </Button>
          </>
        ) : (
          <>
            <Button variant="outline" onClick={() => setAsking({ parentId: currentFolder?.id ?? null })}>
              <FolderPlus />
              Nova pasta
            </Button>
            <Button onClick={() => startCreating(emptyPayload(), newHere)}>
              <Plus />
              Novo registro
            </Button>
          </>
        )
      }
    >
      <VaultSwitcher
        vaults={vaults}
        current={vault}
        onPick={(id) => guard(() => onVault(id))}
        onNew={onNewVault}
        onManage={onManageVaults}
        onCopy={() => setCopying({ vaultId: vault.id, what: `o cofre ${vault.name}` })}
      />
      {chain.map((folder, index) => (
        <Crumb
          key={folder.id}
          last={index === chain.length - 1 && !openId}
          to={{ place: 'vault', vault: vault.id, folder: folder.id }}
          onClick={() => select({ kind: 'folder', id: folder.id })}
        >
          {folder.name}
        </Crumb>
      ))}
      {open && openId && !creating && (
        <Crumb
          last={!editing}
          to={{ place: 'vault', vault: vault.id, record: open.id }}
          onClick={() => guard(() => setEditing(null))}
        >
          {open.payload.title}
        </Crumb>
      )}
      {editing && <Crumb last>{creating ? (creating.cloning ? 'Clonar' : 'Novo registro') : 'Editar'}</Crumb>}
      {selection?.kind === 'trash' && <Crumb last>Lixeira</Crumb>}
    </TopBar>
  )

  function main() {
    if (unavailable) {
      return (
        <div className="grid h-full place-items-center rounded-[14px] border border-dashed p-8 text-center">
          <div className="max-w-sm space-y-3">
            <b className="block text-[15px] font-semibold">
              {unavailable.code === 'session_expired' ? 'A sessão da AWS acabou' : 'O cofre na AWS não respondeu'}
            </b>
            <p className="text-sm text-muted-foreground">{unavailable.message}</p>
            <div className="flex justify-center gap-2">
              {unavailable.code === 'session_expired' && <Button onClick={onAccounts}>Renovar em Contas AWS</Button>}
              <Button variant="outline" onClick={() => void refresh()}>
                Tentar de novo
              </Button>
            </div>
          </div>
        </div>
      )
    }

    if (!loaded) {
      return openId ? <RecordSkeleton /> : <div className="h-full rounded-[14px] border border-dashed" />
    }

    if (creating && editing) {
      return (
        <div className="h-full min-h-0 overflow-auto">
          <RecordEdit draft={editing} aws={aws} creating={creating} folders={folders} onChange={setEditing} />
        </div>
      )
    }

    if (selection?.kind === 'trash') {
      return (
        <TrashPanel
          title={`Lixeira de ${vault.name}`}
          records={trash}
          retentionDays={status.retention_days}
          vaultId={vault.id}
          onChanged={() => void refresh()}
        />
      )
    }

    if (currentFolder) {
      return (
        <FolderView
          vaultId={vault.id}
          folder={currentFolder}
          folders={folders}
          records={records}
          vaultName={vault.name}
          entries={folderEntries(currentFolder)}
          onRecord={(id) => onSelect({ kind: 'record', id })}
          onFolder={(id) => onSelect({ kind: 'folder', id })}
          onSubfolder={() => setAsking({ parentId: currentFolder.id })}
        />
      )
    }

    // Another record was picked and is on its way: its shape, not the previous record.
    if (openId && open?.id !== openId) {
      return <RecordSkeleton />
    }

    if (open && openId) {
      if (editing) {
        return (
          <div className="h-full min-h-0 overflow-auto">
            <RecordEdit
              draft={editing}
              kind={open.kind}
              aws={aws}
              parameter={open.parameter}
              originalTitle={open.payload.title}
              folders={folders}
              onChange={setEditing}
            />
          </div>
        )
      }

      return (
        <RecordView
          key={open.id}
          record={open}
          folders={folders}
          onEdit={() =>
            startEditing({
              payload: structuredClone(open.payload),
              folderId: open.folder_id,
              rotation: open.rotation_due_at?.slice(0, 10) ?? null,
            })
          }
          onReload={reloadOpen}
          onClone={() => void clone(open.id)}
          onMoveToFolder={(folderId) => void moveTo(open.id, folderId)}
          onMoveToVault={() => setMovingToVault({ id: open.id, title: open.payload.title })}
          onCopy={() => setCopying({ vaultId: vault.id, recordIds: [open.id], what: `“${open.payload.title}”` })}
          onTrash={() => setDeleting({ id: open.id, title: open.payload.title })}
          onRemoveAlert={(index) => void removeAlert(index)}
          permanent={aws}
        />
      )
    }

    return (
      <div className="grid h-full place-items-center rounded-[14px] border border-dashed p-8 text-center">
        <div className="max-w-xs space-y-3">
          <p className="text-sm text-muted-foreground">
            {records.length === 0
              ? 'Este cofre está vazio. Comece pelo primeiro registro.'
              : 'Escolha um registro ou uma pasta na árvore. Ctrl P busca em todos os cofres.'}
          </p>
          {records.length === 0 && (
            <Button onClick={() => startCreating(emptyPayload(), null)}>
              <Plus />
              Novo registro
            </Button>
          )}
        </div>
      </div>
    )
  }

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      {bar}
      <div className="kh-wide grid min-h-0 flex-1 grid-cols-[var(--kh-tree,300px)_minmax(0,1fr)] grid-rows-[minmax(0,1fr)] gap-5 p-6">
        <div className="relative grid min-h-0 min-w-0 grid-cols-[minmax(0,1fr)] grid-rows-[minmax(0,1fr)]">
        {loaded ? (
        <TreeCard
          vaultId={vault.id}
          folders={folders}
          records={records}
          attention={attention}
          onEditRecord={(id) => {
            setEditWhenOpen(id)
            select({ kind: 'record', id })
          }}
          onClearCompromised={(id) =>
            void api
              .setCompromised(id, false)
              .then(() => (open?.id === id ? reloadOpen() : refresh()))
              .then(() => toast('Marca de comprometido retirada'))
              .catch((cause) => toastError(cause))
          }
          selection={selection}
          mode={mode}
          filter={filter}
          trashCount={trash.length}
          showTrash={!aws}
          onMode={setMode}
          onFilter={setFilter}
          onRecord={(id) => select({ kind: 'record', id })}
          onFolder={(id) => select({ kind: 'folder', id })}
          onTrash={() => select({ kind: 'trash' })}
          onDrop={(items, folderId) => void moveMany(items, folderId)}
          canMoveFolders={!aws}
          folderEntries={folderEntries}
          recordEntries={recordEntries}
          rootEntries={[
            { kind: 'item', label: 'Nova pasta na raiz', icon: FolderPlus, onSelect: () => setAsking({ parentId: null }) },
            { kind: 'item', label: 'Novo registro na raiz', icon: Plus, onSelect: () => startCreating(emptyPayload(), null) },
          ]}
        />
        ) : (
          <TreeSkeleton />
        )}
          <Splitter which="tree" className="-right-4" />
        </div>
        <div className="min-h-0 min-w-0">{main()}</div>
      </div>

      <DiscardDialog
        open={leaving !== null}
        onKeep={() => setLeaving(null)}
        onDiscard={() => {
          const go = leaving
          setLeaving(null)
          stopEditing()
          go?.()
        }}
      />


      {asking && (
        <NameDialog
          title={asking.parentId ? 'Nova subpasta' : 'Nova pasta'}
          hint={asking.parentId ? `Dentro de ${folderPath(folders, asking.parentId)}.` : 'Na raiz do cofre.'}
          withColour={!aws}
          awsParent={aws ? awsPathOf(asking.parentId) : undefined}
          onConfirm={(name, colour, segment) => {
            const parentId = asking.parentId
            setAsking(null)
            void api
              .createFolder(vault.id, name, parentId, colour, segment)
              .then(refresh)
              .then(() => toast('Pasta criada', name))
              .catch((cause) => toastError(cause, 'Não criei a pasta'))
          }}
          onCancel={() => setAsking(null)}
        />
      )}

      {folderAction?.what === 'rename' && (
        <RenameFolderDialog
          folder={folderAction.folder}
          aws={aws ? { renameDefault: vault.aws?.rename_with_title ?? false } : undefined}
          onDone={(renamed) => {
            const before = folderAction.folder.id
            setFolderAction(null)
            // An AWS path that moved is a new id, and so is every record under it.
            if (renamed && renamed.id !== before) {
              if (selection?.kind === 'folder' && selection.id === before) onSelect({ kind: 'folder', id: renamed.id })
              else if (selection?.kind === 'record') onSelect(null)
            }
            void refresh()
          }}
          onCancel={() => setFolderAction(null)}
        />
      )}
      {folderAction?.what === 'move' && (
        <MoveFolderDialog
          folder={folderAction.folder}
          folders={folders}
          onDone={() => {
            setFolderAction(null)
            void refresh()
          }}
          onCancel={() => setFolderAction(null)}
        />
      )}
      {folderAction?.what === 'delete' && (
        <DeleteFolderDialog
          folder={folderAction.folder}
          records={records.filter((record) => record.folder_id === folderAction.folder.id).length}
          onDone={() => {
            if (selection?.kind === 'folder' && selection.id === folderAction.folder.id) onSelect(null)
            setFolderAction(null)
            void refresh()
          }}
          onCancel={() => setFolderAction(null)}
        />
      )}

      {deleting && (
        <AlertDialog open onOpenChange={(isOpen) => !isOpen && setDeleting(null)}>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>
                {aws ? `Apagar “${deleting.title}” de vez?` : `Mandar “${deleting.title}” para a lixeira?`}
              </AlertDialogTitle>
              <AlertDialogDescription>
                {aws
                  ? 'Um cofre na AWS não tem lixeira: o parâmetro é apagado da conta agora, para todo mundo que tem acesso a ele.'
                  : 'Dá para restaurar de lá enquanto o prazo da lixeira não vencer.'}
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>Cancelar</AlertDialogCancel>
              <AlertDialogAction
                className={aws ? 'bg-destructive text-white hover:bg-destructive/90' : undefined}
                onClick={() => {
                  const id = deleting.id
                  setDeleting(null)
                  void sendToTrash(id)
                }}
              >
                {aws ? 'Apagar de vez' : 'Mandar para a lixeira'}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      )}

      {askingJson && (
        <AlertDialog open onOpenChange={(isOpen) => !isOpen && setAskingJson(false)}>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>O parâmetro vai virar JSON</AlertDialogTitle>
              <AlertDialogDescription>
                Hoje este registro é só uma senha, gravada na AWS como o valor puro do parâmetro, que é o que os
                aplicativos leem. Com o que você acrescentou (alerta, ícone, observação ou outro campo), o Keyhold
                precisa gravar o valor no formato JSON dele, e quem lê esse parâmetro esperando só a senha deixa de
                funcionar.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>Voltar e ajustar</AlertDialogCancel>
              <AlertDialogAction
                className="bg-destructive text-white hover:bg-destructive/90"
                onClick={() => {
                  setAskingJson(false)
                  void save(true)
                }}
              >
                Gravar como JSON
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      )}

      {movingAws && (
        <AlertDialog open onOpenChange={(isOpen) => !isOpen && setMovingAws(null)}>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>
                {movingAws.ids.length > 1 ? `Mover ${movingAws.ids.length} registros na AWS?` : 'Mover na AWS?'}
              </AlertDialogTitle>
              <AlertDialogDescription>
                No Parameter Store, a pasta é parte do nome. Mover grava cada registro com o nome novo e apaga o
                antigo, e o histórico de versões do nome antigo vai junto com ele.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>Cancelar</AlertDialogCancel>
              <AlertDialogAction
                onClick={() => {
                  const { ids, folderId } = movingAws
                  setMovingAws(null)
                  if (ids.length === 1) void moveTo(ids[0], folderId, true)
                  else void moveMany(ids.map((id) => `r:${id}`), folderId, true)
                }}
              >
                Mover
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      )}

      {movingToVault && (
        <MoveToVaultDialog
          ids={[movingToVault.id]}
          what={`“${movingToVault.title}”`}
          vaults={vaults}
          fromVaultId={vault.id}
          onDone={() => {
            if (open?.id === movingToVault.id) onSelect(null)
            setMovingToVault(null)
            void refresh()
            toast('Registro movido para outro cofre')
          }}
          onCancel={() => setMovingToVault(null)}
        />
      )}

      {copying && (
        <CopyDialog
          source={copying}
          vaults={vaults}
          onAccounts={onAccounts}
          onCancel={() => setCopying(null)}
          onDone={() => {
            setCopying(null)
            void refresh()
          }}
        />
      )}
    </div>
  )
}

/** Whether AWS would store this payload as the plain value (see `mapping::encode_value`). */
function isPlainValue(payload: Payload): boolean {
  const [only, ...rest] = payload.fields
  return (
    !!only &&
    rest.length === 0 &&
    only.kind === 'password' &&
    Object.keys(only.extra ?? {}).length === 0 &&
    !only.list &&
    !only.primary &&
    only.value !== '' &&
    !payload.icon &&
    (payload.alerts?.length ?? 0) === 0 &&
    payload.notes.trim() === ''
  )
}
