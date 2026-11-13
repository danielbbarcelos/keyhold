import { useCallback, useEffect, useState } from 'react'
import {
  Check,
  ChevronDown,
  ChevronRight,
  Copy,
  Cpu,
  Home,
  Lock,
  Plus,
  ScrollText,
  Search,
  Settings2,
  Paintbrush,
  SlidersHorizontal,
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { cn } from '@/lib/utils'

import { api, type AwsLogin, type RecordIcon, type Status, type VaultSummary } from '../api'
import { linkProps, TabMenu, type Destination } from './nav'
import { AWS_CHANGED, useAwsRenew } from './AwsAccounts'
import { AwsMark } from './AwsMark'
import { colourOf } from './colours'
import { RecordTile } from './icons'
import { LogoIcon } from './Logo'

/**
 * The shell every screen shares: a narrow rail with what belongs to Keyhold itself, and a top bar
 * with where you are, the search and the page's own actions.
 *
 * What belongs to a vault (its records, favourites, attention, trash) lives inside the vault page,
 * not on the rail; switching vaults is the first step of the path in the top bar.
 */
export type Place = 'home' | 'vault' | 'trail' | 'instances' | 'custom' | 'settings'

const RAIL: Array<{ id: Exclude<Place, 'vault'>; label: string; icon: LucideIcon }> = [
  { id: 'home', label: 'Início', icon: Home },
  { id: 'trail', label: 'Trilha', icon: ScrollText },
  { id: 'instances', label: 'Instâncias', icon: Cpu },
  { id: 'custom', label: 'Personalizar', icon: Paintbrush },
  { id: 'settings', label: 'Ajustes', icon: SlidersHorizontal },
]

export function Rail({
  place,
  status,
  onGo,
  onSettings,
  onLock,
  onChanged,
}: {
  place: Place
  status: Status
  onGo: (place: Exclude<Place, 'vault'>) => void
  /** Ajustes, opened on one section ("aws", "tranca"). */
  onSettings: (section: string) => void
  onLock: () => void
  onChanged: () => void
}) {
  return (
    <nav className="flex min-h-0 flex-col items-center gap-1.5 border-r bg-sidebar px-2 pt-4 pb-3">
      <span className="mb-4 grid size-9 place-items-center">
        <LogoIcon className="size-9" />
      </span>

      {RAIL.map(({ id, label, icon: Icon }) => (
        <TabMenu key={id} to={{ place: id }}>
        <button
          type="button"
          aria-current={place === id}
          {...linkProps({ place: id }, () => onGo(id))}
          className={cn(
            'relative flex w-[72px] flex-col items-center gap-1 rounded-[10px] pt-2.5 pb-2 text-[11px] font-medium text-faint hover:bg-hover hover:text-foreground',
            place === id &&
              'bg-card text-foreground ring-1 ring-border before:absolute before:top-3 before:bottom-3 before:-left-2 before:w-[3px] before:rounded-r before:bg-primary',
          )}
        >
          <Icon className="size-[18px]" />
          {label}
        </button>
        </TabMenu>
      ))}

      <span className="flex-1" />

      <AwsSession onAccounts={() => onSettings('aws')} />
      <LockButton status={status} onLock={onLock} onChanged={onChanged} onSettings={() => onSettings('tranca')} />
    </nav>
  )
}

/** "2 h", "45 min", "30 s": how long is left, in the rail's few characters. */
function remaining(seconds: number): string {
  if (seconds <= 0) return 'expirou'
  if (seconds >= 3600) return `${Math.floor(seconds / 3600)} h`
  if (seconds >= 60) return `${Math.ceil(seconds / 60)} min`

  return `${Math.ceil(seconds)} s`
}

function clock(at: number): string {
  return new Date(at * 1000).toLocaleTimeString('pt-BR', { hour: '2-digit', minute: '2-digit' })
}

/**
 * How long the AWS session lasts, under the lock's own countdown. Shown only while some SSO login
 * has a session with an end: the soonest one is the number, the popover has each of them.
 *
 * Asking `/api/aws` does not count as activity, so this polling never keeps the vault from locking.
 */
function AwsSession({ onAccounts }: { onAccounts: () => void }) {
  const [logins, setLogins] = useState<AwsLogin[]>([])
  const [now, setNow] = useState(() => Date.now() / 1000)
  const [open, setOpen] = useState(false)

  const load = useCallback(
    () =>
      api
        .aws()
        .then((found) => setLogins(found.logins))
        .catch(() => setLogins([])),
    [],
  )
  const { renew, busy, dialog } = useAwsRenew(() => void load())

  // Read once, and again whenever Contas AWS changes something. Reads are not activity, so this
  // never keeps the vault from locking itself.
  useEffect(() => {
    void load()
    const again = () => void load()
    window.addEventListener(AWS_CHANGED, again)
    return () => window.removeEventListener(AWS_CHANGED, again)
  }, [load])

  const timed = logins
    .filter((login): login is AwsLogin & { expires_at: number } => login.expires_at !== null)
    .sort((a, b) => a.expires_at - b.expires_at)
  const any = timed.length > 0

  // The clock ticks, and the sessions are read again every 30 s, only while there is one to show.
  useEffect(() => {
    if (!any) return
    const poll = window.setInterval(() => void load(), 30_000)
    const tick = window.setInterval(() => setNow(Date.now() / 1000), 1000)

    return () => {
      window.clearInterval(poll)
      window.clearInterval(tick)
    }
  }, [any, load])

  if (!any) return dialog || null

  const left = timed[0].expires_at - now
  const urgent = left <= 600
  // A session that renews itself is only a countdown once renewing stopped working (the last ten
  // minutes, which Keyhold would otherwise have spent renewing it).
  const quiet = !urgent && timed.every((login) => login.renews)
  const shown = quiet ? 'ativa' : remaining(left)

  return (
    <>
    {dialog}
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={`Sessão AWS: ${shown}`}
          className={cn(
            'mb-1 flex w-16 flex-col items-center gap-1 rounded-[10px] py-2 text-[11px] text-muted-foreground hover:bg-hover',
            urgent && 'text-rose',
          )}
        >
          <AwsMark
            className={cn('size-[26px]', urgent && 'ring-2 ring-rose ring-offset-2 ring-offset-sidebar')}
          />
          <span className="tabular-nums">{shown}</span>
        </button>
      </PopoverTrigger>

      <PopoverContent side="right" align="end" sideOffset={10} className="w-[300px] p-0">
        <div className="flex items-center gap-2.5 border-b px-4 py-3">
          <AwsMark size="xs" />
          <b className="text-sm font-semibold">Sessões AWS</b>
        </div>
        <div className="divide-y">
          {timed.map((login) => {
            const gone = login.expires_at <= now

            return (
              <div key={login.id} className="flex items-center gap-3 px-4 py-3">
                <span className="min-w-0 flex-1">
                  <b className="block truncate text-sm font-medium">{login.label || login.start_url}</b>
                  <span
                    className={cn(
                      'block text-xs',
                      gone ? 'text-rose' : login.expires_at - now <= 600 ? 'text-rose' : 'text-faint',
                    )}
                  >
                    {gone
                      ? `expirou às ${clock(login.expires_at)}`
                      : login.renews && login.expires_at - now > 600
                        ? 'renovada automaticamente enquanto o Keyhold estiver aberto'
                        : `expira às ${clock(login.expires_at)}`}
                  </span>
                </span>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={busy}
                  aria-label={`Renovar ${login.label || login.start_url}`}
                  onClick={() => {
                    setOpen(false)
                    void renew(login)
                  }}
                >
                  Renovar
                </Button>
              </div>
            )
          })}
        </div>
        <div className="border-t p-2">
          <button
            type="button"
            onClick={() => {
              setOpen(false)
              onAccounts()
            }}
            className="w-full rounded-md px-2 py-1.5 text-xs text-muted-foreground hover:bg-hover hover:text-foreground"
          >
            Abrir Contas AWS
          </button>
        </div>
      </PopoverContent>
    </Popover>
    </>
  )
}

function LockButton({
  status,
  onLock,
  onChanged,
  onSettings,
}: {
  status: Status
  onLock: () => void
  onChanged: () => void
  onSettings: () => void
}) {
  const seconds = status.lock.locks_in
  const total = status.lock.policy.idle_seconds ?? seconds ?? 1
  const share = seconds === null ? 1 : Math.max(0, Math.min(1, seconds / Math.max(total, 1)))
  const label = seconds === null ? 'sem tranca' : seconds >= 60 ? `${Math.ceil(seconds / 60)} min` : `${seconds} s`
  const urgent = seconds !== null && seconds <= 30

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={`Tranca: ${label}`}
          className="flex w-16 flex-col items-center gap-1 rounded-[10px] py-2 text-[11px] text-muted-foreground hover:bg-hover"
        >
          <span
            className="grid size-[30px] place-items-center rounded-full"
            style={{
              background: `conic-gradient(var(${urgent ? '--rose' : '--primary'}) 0 ${share * 100}%, var(--border) ${share * 100}% 100%)`,
            }}
          >
            <span className="grid size-6 place-items-center rounded-full bg-sidebar">
              <Lock className="size-[13px]" />
            </span>
          </span>
          <span className="tabular-nums">{label}</span>
        </button>
      </PopoverTrigger>

      <PopoverContent side="right" align="end" sideOffset={10} className="w-[264px] p-4">
        <b className="block text-sm font-semibold">
          {seconds === null ? 'Sem tranca automática' : `Tranca em ${label}`}
        </b>
        <p className="mt-1 text-xs text-muted-foreground">
          {seconds === null
            ? 'O cofre só é trancado manualmente ou ao fechar o Keyhold.'
            : 'Sem nenhum clique ou tecla no Keyhold. O prazo é contado pelo Keyhold, não por esta janela.'}
        </p>
        <Button className="mt-3.5 w-full" onClick={onLock}>
          <Lock />
          Trancar agora
        </Button>
        <span className="mt-2 flex justify-center gap-1 text-xs">
          {seconds !== null && (
            <button
              type="button"
              className="rounded-md px-2 py-1 text-muted-foreground hover:bg-hover hover:text-foreground"
              onClick={() => void api.heartbeat().then(onChanged)}
            >
              Adiar
            </button>
          )}
          <button
            type="button"
            className="rounded-md px-2 py-1 text-muted-foreground hover:bg-hover hover:text-foreground"
            onClick={onSettings}
          >
            Ajustar a tranca
          </button>
        </span>
      </PopoverContent>
    </Popover>
  )
}

