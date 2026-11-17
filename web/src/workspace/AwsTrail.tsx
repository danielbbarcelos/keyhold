import { useEffect, useState } from 'react'
import { CalendarRange, ChevronDown, Laptop, ListFilter, Lock, User } from 'lucide-react'

import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { locale, t } from '@/i18n'

import { api, ApiError, type VaultSummary, type VaultTrail } from '../api'
import { filterable, kindOf, momentOf } from './activity'
import { AwsMark } from './AwsMark'

/**
 * Which trail the Trilha shows: this computer's, or one AWS vault's (from its account). Picked from
 * the path in the top bar, the way a vault is switched.
 */
export function TrailSource({
  vaults,
  source,
  onPick,
}: {
  vaults: VaultSummary[]
  /** `null` is this computer. */
  source: string | null
  onPick: (vaultId: string | null) => void
}) {
  const aws = vaults.filter((vault) => vault.kind === 'aws' && !vault.archived)
  if (aws.length === 0) return null

  const current = aws.find((vault) => vault.id === source)

  return (
    <>
      <ChevronDown className="size-3.5 -rotate-90 text-faint" />
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            className="inline-flex h-8 items-center gap-2 rounded-lg px-2 text-[15px] font-semibold text-foreground hover:bg-stone-soft"
          >
            {current ? <AwsMark size="xs" className="size-4" /> : <Laptop className="size-4 text-faint" />}
            {current ? current.name : t('Este computador')}
            <ChevronDown className="size-3.5 text-faint" />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-64">
          <DropdownMenuItem onSelect={() => onPick(null)}>
            <Laptop />
            {t('Este computador')}
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuLabel className="text-xs text-faint">{t('Cofres AWS')}</DropdownMenuLabel>
          {aws.map((vault) => (
            <DropdownMenuItem key={vault.id} onSelect={() => onPick(vault.id)}>
              <AwsMark size="xs" className="size-4" />
              {vault.name}
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
    </>
  )
}

const periods = (): Array<[number, string]> => [
  [1, t('Último dia')],
  [7, t('Últimos {n} dias', { n: 7 })],
  [30, t('Últimos {n} dias', { n: 30 })],
  [90, t('Últimos {n} dias', { n: 90 })],
]

/** One AWS vault's trail, read from its account's log groups. */
export function AwsTrail({ vault, onOpenVault }: { vault: VaultSummary; onOpenVault: () => void }) {
  const [person, setPerson] = useState('')
  const [action, setAction] = useState('')
  const [days, setDays] = useState(7)
  const [trail, setTrail] = useState<VaultTrail | null>(null)
  const [people, setPeople] = useState<string[]>([])
  const [error, setError] = useState('')

  useEffect(() => {
    let alive = true
    setTrail(null)
    setError('')

    api
      .vaultTrail(vault.id, { person, action, days })
      .then((found) => {
        if (!alive) return
        setTrail(found)
        // The people menu keeps everyone seen, not only who the current filter left.
        setPeople((known) => [...new Set([...known, ...found.people])].sort())
      })
      .catch((cause) => alive && setError(cause instanceof ApiError ? cause.message : t('Não foi possível ler a trilha.')))

    return () => {
      alive = false
    }
  }, [vault.id, person, action, days])

  const actions = filterable()
  const allDenied = trail !== null && trail.groups.length > 0 && trail.denied.length === trail.groups.length

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden rounded-[14px] border bg-card">
      <div className="flex flex-wrap items-center gap-2 border-b px-5 py-3.5">
        <Filter
          icon={User}
          label={person || t('Todas as pessoas')}
          options={[['', t('Todas as pessoas')], ...people.map((one): [string, string] => [one, one])]}
          onPick={setPerson}
        />
        <Filter
          icon={ListFilter}
          label={actions.find(([id]) => id === action)?.[1] ?? t('Todas as ações')}
          options={[['', t('Todas as ações')], ...actions]}
          onPick={setAction}
        />
        <Filter
          icon={CalendarRange}
          label={periods().find(([n]) => n === days)?.[1] ?? t('Últimos {n} dias', { n: days })}
          options={periods().map(([n, label]): [string, string] => [String(n), label])}
          onPick={(value) => setDays(Number(value))}
        />
        <span className="flex-1" />
        {trail && <span className="font-mono text-[11px] text-faint">{trail.groups.join(', ')}</span>}
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        {error ? (
          <p className="p-6 text-sm text-rose">{error}</p>
        ) : !trail ? (
          <p className="p-6 text-sm text-faint">{t('Lendo a trilha na AWS…')}</p>
        ) : allDenied ? (
          <div className="mx-auto flex max-w-md flex-col items-center gap-3 px-5 py-16 text-center">
            <span className="grid size-10 place-items-center rounded-xl bg-stone-soft text-muted-foreground">
              <Lock className="size-5" />
            </span>
            <b className="text-[15px] font-semibold">{t('Sua permissão não inclui ler a trilha inteira deste cofre')}</b>
            <p className="text-sm text-muted-foreground">
              {t('Quem administra a conta decide quem lê a trilha do cofre todo (a política audit-read). O que acontece em cada registro continua na aba Atividade dele.')}
            </p>
            <Button variant="outline" size="sm" onClick={onOpenVault}>
              {t('Abrir {name}', { name: vault.name })}
            </Button>
          </div>
        ) : trail.events.length === 0 ? (
          <p className="px-5 py-10 text-center text-sm text-faint">{t('Nenhum evento neste período.')}</p>
        ) : (
          <table className="w-full text-sm">
            <thead className="sticky top-0 bg-card">
              <tr className="border-b text-left text-xs text-faint">
                <th className="px-5 py-2.5 font-normal">{t('Hora')}</th>
                <th className="px-3 py-2.5 font-normal">{t('Evento')}</th>
                <th className="px-3 py-2.5 font-normal">{t('Quem')}</th>
                <th className="px-3 py-2.5 font-normal">{t('Onde')}</th>
              </tr>
            </thead>
            <tbody>
              {trail.events.map((event, index) => {
                const at = momentOf(event)
                const today = at?.toDateString() === new Date().toDateString()
                const detail = event.detail
                const where = [
                  event.title ?? event.parameter ?? '',
                  typeof detail.field === 'string' ? detail.field : '',
                  typeof detail.attachment === 'string' ? detail.attachment : '',
                  typeof detail.from === 'string' && typeof detail.to === 'string' ? t('de {from} para {to}', { from: detail.from, to: detail.to }) : '',
                ]
                  .filter(Boolean)
                  .join(', ')

                return (
                  <tr key={`${event.at_ms}-${index}`} className="border-b last:border-b-0">
                    <td className="px-5 py-3 font-mono text-[13px] whitespace-nowrap text-muted-foreground">
                      {at ? (today ? at.toLocaleTimeString(locale()) : at.toLocaleString(locale(), { dateStyle: 'short', timeStyle: 'short' })) : ''}
                    </td>
                    <td className="px-3 py-3">
                      <span className="inline-flex items-center gap-1.5 rounded-full bg-stone-soft px-2.5 py-0.5 text-xs font-medium text-muted-foreground">
                        {kindOf(event.action).label}
                      </span>
                    </td>
                    <td className="px-3 py-3">
                      <span className="inline-flex items-center gap-1.5">
                        <User className="size-3.5 text-faint" />
                        {event.person}
                      </span>
                    </td>
                    <td className="px-3 py-3 text-muted-foreground">{where}</td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        )}
      </div>

      <p className="border-t px-5 py-3 text-[13px] text-faint">
        {t('A hora é a de quando a AWS recebeu o evento, e o nome é o usuário no Identity Center. O que foi feito fora do Keyhold, pelo console da AWS ou pela CLI, fica no AWS CloudTrail da conta.')}
      </p>
    </div>
  )
}

function Filter({
  icon: Icon,
  label,
  options,
  onPick,
}: {
  icon: React.ComponentType<{ className?: string }>
  label: string
  options: Array<[string, string]>
  onPick: (value: string) => void
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="sm" className="font-normal">
          <Icon />
          {label}
          <ChevronDown className="text-faint" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start">
        {options.map(([value, text]) => (
          <DropdownMenuItem key={value || 'all'} onSelect={() => onPick(value)}>
            {text}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
