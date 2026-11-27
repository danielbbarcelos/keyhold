import { useEffect, useRef, useState } from 'react'
import { ArrowRight, Bot, ChevronLeft, ChevronRight, Copy, Folder, Globe, Lock, Plus, ShieldCheck, Star, Vault, X } from 'lucide-react'

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
import { locale, t } from '../i18n'
import { RecordTile } from './icons'
import { linkProps, TabMenu } from './nav'
import { Body, Panel, PanelHead, TopBar, Title, VaultInitial } from './shell'
import { when } from './vault/paths'
import { byUse, useHealthIssues, VaultCard } from './VaultCard'
import { toast } from './toast'

/**
 * Início: where the day starts. The vaults, what was opened lately, what is starred or asks for
 * attention across every vault, and what agents have been doing.
 */
export interface Seen {
  record: RecordPreview
  at: number
}

const issueOf = (): Record<HealthIssue, { label: string; tone: 'rose' | 'amber' }> => ({
  compromised: { label: t('comprometido'), tone: 'rose' },
  rotation: { label: t('troca vencida'), tone: 'rose' },
  reused: { label: t('senha repetida'), tone: 'amber' },
  weak: { label: t('senha fraca'), tone: 'amber' },
  stale: { label: t('senha antiga'), tone: 'amber' },
})