/** The bar across the top of the working area: path on the left, search and actions on the right. */
export function TopBar({
  children,
  actions,
  onSearch,
}: {
  children: React.ReactNode
  actions?: React.ReactNode
  onSearch?: () => void
}) {
  return (
    <header className="flex h-[60px] shrink-0 items-center gap-3 border-b px-6">
      <div className="flex min-w-0 flex-1 items-center gap-1.5 text-[13px] text-faint">{children}</div>
      {onSearch && (
        <button
          type="button"
          onClick={onSearch}
          className="flex h-9 w-[340px] max-w-[30vw] items-center gap-2.5 rounded-[9px] border bg-field px-3 text-sm text-faint hover:border-line-2"
        >
          <Search className="size-4" />
          <span className="flex-1 truncate text-left">Buscar em todos os registros</span>
          <kbd className="rounded-md border px-1.5 py-px font-sans text-[11px]">Ctrl P</kbd>
        </button>
      )}
      {actions}
    </header>
  )
}

export function Crumb({
  children,
  last,
  onClick,
  to,
}: {
  children: React.ReactNode
  last?: boolean
  onClick?: () => void
  /** Where the crumb leads, for "Abrir em nova guia" and Ctrl click. */
  to?: Destination
}) {
  const shown = last ? (
    <b className="truncate text-[15px] font-semibold text-foreground">{children}</b>
  ) : (
    <button
      type="button"
      {...(to ? linkProps(to, () => onClick?.()) : { onClick })}
      className="truncate rounded px-0.5 hover:text-foreground"
    >
      {children}
    </button>
  )

  return (
    <>
      <ChevronRight className="size-3.5 shrink-0" />
      {to ? <TabMenu to={to}>{shown}</TabMenu> : shown}
    </>
  )
}

