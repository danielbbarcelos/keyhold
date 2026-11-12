import { useCallback, useEffect, useMemo, useState } from 'react'

import { api, type RecordPreview, type Status, type VaultSummary } from '../api'
import { clearNow } from '../clipboard'
import { AgentApproval } from './AgentApproval'
import { AuditPanel } from './AuditPanel'
import { CommandPalette } from './CommandPalette'
import { DiscardDialog } from './DiscardDialog'
import { Home, type Seen } from './Home'
import { InstancesPanel } from './InstancesPanel'
import { destinationFromLocation, rememberDestination, type Destination } from './nav'
import { NewVaultDialog } from './NewVaultDialog'
import { Settings } from './Settings'
import { Body, Crumb, Rail, Title, TopBar, type Place } from './shell'
import { toast, toastError, Toaster } from './toast'
import type { Selection } from './vault/TreeCard'
import { VaultPage } from './vault/VaultPage'
import { VaultSkeleton } from './Skeletons'

/**
 * The workspace: one shell for every screen.
 *
 * A narrow rail with what belongs to Keyhold itself (Início, Trilha, Instâncias, Ajustes and the
 * lock), and a working area that is always a top bar over a body of cards. Inside a vault the body
 * is the vault's tree beside whatever was picked in it. There is no route table: a locked vault has
 * nowhere to navigate to, and a URL would describe the shape of what is inside.
 */
interface WorkspaceProps {
  status: Status
  onChanged: () => void
}

