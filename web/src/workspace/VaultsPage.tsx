import { useMemo, useState } from 'react'
import { ArrowUpDown, HardDrive, Plus, Search, Star } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { cn } from '@/lib/utils'

import type { RecordPreview, VaultSummary } from '../api'
import { t } from '../i18n'
import { AwsMark } from './AwsMark'
import { Body, Title, TopBar } from './shell'
import { byUse, useHealthIssues, VaultCard } from './VaultCard'

type Filter = 'todos' | 'favoritos' | 'locais' | 'aws'
type Order = 'uso' | 'nome'

/**
 * Cofres: every vault, to find one, star it or start a new one. Início only shows the few that
 * are used the most; this is where the rest are.
 */
export function VaultsPage({
  vaults,
  records,
  unreadable,
  onVault,
  onNewVault,
  onChanged,
  onSearch,
}: {
  vaults: VaultSummary[]
  records: RecordPreview[]
  unreadable: Set<string>
  onVault: (id: string) => void
  onNewVault: () => void
  onChanged: () => void
  onSearch: () => void
}) {
  const [query, setQuery] = useState('')
  const [filter, setFilter] = useState<Filter>('todos')
  const [order, setOrder] = useState<Order>('uso')
  const open = vaults.filter((vault) => !vault.archived)
  const issues = useHealthIssues(vaults)

  const counts = {
    todos: open.length,
    favoritos: open.filter((vault) => vault.favorite).length,
    locais: open.filter((vault) => vault.kind !== 'aws').length,
    aws: open.filter((vault) => vault.kind === 'aws').length,
  }

  const shown = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase()

    return open
      .filter((vault) => !needle || vault.name.toLocaleLowerCase().includes(needle))
      .filter(
        (vault) =>
          filter === 'todos' ||
          (filter === 'favoritos' && vault.favorite) ||
          (filter === 'locais' && vault.kind !== 'aws') ||
          (filter === 'aws' && vault.kind === 'aws'),
      )
      .sort(order === 'nome' ? (a, b) => a.name.localeCompare(b.name) : byUse)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [vaults, query, filter, order])

  const chips: Array<[Filter, string, React.ReactNode]> = [
    ['todos', t('Todos'), null],
    ['favoritos', t('Favoritos'), <Star key="s" className="size-3" />],
    ['locais', t('Locais'), <HardDrive key="h" className="size-3" />],
  ]
  if (counts.aws > 0) chips.push(['aws', t('AWS'), <AwsMark key="a" className="size-3" />])

  const totalRecords = records.length

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
        <Title>{t('Cofres')}</Title>
      </TopBar>

      <Body>
        <h1 className="text-[22px] font-semibold tracking-tight">{t('Cofres')}</h1>
        <p className="mt-1 text-muted-foreground">
          {[
            open.length === 1 ? t('1 cofre') : t('{n} cofres', { n: open.length }),
            counts.favoritos === 1 ? t('1 favorito') : t('{n} favoritos', { n: counts.favoritos }),
            totalRecords === 1 ? t('1 registro') : t('{n} registros', { n: totalRecords }),
          ].join(', ')}
          .
        </p>

        <div className="mt-6 flex flex-wrap items-center gap-3">
          <div className="relative w-[300px] max-w-full">
            <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-faint" />
            <Input
              aria-label={t('Filtrar cofres')}
              placeholder={t('Filtrar cofres')}
              className="pl-9"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
          </div>
          <div className="flex flex-wrap gap-1.5">
            {chips.map(([id, label, icon]) => (
              <button
                key={id}
                type="button"
                aria-pressed={filter === id}
                onClick={() => setFilter(id)}
                className={cn(
                  'inline-flex h-[26px] items-center gap-1.5 rounded-full border px-2.5 text-xs font-medium',
                  filter === id ? 'border-transparent bg-primary-soft text-primary-text' : 'text-muted-foreground hover:bg-hover',
                )}
              >
                {icon}
                {label}
                <span className="opacity-70">{counts[id]}</span>
              </button>
            ))}
          </div>
          <span className="flex-1" />
          <Select
            value={order}
            onValueChange={(value) => setOrder(value as Order)}
          >
            <SelectTrigger size="sm" className="w-[240px]" aria-label={t('Ordem')}>
              <ArrowUpDown className="text-faint" />
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="uso">{t('Mais acessados primeiro')}</SelectItem>
              <SelectItem value="nome">{t('Por nome')}</SelectItem>
            </SelectContent>
          </Select>
        </div>

        <div className="mt-5 grid grid-cols-[repeat(auto-fill,minmax(260px,1fr))] gap-5">
          {shown.map((vault) => (
            <VaultCard
              key={vault.id}
              vault={vault}
              records={records}
              issues={issues}
              unavailable={unreadable.has(vault.id)}
              onOpen={() => onVault(vault.id)}
              onChanged={onChanged}
            />
          ))}
          {filter === 'todos' && !query && (
            <button
              type="button"
              onClick={onNewVault}
              className="flex h-[136px] flex-col items-center justify-center gap-2 rounded-[14px] border border-dashed text-sm text-muted-foreground hover:bg-hover"
            >
              <Plus className="size-5" />
              {t('Novo cofre')}
            </button>
          )}
        </div>

        {shown.length === 0 && (filter !== 'todos' || query) && (
          <p className="mt-6 text-sm text-muted-foreground">
            {filter === 'favoritos' && !query
              ? t('Nenhum cofre favorito. Use a estrela de um cartão para favoritar.')
              : t('Nenhum cofre encontrado.')}
          </p>
        )}
      </Body>
    </div>
  )
}