export function Title({ children }: { children: React.ReactNode }) {
  return <b className="truncate text-[15px] font-semibold text-foreground">{children}</b>
}

export function VaultInitial({
  vault,
  size = 'sm',
}: {
  vault: Pick<VaultSummary, 'name' | 'colour'> & { icon?: RecordIcon | null }
  size?: 'sm' | 'md' | 'lg'
}) {
  if (vault.icon && (vault.icon.image || vault.icon.name)) {
    return (
      <RecordTile
        icon={vault.icon}
        size={size === 'sm' ? 'xs' : size === 'lg' ? 'md' : 'sm'}
        className={size === 'md' ? 'size-7 rounded-lg [&_svg]:size-[15px]' : undefined}
      />
    )
  }

  const letters = vault.name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((word) => word[0]?.toUpperCase())
    .join('')

  return (
    <span
      className={cn(
        'grid shrink-0 place-items-center font-semibold',
        colourOf(vault.colour).tile,
        size === 'sm' && 'size-[22px] rounded-md text-[11px]',
        size === 'md' && 'size-7 rounded-lg text-xs',
        size === 'lg' && 'size-9 rounded-[10px] text-[13px]',
      )}
    >
      {letters || '?'}
    </span>
  )
}

/** The first step of the path: which vault, and the way to another. */
export function VaultSwitcher({
  vaults,
  current,
  onPick,
  onNew,
  onManage,
  onCopy,
}: {
  vaults: VaultSummary[]
  current: VaultSummary
  onPick: (id: string) => void
  onNew: () => void
  onManage: () => void
  /** "Copiar cofre para…", for the vault being shown. */
  onCopy?: () => void
}) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const shown = vaults.filter(
    (vault) => !vault.archived && vault.name.toLowerCase().includes(query.trim().toLowerCase()),
  )

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <TabMenu to={{ place: 'vault', vault: current.id }}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="-ml-1.5 flex h-8 shrink-0 items-center gap-2 rounded-lg pr-2 pl-1.5 text-sm font-medium text-foreground hover:bg-hover"
        >
          <VaultInitial vault={current} />
          {current.name}
          {current.kind === 'aws' && <AwsMark className="size-4" title="Cofre AWS" />}
          <ChevronDown className="size-3.5 text-faint" />
        </button>
      </PopoverTrigger>
      </TabMenu>
      <PopoverContent className="w-[280px] p-1.5">
        <Input
          autoFocus
          className="mb-1 h-8"
          placeholder="Buscar cofre"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
        <div className="max-h-72 overflow-y-auto">
          {shown.map((vault) => (
            <TabMenu key={vault.id} to={{ place: 'vault', vault: vault.id }}>
            <button
              type="button"
              {...linkProps({ place: 'vault', vault: vault.id }, () => {
                setOpen(false)
                setQuery('')
                onPick(vault.id)
              })}
              className="flex h-9 w-full items-center gap-2.5 rounded-lg px-2 text-left text-sm hover:bg-hover"
            >
              <VaultInitial vault={vault} />
              {/* The name gives way first; the check stays beside it and the AWS mark at the end,
                  so the marks line up in one column whatever is picked. */}
              <span className="flex min-w-0 flex-1 items-center gap-1.5">
                <span className="truncate">{vault.name}</span>
                {vault.id === current.id && <Check className="size-4 shrink-0 text-primary" />}
              </span>
              {vault.kind === 'aws' && <AwsMark className="size-4 shrink-0" title="Cofre AWS" />}
            </button>
            </TabMenu>
          ))}
          {shown.length === 0 && <p className="px-2 py-3 text-xs text-faint">Nenhum cofre com esse nome.</p>}
        </div>
        <div className="my-1 h-px bg-border" />
        <button
          type="button"
          onClick={() => {
            setOpen(false)
            onNew()
          }}
          className="flex h-9 w-full items-center gap-2.5 rounded-lg px-2 text-sm hover:bg-hover"
        >
          <Plus className="size-4 text-muted-foreground" />
          Novo cofre
        </button>
        <button
          type="button"
          onClick={() => {
            setOpen(false)
            onManage()
          }}
          className="flex h-9 w-full items-center gap-2.5 rounded-lg px-2 text-sm hover:bg-hover"
        >
          <Settings2 className="size-4 text-muted-foreground" />
          Gerenciar cofres
        </button>
        {onCopy && (
          <button
            type="button"
            onClick={() => {
              setOpen(false)
              onCopy()
            }}
            className="flex h-9 w-full items-center gap-2.5 rounded-lg px-2 text-sm hover:bg-hover"
          >
            <Copy className="size-4 text-muted-foreground" />
            Copiar este cofre
          </button>
        )}
      </PopoverContent>
    </Popover>
  )
}

/** A page's body: the same padding everywhere, scrolling inside. */
export function Body({ children, className }: { children: React.ReactNode; className?: string }) {
  return <div className={cn('kh-wide min-h-0 flex-1 overflow-auto p-6', className)}>{children}</div>
}

/** A card: the one container every screen is made of. */
export function Panel({ children, className }: { children: React.ReactNode; className?: string }) {
  return <section className={cn('rounded-[14px] border bg-card', className)}>{children}</section>
}

export function PanelHead({
  title,
  hint,
  children,
}: {
  title: React.ReactNode
  hint?: React.ReactNode
  children?: React.ReactNode
}) {
  return (
    <div className="flex items-center gap-3 border-b px-5 py-4">
      <div className="min-w-0 flex-1">
        <h3 className="text-[15px] font-semibold">{title}</h3>
        {hint && <p className="mt-0.5 text-[13px] text-muted-foreground">{hint}</p>}
      </div>
      {children}
    </div>
  )
}