export function Workspace({ status, onChanged }: WorkspaceProps) {
  // A tab opened with "Abrir em nova guia" starts where it was sent.
  const [start] = useState<Destination>(destinationFromLocation)
  const [vaults, setVaults] = useState<VaultSummary[]>([])
  const [vaultsLoaded, setVaultsLoaded] = useState(false)
  const [records, setRecords] = useState<RecordPreview[]>([])
  // Vaults whose records could not be read (an AWS vault without a session): unknown, not empty.
  const [unreadable, setUnreadable] = useState<Set<string>>(new Set())
  const [place, setPlace] = useState<Place>(start.place)
  const [vaultId, setVaultId] = useState<string | null>(start.place === 'vault' ? start.vault : null)
  const [selection, setSelection] = useState<Selection>(() => {
    if (start.place !== 'vault') return null
    if (start.record) return { kind: 'record', id: start.record }
    if (start.folder) return { kind: 'folder', id: start.folder }
    return start.trash ? { kind: 'trash' } : null
  })
  // A vault being created: its page stands as a skeleton until the binary answers.
  const [pendingVault, setPendingVault] = useState(false)
  const [seen, setSeen] = useState<Seen[]>([])
  const [paletteOpen, setPaletteOpen] = useState(false)
  const [askingVault, setAskingVault] = useState(false)
  const [section, setSection] = useState('')
  // Which card Ajustes opens on; the counter makes a second "Abrir Contas AWS" land there again.
  const [settingsAt, setSettingsAt] = useState({
    id: start.place === 'settings' && start.section ? start.section : 'aparencia',
    n: 0,
  })
  const [dirty, setDirty] = useState(false)
  const [leaving, setLeaving] = useState<(() => void) | null>(null)

  /** Leaving the vault page with an unsaved edit asks first. */
  const guard = (go: () => void) => (dirty ? setLeaving(() => go) : go())

  // Somebody is here: a key press or a click says so to the binary, at most every 20 s. Reads no
  // longer count as activity (screens poll with them), so this is what keeps an attended vault open
  // while a person reads a record without changing anything.
  useEffect(() => {
    let last = 0
    const here = () => {
      const now = Date.now()
      if (now - last < 20_000) return
      last = now
      void api.heartbeat().catch(() => undefined)
    }
    window.addEventListener('pointerdown', here, true)
    window.addEventListener('keydown', here, true)

    return () => {
      window.removeEventListener('pointerdown', here, true)
      window.removeEventListener('keydown', here, true)
    }
  }, [])

  // Closing the tab or reloading with an unsaved edit gets the browser's own question.
  useEffect(() => {
    if (!dirty) return
    const ask = (event: BeforeUnloadEvent) => event.preventDefault()
    window.addEventListener('beforeunload', ask)

    return () => window.removeEventListener('beforeunload', ask)
  }, [dirty])

  const refresh = useCallback(async () => {
    try {
      const found = await api.vaults()
      setVaults(found.vaults)
      setVaultsLoaded(true)

      const open = found.vaults.filter((vault) => !vault.archived)
      const lists = await Promise.all(open.map((vault) => api.records(vault.id).catch(() => null)))
      setRecords(lists.flatMap((list) => list?.records ?? []).filter((record) => !record.archived))
      setUnreadable(new Set(open.filter((_, index) => lists[index] === null).map((vault) => vault.id)))
    } catch (cause) {
      setVaultsLoaded(true)
      toastError(cause, 'Não foi possível carregar os cofres')
    }
  }, [])

  useEffect(() => void refresh(), [refresh])

  // Coming back to Início or opening the search should show what changed inside a vault.
  useEffect(() => {
    if (place !== 'vault') void refresh()
  }, [place, refresh])

  // Locking is not a screen change: it takes the clipboard back too.
  useEffect(() => {
    if (!status.lock.unlocked) clearNow()
  }, [status.lock.unlocked])

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'p') {
        event.preventDefault()
        void refresh()
        setPaletteOpen((open) => !open)
      }
    }

    window.addEventListener('keydown', onKey)

    return () => window.removeEventListener('keydown', onKey)
  }, [refresh])

  const vault = vaults.find((one) => one.id === vaultId && !one.archived) ?? null

  // The address follows the tab, so a reload lands where it was.
  useEffect(() => {
    if (place === 'vault') {
      if (!vaultId) return
      rememberDestination({
        place: 'vault',
        vault: vaultId,
        record: selection?.kind === 'record' ? selection.id : undefined,
        folder: selection?.kind === 'folder' ? selection.id : undefined,
        trash: selection?.kind === 'trash',
      })
    } else if (place === 'settings') {
      rememberDestination({ place, section: settingsAt.id })
    } else {
      rememberDestination({ place })
    }
  }, [place, vaultId, selection, settingsAt.id])

  const names = useMemo(() => {
    const out: Record<string, string> = {}
    records.forEach((record) => {
      out[record.id] = `${vaults.find((one) => one.id === record.vault_id)?.name ?? ''}, ${record.title}`
    })

    return out
  }, [records, vaults])

  function openVault(id: string, next: Selection = null) {
    setVaultId(id)
    setSelection(next)
    setPlace('vault')
  }

  /** Ajustes, on one of its sections (the first one when none is said). */
  function goSettings(id = 'aparencia') {
    setSettingsAt((current) => ({ id, n: current.n + 1 }))
    setPlace('settings')
  }

  function openRecord(record: Pick<RecordPreview, 'id' | 'vault_id'>) {
    openVault(record.vault_id, { kind: 'record', id: record.id })
  }

  const lock = () =>
    void api
      .lock()
      .then(onChanged)
      .catch((cause) => toastError(cause, 'Não foi possível trancar o cofre'))

  const onSeen = useCallback((record: RecordPreview) => {
    setSeen((current) => [{ record, at: Date.now() }, ...current.filter((one) => one.record.id !== record.id)].slice(0, 12))
  }, [])

  const search = () => {
    void refresh()
    setPaletteOpen(true)
  }

  function page() {
    if (pendingVault || (place === 'vault' && !vault && !vaultsLoaded)) {
      return <VaultSkeleton />
    }

    if (place === 'vault' && vault) {
      return (
        <VaultPage
          key={vault.id}
          status={status}
          vault={vault}
          vaults={vaults}
          selection={selection}
          onSelect={setSelection}
          onVault={(id) => openVault(id)}
          onNewVault={() => setAskingVault(true)}
          onManageVaults={() => guard(() => goSettings('cofres'))}
          onSearch={search}
          onSeen={onSeen}
          onAccounts={() => guard(() => goSettings('aws'))}
          onDirty={setDirty}
        />
      )
    }

    if (place === 'trail') {
      return (
        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          <TopBar onSearch={search}>
            <Title>Trilha</Title>
          </TopBar>
          <Body className="flex flex-col">
            <AuditPanel
              names={names}
              vaultOf={(id) => records.find((one) => one.id === id)?.vault_id}
              onOpenRecord={(id) => {
                const record = records.find((one) => one.id === id)
                if (record) openRecord(record)
              }}
            />
          </Body>
        </div>
      )
    }

    if (place === 'instances') {
      return (
        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          <TopBar onSearch={search}>
            <Title>Instâncias</Title>
          </TopBar>
          <Body>
            <InstancesPanel />
          </Body>
        </div>
      )
    }

    if (place === 'settings') {
      return (
        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          <TopBar onSearch={search}>
            <span>Ajustes</span>
            {section && <Crumb last>{section}</Crumb>}
          </TopBar>
          <Settings
            initial={settingsAt.id}
            request={settingsAt.n}
            status={status}
            vaults={vaults}
            onChanged={onChanged}
            onVaultsChanged={() => void refresh()}
            onSection={setSection}
          />
        </div>
      )
    }

    return (
      <Home
        status={status}
        vaults={vaults}
        records={records}
        unreadable={unreadable}
        seen={seen}
        onOpen={openRecord}
        onVault={(id) => openVault(id)}
        onNewVault={() => setAskingVault(true)}
        onTrail={() => setPlace('trail')}
        onSettings={() => goSettings('agentes')}
        onLock={lock}
        onSearch={search}
      />
    )
  }

  return (
    <div className="grid h-screen grid-cols-[88px_minmax(0,1fr)] grid-rows-[minmax(0,1fr)] overflow-hidden bg-background">
      <Rail
        place={place === 'vault' && !vault ? 'home' : place}
        status={status}
        onGo={(next) => guard(() => (next === 'settings' ? goSettings() : setPlace(next)))}
        onSettings={(id) => guard(() => goSettings(id))}
        onLock={lock}
        onChanged={onChanged}
      />

      <main className="flex min-h-0 min-w-0">{page()}</main>

      {askingVault && (
        <NewVaultDialog
          onAccounts={() => {
            setAskingVault(false)
            guard(() => goSettings('aws'))
          }}
          onCreate={(name, aws, icon) => {
            setAskingVault(false)
            setPendingVault(true)
            void api
              .createVault(name, 'default', aws, icon)
              .then((made) => {
                // Straight to the new vault: its page loads itself, and the rest of the
                // workspace catches up behind it.
                setVaults((current) => [...current.filter((one) => one.id !== made.vault.id), made.vault])
                toast('Cofre criado', aws ? `${name}, no AWS Parameter Store` : name)
                openVault(made.vault.id)
                void refresh()
              })
              .catch((cause) => toastError(cause, 'Não foi possível criar o cofre'))
              .finally(() => setPendingVault(false))
          }}
          onCancel={() => setAskingVault(false)}
        />
      )}

      {/* An agent asks whenever it asks: the question lives above every screen. */}
      <AgentApproval onAnswered={() => void refresh()} />

      <CommandPalette
        records={records}
        vaults={vaults}
        open={paletteOpen}
        onOpenChange={setPaletteOpen}
        onPick={(record) => {
          setPaletteOpen(false)
          guard(() => openRecord(record))
        }}
      />

      <DiscardDialog
        open={leaving !== null}
        onKeep={() => setLeaving(null)}
        onDiscard={() => {
          const go = leaving
          setLeaving(null)
          setDirty(false)
          go?.()
        }}
      />

      <Toaster />
    </div>
  )
}
