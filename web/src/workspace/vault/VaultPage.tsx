import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
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
import { NameDialog } from '../NameDialog'
import { NewRecordDialog } from '../record/NewRecordDialog'
import { RecordEdit, type EditDraft } from '../record/RecordEdit'
import { RecordView } from '../record/RecordView'
import { Crumb, TopBar, VaultSwitcher } from '../shell'
import { toast, toastError } from '../toast'
import { TrashPanel } from '../TrashPanel'
import { FolderView } from './FolderView'
import { folderChain, folderPath } from './paths'
import { TreeCard, type Selection, type TreeFilter, type TreeMode } from './TreeCard'

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
  const [attention, setAttention] = useState<Set<string>>(new Set())
  const [mode, setMode] = useState<TreeMode>('tree')
  const [filter, setFilter] = useState<TreeFilter>('all')
  const [open, setOpen] = useState<OpenRecord | null>(null)
  const [editing, setEditing] = useState<EditDraft | null>(null)
  const [creating, setCreating] = useState<{ payload: Payload; folderId: string | null; cloning?: boolean } | null>(null)
  const [asking, setAsking] = useState<{ parentId: string | null } | null>(null)
  const [folderAction, setFolderAction] = useState<{ what: 'rename' | 'move' | 'delete'; folder: FolderNode } | null>(null)
  const [deleting, setDeleting] = useState<{ id: string; title: string } | null>(null)
  const [movingToVault, setMovingToVault] = useState<{ id: string; title: string } | null>(null)
  const [saving, setSaving] = useState(false)
  const [unavailable, setUnavailable] = useState<{ message: string; code?: string } | null>(null)
  const aws = vault.kind === 'aws'
  const [origin, setOrigin] = useState('')
  const [leaving, setLeaving] = useState<(() => void) | null>(null)
  const dirty = editing !== null && JSON.stringify(editing) !== origin
  const editingRef = useRef(false)
  editingRef.current = editing !== null

  useEffect(() => onDirty(dirty), [dirty, onDirty])

  /** Run `go` now, or after "Descartar" when there are unsaved changes. */
  const guard = (go: () => void) => (dirty ? setLeaving(() => go) : go())
  const select = (next: Selection) => guard(() => onSelect(next))

  const startEditing = (draft: EditDraft) => {
    setOrigin(JSON.stringify(draft))
    setEditing(draft)
  }

  const refresh = useCallback(async () => {
    try {
      const [tree, list, bin] = await Promise.all([api.folders(vault.id), api.records(vault.id), api.trash(vault.id)])

      setFolders(tree.folders)
      setRecords(list.records.filter((record) => !record.archived))
      setTrash(bin.records)
      setUnavailable(null)

      // Atenção: what the file already knows (compromised, rotation passed) plus the health report,
      // which opens every record to judge weak and repeated passwords.
      const flagged = new Set(
        list.records
          .filter(
            (record) =>
              record.compromised_at !== null ||
              (record.rotation_due_at !== null && new Date(record.rotation_due_at) <= new Date()),
          )
          .map((record) => record.id),
      )
      setAttention(flagged)

      void api
        .health(vault.id)
        .then((report) => setAttention(new Set([...flagged, ...report.health.findings.map((one) => one.record_id)])))
        .catch(() => {})
    } catch (cause) {
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
  async function save() {
    if (!open || !editing) return
    setSaving(true)

    const before = JSON.parse(origin) as EditDraft
    const payloadChanged = JSON.stringify(editing.payload) !== JSON.stringify(before.payload)
    let id = open.id
    let wrote = false

    try {
      if (payloadChanged) {
        await api.saveRecord(id, editing.payload, aws ? open.version : undefined)
        wrote = true
      }
      if (editing.folderId !== open.folder_id) {
        const moved = await api.moveRecord(id, editing.folderId)
        if (moved.id) id = moved.id
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

  async function create(payload: Payload) {
    if (!creating) return

    try {
      const made = await api.createRecord(vault.id, creating.folderId, payload)
      setCreating(null)
      await refresh()
      onSelect({ kind: 'record', id: made.record.id })
      toast('Registro criado', payload.title)
    } catch (cause) {
      toastError(cause, 'Não criei o registro')
    }
  }

  async function clone(id: string) {
    try {
      const found = await api.openRecord(id)
      setCreating({
        payload: { ...found.record.payload, title: `${found.record.payload.title} (cópia)` },
        folderId: found.record.folder_id,
        cloning: true,
      })
    } catch (cause) {
      toastError(cause)
    }
  }

  async function moveTo(id: string, folderId: string | null) {
    try {
      const moved = await api.moveRecord(id, folderId)
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

  const folderEntries = (folder: FolderNode): MenuEntry[] => aws ? awsFolderEntries(folder) : [
    {
      kind: 'item',
      label: 'Registro novo aqui',
      icon: Plus,
      onSelect: () => setCreating({ payload: emptyPayload(), folderId: folder.id }),
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
    { kind: 'separator' },
    { kind: 'item', label: 'Excluir', icon: Trash2, destructive: true, onSelect: () => setFolderAction({ what: 'delete', folder }) },
  ]

  /** In Parameter Store a folder is a path: it can hold new things and be removed when empty. */
  const awsFolderEntries = (folder: FolderNode): MenuEntry[] => [
    {
      kind: 'item',
      label: 'Registro novo aqui',
      icon: Plus,
      onSelect: () => setCreating({ payload: emptyPayload(), folderId: folder.id }),
    },
    { kind: 'item', label: 'Subpasta', icon: FolderPlus, onSelect: () => setAsking({ parentId: folder.id }) },
    { kind: 'separator' },
    { kind: 'item', label: 'Excluir a pasta vazia', icon: Trash2, destructive: true, onSelect: () => setFolderAction({ what: 'delete', folder }) },
  ]

  const recordEntries = (record: RecordPreview): MenuEntry[] => [
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
            <Button variant="outline" onClick={() => guard(() => setEditing(null))}>
              Cancelar
            </Button>
            <Button disabled={saving || !editing.payload.title.trim()} onClick={() => void save()}>
              Salvar alterações
            </Button>
          </>
        ) : (
          <Button onClick={() => setCreating({ payload: emptyPayload(), folderId: newHere })}>
            <Plus />
            Novo registro
          </Button>
        )
      }
    >
      <VaultSwitcher vaults={vaults} current={vault} onPick={(id) => guard(() => onVault(id))} onNew={onNewVault} onManage={onManageVaults} />
      {chain.map((folder, index) => (
        <Crumb
          key={folder.id}
          last={index === chain.length - 1 && !openId}
          onClick={() => select({ kind: 'folder', id: folder.id })}
        >
          {folder.name}
        </Crumb>
      ))}
      {open && openId && (
        <Crumb last={!editing} onClick={() => guard(() => setEditing(null))}>
          {open.payload.title}
        </Crumb>
      )}
      {editing && <Crumb last>Editar</Crumb>}
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

    if (open && openId) {
      if (editing) {
        return (
          <div className="h-full min-h-0 overflow-auto">
            <RecordEdit draft={editing} kind={open.kind} folders={folders} onChange={setEditing} />
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
          onTrash={() => setDeleting({ id: open.id, title: open.payload.title })}
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
            <Button onClick={() => setCreating({ payload: emptyPayload(), folderId: null })}>
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
      <div className="grid min-h-0 flex-1 grid-cols-[300px_minmax(0,1fr)] grid-rows-[minmax(0,1fr)] gap-5 p-6 max-xl:grid-cols-[260px_minmax(0,1fr)]">
        <TreeCard
          folders={folders}
          records={records}
          attention={attention}
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
          onDrop={(id, folderId) => void moveTo(id, folderId)}
          folderEntries={folderEntries}
          recordEntries={recordEntries}
        />
        <div className="min-h-0 min-w-0">{main()}</div>
      </div>

      <DiscardDialog
        open={leaving !== null}
        onKeep={() => setLeaving(null)}
        onDiscard={() => {
          const go = leaving
          setLeaving(null)
          setEditing(null)
          go?.()
        }}
      />

      {creating && (
        <NewRecordDialog
          initial={creating.payload}
          cloning={creating.cloning}
          where={`Em ${vault.name}, ${folderPath(folders, creating.folderId) ? `em ${folderPath(folders, creating.folderId)}` : 'na raiz'}`}
          onCreate={create}
          onCancel={() => setCreating(null)}
        />
      )}

      {asking && (
        <NameDialog
          title={asking.parentId ? 'Nova subpasta' : 'Nova pasta'}
          hint={asking.parentId ? `Dentro de ${folderPath(folders, asking.parentId)}.` : 'Na raiz do cofre.'}
          withColour
          onConfirm={(name, colour) => {
            const parentId = asking.parentId
            setAsking(null)
            void api
              .createFolder(vault.id, name, parentId, colour)
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
          onDone={() => {
            setFolderAction(null)
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
    </div>
  )
}