export function Home({
  status,
  vaults,
  records,
  seen,
  unreadable,
  onOpen,
  onVault,
  onNewVault,
  onAllVaults,
  onVaultsChanged,
  onTrail,
  onSettings,
  onLock,
  onSearch,
}: {
  status: Status
  vaults: VaultSummary[]
  records: RecordPreview[]
  seen: Seen[]
  /** Vaults that could not be read just now: shown as unavailable, never as empty. */
  unreadable: Set<string>
  onOpen: (record: RecordPreview) => void
  onVault: (id: string) => void
  onNewVault: () => void
  /** The Cofres page. */
  onAllVaults: () => void
  /** A vault was starred or unstarred. */
  onVaultsChanged: () => void
  onTrail: () => void
  onSettings: () => void
  onLock: () => void
  onSearch: () => void
}) {
  const issues = useHealthIssues(vaults)
  const [events, setEvents] = useState<AuditEvent[]>([])
  const [grants, setGrants] = useState<AgentGrant[]>([])
  const [tab, setTab] = useState<'tudo' | 'favoritos' | 'atencao'>('tudo')
  const open = vaults.filter((vault) => !vault.archived)

  useEffect(() => {
    void api.audit(60).then((answer) => setEvents(answer.events)).catch(() => {})
    void api.agentGrants().then((answer) => setGrants(answer.grants)).catch(() => {})
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [vaults])

  const hello = t('Visão geral')
  const ISSUE = issueOf()
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
            {t('Novo cofre')}
          </Button>
        }
      >
        <Title>{t('Início')}</Title>
      </TopBar>

      <Body>
        <h1 className="text-[22px] font-semibold tracking-tight">{hello}</h1>
        <p className="mt-1 text-muted-foreground">
          {[
            open.length === 1 ? t('1 cofre') : t('{n} cofres', { n: open.length }),
            records.length === 1 ? t('1 registro') : t('{n} registros', { n: records.length }),
            attention.length > 0 &&
              (attention.length === 1 ? t('1 pede atenção') : t('{n} pedem atenção', { n: attention.length })),
          ]
            .filter(Boolean)
            .join(', ')}
          .
        </p>

        <SetHostHint dismissed={status.set_host_hint_dismissed ?? false} />

        <VaultCarousel
          vaults={open}
          records={records}
          issues={issues}
          unreadable={unreadable}
          onVault={onVault}
          onAll={onAllVaults}
          onChanged={onVaultsChanged}
        />

        <div className="mt-5 grid grid-cols-1 gap-5 xl:grid-cols-2">
          <Panel>
            <PanelHead title={seen.length ? t('Abertos recentemente') : t('Alterados recentemente')} />
            {recent.map((record) => (
              <RecordLine key={record.id} record={record} folder={vaultOf(record.vault_id)} onOpen={() => onOpen(record)}>
                <span className="text-xs text-faint">
                  {when(seen.length ? new Date(seen.find((one) => one.record.id === record.id)?.at ?? Date.now()).toISOString() : record.updated_at)}
                </span>
              </RecordLine>
            ))}
            {recent.length === 0 && <Empty>{t('Nenhum registro criado. Abra um cofre e crie o primeiro.')}</Empty>}
          </Panel>

          <Panel>
            <PanelHead title={t('Favoritos e atenção')}>
              <div className="flex rounded-lg border bg-raised p-0.5">
                {(
                  [
                    ['tudo', t('Tudo')],
                    ['favoritos', t('Favoritos')],
                    ['atencao', t('Atenção')],
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
                  ? t('Nenhum registro pede atenção. Senhas fracas, repetidas ou vencidas são exibidas aqui.')
                  : t('Nenhum favorito. Marque a estrela de um registro para exibi-lo aqui.')}
              </Empty>
            )}
          </Panel>
        </div>

        <Panel className="mt-5">
          <PanelHead title={t('Atividade e agentes')}>
            <TabMenu to={{ place: 'trail' }}>
              <Button variant="ghost" size="sm" {...linkProps({ place: 'trail' }, onTrail)}>
                {t('Visualizar a trilha')}
                <ArrowRight />
              </Button>
            </TabMenu>
          </PanelHead>
          <Line
            icon={Lock}
            tone="amber"
            title={
              unlockedAt
                ? t('Destrancado desde {time}', {
                    time: new Date(unlockedAt.endsWith('Z') ? unlockedAt : `${unlockedAt}Z`).toLocaleTimeString(locale(), {
                      hour: '2-digit',
                      minute: '2-digit',
                    }),
                  })
                : t('Destrancado')
            }
            detail={
              seconds === null
                ? t('sem tranca automática')
                : t('tranca em {time} sem uso', { time: seconds >= 60 ? `${Math.ceil(seconds / 60)} min` : `${seconds} s` })
            }
          >
            <Button variant="outline" size="sm" onClick={onLock}>
              {t('Trancar')}
            </Button>
          </Line>
          {agentEvents.map((event) => (
            <Line
              key={event.seq}
              icon={Bot}
              title={agentLine(
                event.actor.process ?? t('Agente'),
                event.result === 'denied' ? 'denied' : event.action === 'secret.copied' ? 'copied' : 'read',
                event.scope ? (records.find((record) => record.id === event.scope)?.title ?? t('um registro')) : null,
              )}
              detail={when(event.at)}
            />
          ))}
          <Line
            icon={ShieldCheck}
            title={
              grants.length === 0
                ? t('Nenhuma permissão ativa')
                : grants.length === 1
                  ? t('1 permissão ativa')
                  : t('{n} permissões ativas', { n: grants.length })
            }
            detail={(() => {
              const on = open.filter((vault) => vault.agent_access).length

              return on === 0
                ? t('agente bloqueado em todos os cofres')
                : on === 1
                  ? t('agente liberado em 1 cofre')
                  : t('agente liberado em {n} cofres', { n: on })
            })()}
          >
            <TabMenu to={{ place: 'settings', section: 'agentes' }}>
              <Button variant="ghost" size="sm" {...linkProps({ place: 'settings', section: 'agentes' }, onSettings)}>
                {t('Visualizar em Ajustes')}
              </Button>
            </TabMenu>
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
    const to = { place: 'vault' as const, vault: record.vault_id, record: record.id }

    return (
      <TabMenu to={to}>
      <button
        type="button"
        {...linkProps(to, onOpen)}
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
      </TabMenu>
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

/** "Claude copiou um segredo de Banco Inter": one sentence per action, with and without the record. */
function agentLine(who: string, did: 'denied' | 'copied' | 'read', record: string | null): string {
  if (did === 'denied') {
    return record ? t('{who} teve um pedido negado de {record}', { who, record }) : t('{who} teve um pedido negado', { who })
  }
  if (did === 'copied') {
    return record ? t('{who} copiou um segredo de {record}', { who, record }) : t('{who} copiou um segredo', { who })
  }

  return record ? t('{who} leu um segredo de {record}', { who, record }) : t('{who} leu um segredo', { who })
}

function Empty({ children }: { children: React.ReactNode }) {
  return <p className="px-5 py-6 text-sm text-faint">{children}</p>
}

/**
 * Keyhold opened on 127.0.0.1 because this system does not resolve keyhold.localhost (Safari's
 * resolver before macOS 26, a Linux without systemd-resolved). On 127.0.0.1 the browser sends the
 * session cookie to every local server, whatever its port. Dismissed on this machine, not in the
 * browser: the interface keeps nothing in web storage.
 */
function SetHostHint({ dismissed }: { dismissed: boolean }) {
  const [hidden, setHidden] = useState(dismissed)
  const command = 'sudo keyhold set-host keyhold.localhost'

  if (hidden || window.location.hostname !== '127.0.0.1') return null

  return (
    <section className="mt-6 rounded-[14px] border bg-card">
      <div className="flex items-start gap-3 px-5 py-4">
        <span className="grid size-8 shrink-0 place-items-center rounded-lg bg-amber-soft text-amber">
          <Globe className="size-4" />
        </span>
        <div className="min-w-0 flex-1">
          <b className="block text-sm font-medium">{t('O Keyhold abriu em 127.0.0.1')}</b>
          <p className="mt-0.5 text-[13px] text-muted-foreground">
            {t(
              'Este sistema não reconhece o endereço keyhold.localhost, então o Keyhold abriu pelo 127.0.0.1. Nesse endereço, o navegador também entrega o cookie da sua sessão a outros programas que rodam neste computador, como servidores de desenvolvimento e contêineres, que podem gravá-lo em log. Para usar o endereço próprio, rode uma vez no terminal e abra o Keyhold de novo:',
            )}
          </p>
          <div className="mt-3 flex max-w-[560px] items-center gap-2 rounded-[9px] border bg-field py-1 pr-1 pl-3">
            <code className="min-w-0 flex-1 truncate font-mono text-[13px]">{command}</code>
            <Button
              variant="ghost"
              size="icon"
              aria-label={t('Copiar comando')}
              onClick={() =>
                void navigator.clipboard
                  .writeText(command)
                  .then(() => toast(t('Comando copiado')))
                  .catch(() =>
                    toast(
                      t('Não foi possível copiar'),
                      t('O navegador bloqueou o acesso à área de transferência nesta página.'),
                      'error',
                    ),
                  )
              }
            >
              <Copy />
            </Button>
          </div>
        </div>
        <button
          type="button"
          aria-label={t('Dispensar')}
          title={t('Dispensar')}
          className="grid size-6 shrink-0 place-items-center rounded-md text-faint hover:bg-hover hover:text-foreground"
          onClick={() => {
            setHidden(true)
            void api.dismissSetHostHint().catch(() => undefined)
          }}
        >
          <X className="size-4" />
        </button>
      </div>
    </section>
  )
}

/** How many vaults Início shows. The rest are a click away, on Cofres. */
const CAROUSEL_SIZE = 10

/**
 * Início's vaults: the starred ones first, then the ones opened the most, in a row that scrolls
 * sideways. Nothing starred and nothing opened yet is said rather than shown empty.
 */
function VaultCarousel({
  vaults,
  records,
  issues,
  unreadable,
  onVault,
  onAll,
  onChanged,
}: {
  vaults: VaultSummary[]
  records: RecordPreview[]
  issues: Map<string, HealthIssue[]>
  unreadable: Set<string>
  onVault: (id: string) => void
  onAll: () => void
  onChanged: () => void
}) {
  const track = useRef<HTMLDivElement>(null)
  const [edges, setEdges] = useState({ start: true, end: true })
  const shown = [...vaults]
    .filter((vault) => vault.favorite || (vault.opened ?? 0) > 0)
    .sort(byUse)
    .slice(0, CAROUSEL_SIZE)

  const measure = () => {
    const el = track.current
    if (!el) return
    setEdges({ start: el.scrollLeft <= 1, end: el.scrollLeft + el.clientWidth >= el.scrollWidth - 1 })
  }

  useEffect(() => {
    measure()
    window.addEventListener('resize', measure)
    return () => window.removeEventListener('resize', measure)
  }, [shown.length])

  const page = (direction: 1 | -1) => {
    const el = track.current
    if (el) el.scrollBy({ left: direction * el.clientWidth * 0.9, behavior: 'smooth' })
  }

  if (shown.length === 0) {
    return (
      <section className="mt-6 flex items-center gap-4 rounded-[14px] border border-dashed px-5 py-4">
        <span className="grid size-9 shrink-0 place-items-center rounded-[10px] bg-stone-soft text-muted-foreground">
          <Vault className="size-[18px]" />
        </span>
        <span className="min-w-0 flex-1">
          <b className="block text-sm font-medium">{t('Seus cofres aparecem aqui')}</b>
          <span className="block text-[13px] text-muted-foreground">
            {t('Os favoritos primeiro, depois os que você mais abre. Favorite um cofre pela estrela, na aba Cofres.')}
          </span>
        </span>
        <Button variant="outline" size="sm" onClick={onAll}>
          {t('Abrir Cofres')}
          <ArrowRight />
        </Button>
      </section>
    )
  }

  return (
    <section className="mt-6">
      <div className="mb-3 flex items-center gap-3">
        <h2 className="text-[15px] font-semibold">{t('Seus cofres')}</h2>
        <span className="truncate text-[13px] text-faint">{t('favoritos primeiro, depois os mais acessados')}</span>
        <span className="flex-1" />
        <Button variant="outline" size="icon-sm" aria-label={t('Anteriores')} disabled={edges.start} onClick={() => page(-1)}>
          <ChevronLeft />
        </Button>
        <Button variant="outline" size="icon-sm" aria-label={t('Próximos')} disabled={edges.end} onClick={() => page(1)}>
          <ChevronRight />
        </Button>
        <button
          type="button"
          onClick={onAll}
          className="ml-1 inline-flex shrink-0 items-center gap-1 text-sm font-medium text-muted-foreground hover:text-foreground"
        >
          {t('Todos os cofres')}
          <ArrowRight className="size-4" />
        </button>
      </div>
      <div
        ref={track}
        onScroll={measure}
        className="flex snap-x snap-mandatory gap-5 overflow-x-auto pb-2"
      >
        {shown.map((vault) => (
          <VaultCard
            key={vault.id}
            className="w-[280px] shrink-0 snap-start"
            vault={vault}
            records={records}
            issues={issues}
            unavailable={unreadable.has(vault.id)}
            onOpen={() => onVault(vault.id)}
            onChanged={onChanged}
          />
        ))}
      </div>
    </section>
  )
}
