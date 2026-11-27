import { useEffect, useState } from 'react'
import { HardDrive, Star } from 'lucide-react'

import { cn } from '@/lib/utils'

import { api, type HealthIssue, type RecordPreview, type VaultSummary } from '../api'
import { t } from '../i18n'
import { AwsMark } from './AwsMark'
import { linkProps, TabMenu } from './nav'
import { VaultInitial } from './shell'
import { toastError } from './toast'
import { when } from './vault/paths'

/**
 * One vault as a card: on Início's carousel and on the Cofres page, always the same shape.
 *
 * Four lines in fixed places, so a row of cards lines up whatever is in them: the icon and the
 * name at the top (one line, truncated), what kind of vault it is under the name (one line, no
 * region), the counts, and the state at the foot (the agent, or why the vault cannot be read)
 * with when it was last opened. A long name or a missing session never moves the lines below it.
 */
export function VaultCard({
  vault,
  records,
  issues,
  unavailable,
  onOpen,
  onChanged,
  className,
}: {
  vault: VaultSummary
  records: RecordPreview[]
  issues: Map<string, HealthIssue[]>
  unavailable: boolean
  onOpen: () => void
  /** The vault was starred or unstarred. */
  onChanged: () => void
  className?: string
}) {
  const inside = records.filter((record) => record.vault_id === vault.id)
  const flagged = inside.filter((record) => issues.has(record.id) || record.compromised_at).length
  const aws = vault.kind === 'aws'
  const opened = vault.last_opened_at ? when(new Date(vault.last_opened_at * 1000).toISOString()) : null
  const [favorite, setFavorite] = useState(!!vault.favorite)

  useEffect(() => setFavorite(!!vault.favorite), [vault.favorite])

  const star = () => {
    const next = !favorite
    setFavorite(next)
    void api
      .vaultPrefs(vault.id, { favorite: next })
      .then(onChanged)
      .catch((cause) => {
        setFavorite(!next)
        toastError(cause, t('Não foi possível salvar o favorito'))
      })
  }

  const agent =
    !vault.agent_access
      ? t('agente bloqueado')
      : vault.agent_policy?.mode === 'always' || (vault.agent_policy?.mode === 'until' && vault.agent_policy.at * 1000 > Date.now())
        ? t('agente sem perguntar')
        : t('agente liberado')

  return (
    <TabMenu to={{ place: 'vault', vault: vault.id }}>
      <div
        className={cn(
          'group/card relative flex h-[136px] min-w-0 flex-col rounded-[14px] border bg-card p-[18px] text-left transition-colors hover:border-line-2',
          className,
        )}
      >
        {/* The whole card opens the vault; the star sits above it. */}
        <button
          type="button"
          aria-label={t('Abrir o cofre {name}', { name: vault.name })}
          {...linkProps({ place: 'vault', vault: vault.id }, onOpen)}
          className="absolute inset-0 rounded-[14px] focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none"
        />

        <span className="pointer-events-none flex min-w-0 items-start gap-3">
          <VaultInitial vault={vault} size="lg" />
          <span className="min-w-0 flex-1">
            <b className="block truncate text-[15px] leading-5 font-semibold" title={vault.name}>
              {vault.name}
            </b>
            <span className="mt-0.5 flex min-w-0 items-center gap-1 text-xs text-faint">
              {aws ? <AwsMark className="size-3.5 shrink-0" /> : <HardDrive className="size-3 shrink-0" />}
              <span className="truncate">{aws ? t('AWS Parameter Store') : t('Cofre local')}</span>
            </span>
          </span>
          <button
            type="button"
            aria-label={favorite ? t('Remover dos favoritos') : t('Favoritar cofre')}
            aria-pressed={favorite}
            onClick={star}
            className={cn(
              'pointer-events-auto relative z-10 -mt-1 -mr-1.5 grid size-7 shrink-0 place-items-center rounded-lg text-faint hover:bg-hover hover:text-foreground',
              !favorite && 'opacity-0 group-hover/card:opacity-100 focus-visible:opacity-100',
            )}
          >
            <Star className={cn('size-4', favorite && 'fill-amber text-amber')} />
          </button>
        </span>

        <span className="pointer-events-none mt-auto flex min-w-0 flex-col gap-1 text-[13px]">
          {unavailable ? (
            <>
              <b className="truncate font-semibold text-amber">{t('Indisponível')}</b>
              <span className="truncate text-faint">
                {aws ? t('sem sessão na AWS. Abra o cofre para renovar') : t('não foi possível ler este cofre')}
              </span>
            </>
          ) : (
            <>
              <span className="flex min-w-0 gap-4 text-muted-foreground">
                <span className="truncate">
                  <b className="font-semibold text-foreground">{inside.length}</b> {inside.length === 1 ? t('registro') : t('registros')}
                </span>
                <span className={cn('truncate', flagged > 0 && 'text-amber')}>
                  <b className={cn('font-semibold', flagged > 0 ? 'text-amber' : 'text-foreground')}>{flagged}</b> {t('atenção')}
                </span>
              </span>
              <span className="flex min-w-0 gap-2 text-muted-foreground">
                <span className="truncate">{agent}</span>
                {opened && <span className="ml-auto shrink-0 text-faint">{t('aberto {when}', { when: opened })}</span>}
              </span>
            </>
          )}
        </span>
      </div>
    </TabMenu>
  )
}

/** The health of every vault's passwords, by record: what "atenção" counts on the cards. */
export function useHealthIssues(vaults: VaultSummary[]): Map<string, HealthIssue[]> {
  const [issues, setIssues] = useState<Map<string, HealthIssue[]>>(new Map())
  const key = vaults
    .filter((vault) => !vault.archived)
    .map((vault) => vault.id)
    .join(',')

  useEffect(() => {
    const ids = key ? key.split(',') : []
    void Promise.all(ids.map((id) => api.health(id).catch(() => null))).then((reports) => {
      const found = new Map<string, HealthIssue[]>()
      reports.forEach((report) => report?.health.findings.forEach((one) => found.set(one.record_id, one.issues)))
      setIssues(found)
    })
  }, [key])

  return issues
}

/** Favourites first, then the most opened, the most recent breaking a tie. */
export function byUse(a: VaultSummary, b: VaultSummary): number {
  return (
    Number(!!b.favorite) - Number(!!a.favorite) ||
    (b.opened ?? 0) - (a.opened ?? 0) ||
    (b.last_opened_at ?? 0) - (a.last_opened_at ?? 0) ||
    a.name.localeCompare(b.name)
  )
}
