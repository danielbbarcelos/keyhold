import { useEffect, useState } from 'react'
import { ArrowRight, Bot, Folder, HardDrive, Lock, Plus, ShieldCheck, Star } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

import {
  api,
  type AgentGrant,
  type AuditEvent,
  type HealthIssue,
  type RecordPreview,
  type Status,
  type VaultSummary,
} from '../api'
import { RecordTile } from './icons'
import { AwsMark } from './AwsMark'
import { Body, Panel, PanelHead, TopBar, Title, VaultInitial } from './shell'
import { when } from './vault/paths'

/**
 * Início: where the day starts. The vaults, what was opened lately, what is starred or asks for
 * attention across every vault, and what agents have been doing.
 */
export interface Seen {
  record: RecordPreview
  at: number
}

const ISSUE: Record<HealthIssue, { label: string; tone: 'rose' | 'amber' }> = {
  compromised: { label: 'comprometido', tone: 'rose' },
  rotation: { label: 'troca vencida', tone: 'rose' },
  reused: { label: 'senha repetida', tone: 'amber' },
  weak: { label: 'senha fraca', tone: 'amber' },
  stale: { label: 'senha antiga', tone: 'amber' },
}

export function Home({
  status,
  vaults,
  records,
  seen,
  onOpen,
  onVault,
  onNewVault,
  onTrail,
  onSettings,
  onLock,
  onSearch,
}: {
  status: Status
  vaults: VaultSummary[]
  records: RecordPreview[]
  seen: Seen[]
  onOpen: (record: RecordPreview) => void
  onVault: (id: string) => void
  onNewVault: () => void
  onTrail: () => void
  onSettings: () => void
  onLock: () => void
  onSearch: () => void
}) {
  const [issues, setIssues] = useState<Map<string, HealthIssue[]>>(new Map())
  const [events, setEvents] = useState<AuditEvent[]>([])
  const [grants, setGrants] = useState<AgentGrant[]>([])
  const [tab, setTab] = useState<'tudo' | 'favoritos' | 'atencao'>('tudo')
  const open = vaults.filter((vault) => !vault.archived)

  useEffect(() => {
    void Promise.all(open.map((vault) => api.health(vault.id).catch(() => null)))
      .then((reports) => {
        const found = new Map<string, HealthIssue[]>()
        reports.forEach((report) => report?.health.findings.forEach((one) => found.set(one.record_id, one.issues)))
        setIssues(found)
      })
    void api.audit(60).then((answer) => setEvents(answer.events)).catch(() => {})
    void api.agentGrants().then((answer) => setGrants(answer.grants)).catch(() => {})
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [vaults])

  const hour = new Date().getHours()
  const hello = hour < 12 ? 'Bom dia' : hour < 18 ? 'Boa tarde' : 'Boa noite'
  const attention = records.filter((record) => issues.has(record.id) || record.compromised_at)
  const favourites = records.filter((record) => record.favourite)
  const vaultName = (id: string) => vaults.find((vault) => vault.id === id)?.name ?? ''
  const vaultOf = (id: string) => vaults.find((vault) => vault.id === id)

  const recent: RecordPreview[] = seen.length
    ? seen.map((one) => records.find((record) => record.id === one.record.id) ?? one.record).slice(0, 5)
    : [...records].sort((a, b) => b.updated_at.localeCompare(a.updated_at)).slice(0, 5)

  const starred =
    tab === 'favoritos'
      ? favourites
      : tab === 'atencao'
        ? attention
        : [...favourites, ...attention.filter((record) => !record.favourite)]

  const agentEvents = events.filter((event) => event.actor.kind === 'agent').slice(0, 3)
  const unlockedAt = events.find((event) => event.action === 'vault.unlock')?.at
  const seconds = status.lock.locks_in

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <TopBar
        onSearch={onSearch}
        actions={
          <Button variant="outline" onClick={onNewVault}>
            <Plus />
            Novo cofre
          </Button>
        }
      >
        <Title>Início</Title>
      </TopBar>

      <Body>
        <h1 className="text-[22px] font-semibold tracking-tight">{hello}</h1>
        <p className="mt-1 text-muted-foreground">
          {open.length} {open.length === 1 ? 'cofre' : 'cofres'}, {records.length}{' '}
          {records.length === 1 ? 'registro' : 'registros'}
          {attention.length > 0 && `, ${attention.length} ${attention.length === 1 ? 'pede' : 'pedem'} atenção`}.
        </p>

        <div className="mt-6 grid grid-cols-[repeat(auto-fill,minmax(260px,1fr))] gap-5">
          {open.map((vault) => {
            const inside = records.filter((record) => record.vault_id === vault.id)
            const flagged = inside.filter((record) => issues.has(record.id) || record.compromised_at).length

            return (
              <button
                key={vault.id}
                type="button"
                onClick={() => onVault(vault.id)}
                className="rounded-[14px] border bg-card p-[18px] text-left hover:border-line-2"
              >
                <span className="flex items-center gap-3">
                  <VaultInitial vault={vault} size="lg" />
                  <span className="min-w-0">
                    <b className="block truncate text-[15px] font-semibold">{vault.name}</b>
                    <span className="flex items-center gap-1 text-xs text-faint">
                      {vault.kind === 'aws' ? <AwsMark className="size-3.5" /> : <HardDrive className="size-3" />}
                      {vault.kind === 'aws' ? `AWS Parameter Store, ${vault.aws?.region ?? ''}` : 'Cofre local'}
                    </span>
                  </span>
                </span>
                <span className="mt-4 flex flex-wrap gap-x-4 gap-y-1 text-[13px] text-muted-foreground">
                  <span>
                    <b className="font-semibold text-foreground">{inside.length}</b> registros
                  </span>
                  <span className={cn(flagged > 0 && 'text-amber')}>
                    <b className={cn('font-semibold', flagged > 0 ? 'text-amber' : 'text-foreground')}>{flagged}</b> atenção
                  </span>
                  <span>{vault.agent_access ? 'agente ligado' : 'agente desligado'}</span>
                </span>
              </button>
            )
          })}
          <button
            type="button"
            onClick={onNewVault}
            className="flex min-h-[118px] flex-col items-center justify-center gap-2 rounded-[14px] border border-dashed text-sm text-muted-foreground hover:bg-hover"
          >
            <Plus className="size-5" />
            Novo cofre
          </button>
        </div>

        <div className="mt-5 grid grid-cols-1 gap-5 xl:grid-cols-2">
          <Panel>
            <PanelHead title={seen.length ? 'Abertos recentemente' : 'Alterados recentemente'} />
            {recent.map((record) => (
              <RecordLine key={record.id} record={record} folder={vaultOf(record.vault_id)} onOpen={() => onOpen(record)}>
                <span className="text-xs text-faint">
                  {when(seen.length ? new Date(seen.find((one) => one.record.id === record.id)?.at ?? Date.now()).toISOString() : record.updated_at)}
                </span>
              </RecordLine>
            ))}
            {recent.length === 0 && <Empty>Nenhum registro ainda. Abra um cofre e crie o primeiro.</Empty>}
          </Panel>

          <Panel>
            <PanelHead title="Favoritos e atenção">
              <div className="flex rounded-lg border bg-raised p-0.5">
                {(
                  [
                    ['tudo', 'Tudo'],
                    ['favoritos', 'Favoritos'],
                    ['atencao', 'Atenção'],
                  ] as const
                ).map(([id, label]) => (
                  <button
                    key={id}
                    type="button"
                    onClick={() => setTab(id)}
                    className={cn(
                      'h-7 rounded-md px-2.5 text-[13px] font-medium text-muted-foreground',
                      tab === id && 'bg-card text-foreground shadow-sm',
                    )}
                  >
                    {label}
                  </button>
                ))}
              </div>
            </PanelHead>
            {starred.slice(0, 6).map((record) => {
              const found = issues.get(record.id) ?? (record.compromised_at ? (['compromised'] as HealthIssue[]) : [])

              return (
                <RecordLine key={record.id} record={record} vault={vaultName(record.vault_id)} onOpen={() => onOpen(record)}>
                  {found.length > 0 ? (
                    <span
                      className={cn(
                        'rounded-full px-2.5 py-0.5 text-xs font-medium',
                        ISSUE[found[0]].tone === 'rose' ? 'bg-rose-soft text-rose' : 'bg-amber-soft text-amber',
                      )}
                    >
                      {ISSUE[found[0]].label}
                    </span>
                  ) : (
                    record.favourite && <Star className="size-4 fill-primary text-primary" />
                  )}
                </RecordLine>
              )
            })}
            {starred.length === 0 && (
              <Empty>
                {tab === 'atencao'
                  ? 'Nada pede atenção. Senhas fracas, repetidas ou vencidas aparecem aqui.'
                  : 'Nenhum favorito. A estrela de um registro o traz para cá.'}
              </Empty>
            )}
          </Panel>
        </div>

        <Panel className="mt-5">
          <PanelHead title="Atividade e agentes">
            <Button variant="ghost" size="sm" onClick={onTrail}>
              Ver a trilha
              <ArrowRight />
            </Button>
          </PanelHead>
          <Line icon={Lock} tone="amber" title={unlockedAt ? `Destrancado desde ${new Date(unlockedAt.endsWith('Z') ? unlockedAt : `${unlockedAt}Z`).toLocaleTimeString('pt-BR', { hour: '2-digit', minute: '2-digit' })}` : 'Destrancado'}
            detail={seconds === null ? 'sem tranca automática' : `tranca em ${seconds >= 60 ? `${Math.ceil(seconds / 60)} min` : `${seconds} s`} parado`}>
            <Button variant="outline" size="sm" onClick={onLock}>
              Trancar
            </Button>
          </Line>
          {agentEvents.map((event) => (
            <Line
              key={event.seq}
              icon={Bot}
              title={`${event.actor.process ?? 'Agente'} ${event.result === 'denied' ? 'teve um pedido negado' : event.action === 'secret.copied' ? 'copiou um segredo' : 'leu um segredo'}${event.scope ? ` de ${records.find((record) => record.id === event.scope)?.title ?? 'um registro'}` : ''}`}
              detail={when(event.at)}
            />
          ))}
          <Line
            icon={ShieldCheck}
            title={grants.length === 0 ? 'Nenhuma permissão ativa' : `${grants.length} ${grants.length === 1 ? 'permissão ativa' : 'permissões ativas'}`}
            detail={(() => {
              const on = open.filter((vault) => vault.agent_access).length

              return on === 0 ? 'agente desligado em todos os cofres' : `agente ligado em ${on} ${on === 1 ? 'cofre' : 'cofres'}`
            })()}
          >
            <Button variant="ghost" size="sm" onClick={onSettings}>
              ver em Ajustes
            </Button>
          </Line>
        </Panel>
      </Body>
    </div>
  )

  function RecordLine({
    record,
    vault,
    folder,
    onOpen,
    children,
  }: {
    record: RecordPreview
    vault?: string
    folder?: VaultSummary
    onOpen: () => void
    children?: React.ReactNode
  }) {
    return (
      <button
        type="button"
        onClick={onOpen}
        className="flex h-[52px] w-full items-center gap-3 border-b px-5 text-left last:border-b-0 hover:bg-hover"
      >
        <RecordTile icon={record.icon} kind={record.kind} />
        <span className="min-w-0 flex-1">
          <b className="block truncate text-sm font-medium">{record.title}</b>
          <span className="flex items-center gap-1 truncate text-xs text-faint">
            {folder ? <VaultInitial vault={folder} /> : <Folder className="size-3" />}
            {vault ?? folder?.name ?? ''}
            {record.subtitle && `, ${record.subtitle}`}
          </span>
        </span>
        {children}
      </button>
    )
  }
}

function Line({
  icon: Icon,
  tone,
  title,
  detail,
  children,
}: {
  icon: React.ComponentType<{ className?: string }>
  tone?: 'amber'
  title: string
  detail: string
  children?: React.ReactNode
}) {
  return (
    <div className="flex h-[52px] items-center gap-3 border-b px-5 last:border-b-0">
      <span className={cn('grid size-8 place-items-center rounded-lg', tone === 'amber' ? 'bg-amber-soft text-amber' : 'bg-stone-soft text-muted-foreground')}>
        <Icon className="size-4" />
      </span>
      <span className="min-w-0 flex-1">
        <b className="block truncate text-sm font-medium">{title}</b>
        <span className="block truncate text-xs text-faint">{detail}</span>
      </span>
      {children}
    </div>
  )
}

function Empty({ children }: { children: React.ReactNode }) {
  return <p className="px-5 py-6 text-sm text-faint">{children}</p>
}

