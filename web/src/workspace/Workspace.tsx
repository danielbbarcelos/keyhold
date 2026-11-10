import { useCallback, useEffect, useMemo, useState } from 'react'

import { api, type RecordPreview, type Status, type VaultSummary } from '../api'
import { clearNow } from '../clipboard'
import { AgentApproval } from './AgentApproval'
import { AuditPanel } from './AuditPanel'
import { CommandPalette } from './CommandPalette'
import { Home, type Seen } from './Home'
import { InstancesPanel } from './InstancesPanel'
import { NewVaultDialog } from './NewVaultDialog'
import { Settings } from './Settings'
import { Body, Crumb, Rail, Title, TopBar, type Place } from './shell'
import { toast, toastError, Toaster } from './toast'
import type { Selection } from './vault/TreeCard'
import { VaultPage } from './vault/VaultPage'

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
  const [vaults, setVaults] = useState<VaultSummary[]>([])
  const [records, setRecords] = useState<RecordPreview[]>([])
  const [place, setPlace] = useState<Place>('home')
  const [vaultId, setVaultId] = useState<string | null>(null)
  const [selection, setSelection] = useState<Selection>(null)
  const [seen, setSeen] = useState<Seen[]>([])
  const [paletteOpen, setPaletteOpen] = useState(false)
  const [askingVault, setAskingVault] = useState(false)
  const [section, setSection] = useState('')

  const refresh = useCallback(async () => {
    try {
      const found = await api.vaults()
      setVaults(found.vaults)

      const lists = await Promise.all(
        found.vaults.filter((vault) => !vault.archived).map((vault) => api.records(vault.id).catch(() => null)),
      )
      setRecords(lists.flatMap((list) => list?.records ?? []).filter((record) => !record.archived))
    } catch (cause) {
      toastError(cause, 'Não consegui ler os cofres')
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

  function openRecord(record: Pick<RecordPreview, 'id' | 'vault_id'>) {
    openVault(record.vault_id, { kind: 'record', id: record.id })
  }

  const lock = () =>
    void api
      .lock()
      .then(onChanged)
      .catch((cause) => toastError(cause, 'Não tranquei'))

  const onSeen = useCallback((record: RecordPreview) => {
    setSeen((current) => [{ record, at: Date.now() }, ...current.filter((one) => one.record.id !== record.id)].slice(0, 12))
  }, [])

  const search = () => {
    void refresh()
    setPaletteOpen(true)
  }

  function page() {
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
          onManageVaults={() => setPlace('settings')}
          onSearch={search}
          onSeen={onSeen}
          onAccounts={() => setPlace('settings')}
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
        seen={seen}
        onOpen={openRecord}
        onVault={(id) => openVault(id)}
        onNewVault={() => setAskingVault(true)}
        onTrail={() => setPlace('trail')}
        onSettings={() => setPlace('settings')}
        onLock={lock}
        onSearch={search}
      />
    )
  }

  return (
    <div className="grid h-screen grid-cols-[76px_minmax(0,1fr)] grid-rows-[minmax(0,1fr)] overflow-hidden bg-background">
      <Rail
        place={place === 'vault' && !vault ? 'home' : place}
        status={status}
        onGo={(next) => setPlace(next)}
        onLock={lock}
        onChanged={onChanged}
      />

      <main className="flex min-h-0 min-w-0">{page()}</main>

      {askingVault && (
        <NewVaultDialog
          onAccounts={() => {
            setAskingVault(false)
            setPlace('settings')
          }}
          onCreate={(name, aws) => {
            setAskingVault(false)
            void api
              .createVault(name, 'default', aws)
              .then(async (made) => {
                await refresh()
                toast('Cofre criado', aws ? `${name}, no AWS Parameter Store` : name)
                openVault(made.vault.id)
              })
              .catch((cause) => toastError(cause, 'Não criei o cofre'))
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
          openRecord(record)
        }}
      />

      <Toaster />
    </div>
  )
}
