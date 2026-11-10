import { useState } from 'react'
import {
  Check,
  ChevronDown,
  ChevronRight,
  Cpu,
  Home,
  Lock,
  Plus,
  ScrollText,
  Search,
  Settings2,
  SlidersHorizontal,
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { cn } from '@/lib/utils'

import { api, type Status, type VaultSummary } from '../api'
import { colourOf } from './colours'

/**
 * The shell every screen shares: a narrow rail with what belongs to Keyhold itself, and a top bar
 * with where you are, the search and the page's own actions.
 *
 * What belongs to a vault (its records, favourites, attention, trash) lives inside the vault page,
 * not on the rail; switching vaults is the first step of the path in the top bar.
 */
export type Place = 'home' | 'vault' | 'trail' | 'instances' | 'settings'

const RAIL: Array<{ id: Exclude<Place, 'vault'>; label: string; icon: LucideIcon }> = [
  { id: 'home', label: 'Início', icon: Home },
  { id: 'trail', label: 'Trilha', icon: ScrollText },
  { id: 'instances', label: 'Instâncias', icon: Cpu },
  { id: 'settings', label: 'Ajustes', icon: SlidersHorizontal },
]

export function Rail({
  place,
  status,
  onGo,
  onLock,
  onChanged,
}: {
  place: Place
  status: Status
  onGo: (place: Exclude<Place, 'vault'>) => void
  onLock: () => void
  onChanged: () => void
}) {
  return (
    <nav className="flex min-h-0 flex-col items-center gap-1 border-r bg-sidebar px-1.5 pt-3 pb-2.5">
      <span className="mb-3.5 grid size-9 place-items-center rounded-[10px] bg-amber-soft text-amber">
        <Lock className="size-[18px]" />
      </span>

      {RAIL.map(({ id, label, icon: Icon }) => (
        <button
          key={id}
          type="button"
          aria-current={place === id}
          onClick={() => onGo(id)}
          className={cn(
            'relative flex w-16 flex-col items-center gap-1 rounded-[10px] pt-2 pb-1.5 text-[11px] font-medium text-faint hover:bg-hover hover:text-foreground',
            place === id &&
              'bg-card text-foreground ring-1 ring-border before:absolute before:top-3 before:bottom-3 before:-left-1.5 before:w-[3px] before:rounded-r before:bg-primary',
          )}
        >
          <Icon className="size-[18px]" />
          {label}
        </button>
      ))}

      <span className="flex-1" />

      <LockButton status={status} onLock={onLock} onChanged={onChanged} onSettings={() => onGo('settings')} />
    </nav>
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
            ? 'O cofre só tranca quando você mandar ou fechar o Keyhold.'
            : 'Sem nenhum clique ou tecla no Keyhold. O relógio corre no programa, não nesta janela.'}
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

export function Crumb({ children, last, onClick }: { children: React.ReactNode; last?: boolean; onClick?: () => void }) {
  return (
    <>
      <ChevronRight className="size-3.5 shrink-0" />
      {last ? (
        <b className="truncate text-[15px] font-semibold text-foreground">{children}</b>
      ) : (
        <button type="button" onClick={onClick} className="truncate rounded px-0.5 hover:text-foreground">
          {children}
        </button>
      )}
    </>
  )
}

export function Title({ children }: { children: React.ReactNode }) {
  return <b className="truncate text-[15px] font-semibold text-foreground">{children}</b>
}

export function VaultInitial({ vault, size = 'sm' }: { vault: Pick<VaultSummary, 'name' | 'colour'>; size?: 'sm' | 'md' | 'lg' }) {
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
}: {
  vaults: VaultSummary[]
  current: VaultSummary
  onPick: (id: string) => void
  onNew: () => void
  onManage: () => void
}) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const shown = vaults.filter(
    (vault) => !vault.archived && vault.name.toLowerCase().includes(query.trim().toLowerCase()),
  )

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="-ml-1.5 flex h-8 shrink-0 items-center gap-2 rounded-lg pr-2 pl-1.5 text-sm font-medium text-foreground hover:bg-hover"
        >
          <VaultInitial vault={current} />
          {current.name}
          <ChevronDown className="size-3.5 text-faint" />
        </button>
      </PopoverTrigger>
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
            <button
              key={vault.id}
              type="button"
              onClick={() => {
                setOpen(false)
                setQuery('')
                onPick(vault.id)
              }}
              className="flex h-9 w-full items-center gap-2.5 rounded-lg px-2 text-left text-sm hover:bg-hover"
            >
              <VaultInitial vault={vault} />
              <span className="flex-1 truncate">{vault.name}</span>
              {vault.id === current.id && <Check className="size-4 text-primary" />}
            </button>
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
      </PopoverContent>
    </Popover>
  )
}

/** A page's body: the same padding everywhere, scrolling inside. */
export function Body({ children, className }: { children: React.ReactNode; className?: string }) {
  return <div className={cn('min-h-0 flex-1 overflow-auto p-6', className)}>{children}</div>
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
