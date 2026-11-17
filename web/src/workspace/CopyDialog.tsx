import { useEffect, useMemo, useRef, useState } from 'react'
import { AlertTriangle, Check, CircleAlert, Copy, Loader2 } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { t } from '@/i18n'
import { cn } from '@/lib/utils'

import {
  api,
  ApiError,
  type CopyItem,
  type CopyPlan,
  type CopyRunItem,
  type CopyScope,
  type FolderNode,
  type VaultSummary,
} from '../api'
import { useAwsRenew } from './AwsAccounts'
import { AwsMark } from './AwsMark'
import { FolderCombobox } from './FolderCombobox'
import { VaultInitial } from './shell'

/**
 * "Copiar para…": a whole vault, a folder or some records, into a folder of any vault (§9.1).
 *
 * The steps follow the decisions taken for it: the destination; a review that, going to AWS, lists
 * every name that will become visible metadata there (C6); the copy itself, one record per call, so
 * it stops at a known place, asks about each name already taken (C4) and can continue from where
 * it stopped (C5); and a summary that offers, never by default, to delete the originals (C1, C7).
 */
type Step = 'destino' | 'revisao' | 'copiando' | 'conflito' | 'resumo'

type Choice = 'overwrite' | 'rename' | 'skip'

interface Outcome {
  item: CopyItem
  status: 'copied' | 'renamed' | 'overwritten' | 'skipped'
  destId?: string
  as?: string
  warning?: string
  /** Every attachment arrived. Only then may the original be deleted. */
  filesOk?: boolean
}

export interface CopySource {
  vaultId: string
  folderId?: string | null
  recordIds?: string[] | null
  /** What is being copied, for the title: "o cofre Pessoal", "a pasta Bancos", "“GitHub”". */
  what: string
}

const ROOT = '__root__'

/** A translated sentence with `{name}` slots filled by elements, for a word in bold inside it. */
function rich(text: string, values: Record<string, React.ReactNode>): React.ReactNode {
  return text.split(/(\{\w+\})/).map((part, index) => {
    const slot = part.match(/^\{(\w+)\}$/)?.[1]

    return slot && slot in values ? <span key={index}>{values[slot]}</span> : part
  })
}

function nextTitle(title: string): string {
  const match = title.match(/^(.*) \((\d+)\)$/)
  return match ? `${match[1]} (${Number(match[2]) + 1})` : `${title} (2)`
}

/**
 * The parameter name a rename starts from: the name it would have had, with a number. The binary
 * moves on to the next number when that one is taken too, by the destination or by an earlier
 * record of the same copy.
 */
function renamedName(name: string | null): string | null {
  if (!name) return null
  const match = name.match(/^(.*)-(\d+)$/)
  return match ? `${match[1]}-${Number(match[2]) + 1}` : `${name}-2`
}

/** Overwriting is never offered over a `String` parameter, a name another copied record takes, or the record itself. */
function canOverwrite(item: CopyItem): boolean {
  const conflict = item.conflict
  return !!conflict && !conflict.read_only && !!conflict.dest_id && conflict.reason !== 'self'
}

export function CopyDialog({
  source,
  vaults,
  onDone,
  onCancel,
  onAccounts,
}: {
  source: CopySource
  vaults: VaultSummary[]
  onDone: () => void
  onCancel: () => void
  onAccounts?: () => void
}) {
  const choices = vaults.filter((vault) => !vault.archived)
  const sourceVault = vaults.find((vault) => vault.id === source.vaultId) ?? null

  const [step, setStep] = useState<Step>('destino')
  const [destVault, setDestVault] = useState(
    () => choices.find((vault) => vault.id !== source.vaultId)?.id ?? choices[0]?.id ?? '',
  )
  const [folders, setFolders] = useState<FolderNode[]>([])
  const [destFolder, setDestFolder] = useState(ROOT)
  const [plan, setPlan] = useState<CopyPlan | null>(null)
  const [excluded, setExcluded] = useState<Set<string>>(new Set())
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [errorKind, setErrorKind] = useState<string | undefined>()

  const [queue, setQueue] = useState<CopyItem[]>([])
  const [at, setAt] = useState(0)
  const [outcomes, setOutcomes] = useState<Outcome[]>([])
  const [stoppedAt, setStoppedAt] = useState<{ index: number; message: string } | null>(null)
  const [standing, setStanding] = useState<Choice | null>(null)

  const [conflict, setConflict] = useState<{ index: number; item: CopyItem } | null>(null)
  const [choice, setChoice] = useState<Choice>('rename')
  const [newTitle, setNewTitle] = useState('')
  const [newName, setNewName] = useState('')
  const [forNext, setForNext] = useState(false)

  const [deleteOriginals, setDeleteOriginals] = useState(false)
  const [typed, setTyped] = useState('')
  const [deleted, setDeleted] = useState<string>('')

  const cancelled = useRef(false)
  // A copy keeps going only while this dialog exists: closing it, locking, or leaving the page
  // stops the loop at the next record.
  useEffect(() => () => void (cancelled.current = true), [])

  const [renewed, setRenewed] = useState(false)
  const { renew, busy: renewing, dialog: renewDialog } = useAwsRenew(() => setRenewed(true))

  /** Renew the session of whichever AWS vault this copy touches, without leaving the copy. */
  async function renewHere() {
    try {
      const found = await api.aws()
      const connections = [sourceVault, dest]
        .map((vault) => vault?.aws?.connection_id)
        .filter((id): id is string => !!id)
      const loginIds = found.connections.filter((one) => connections.includes(one.id)).map((one) => one.login_id)
      const mine = found.logins.filter((login) => loginIds.includes(login.id))
      const login = mine.find((one) => !one.connected) ?? mine[0]
      if (login) void renew(login)
      else onAccounts?.()
    } catch (cause) {
      fail(cause, t('Não foi possível carregar as contas AWS.'))
    }
  }

  const dest = choices.find((vault) => vault.id === destVault) ?? null
  const destAws = dest?.kind === 'aws'
  const sourceAws = sourceVault?.kind === 'aws'

  useEffect(() => {
    if (!destVault) return
    let current = true
    setDestFolder(ROOT)
    void api
      .folders(destVault)
      .then((answer) => current && setFolders(answer.folders))
      .catch(() => current && setFolders([]))
    return () => {
      current = false
    }
  }, [destVault])

  const scope: CopyScope = useMemo(
    () => ({
      source_vault: source.vaultId,
      folder_id: source.folderId ?? null,
      record_ids: source.recordIds ?? null,
      dest_vault: destVault,
      dest_folder: destFolder === ROOT ? null : destFolder,
    }),
    [source, destVault, destFolder],
  )

  function fail(cause: unknown, fallback: string) {
    setError(cause instanceof ApiError ? cause.message : fallback)
    setErrorKind(cause instanceof ApiError ? cause.aws : undefined)
  }

  async function review() {
    setBusy(true)
    setError('')
    try {
      const found = await api.copyPlan(scope)
      setPlan(found)
      setExcluded(new Set(found.items.filter((item) => item.error).map((item) => item.source_id)))
      setStep('revisao')
    } catch (cause) {
      fail(cause, t('Não foi possível planejar a cópia.'))
    } finally {
      setBusy(false)
    }
  }

  const chosen = plan ? plan.items.filter((item) => !excluded.has(item.source_id)) : []
  const files = chosen.reduce((sum, item) => sum + (item.attachments ?? 0), 0)
  const conflicts = chosen.filter((item) => item.conflict).length

  function start() {
    cancelled.current = false
    setQueue(chosen)
    setOutcomes([])
    setAt(0)
    setStoppedAt(null)
    setStanding(null)
    setStep('copiando')
    void runFrom(chosen, 0, [], null)
  }

  /** Copy from `index` on, one record per call. Stops at a conflict to ask, or at a failure. */
  async function runFrom(items: CopyItem[], index: number, done: Outcome[], rule: Choice | null) {
    let current = done

    for (let i = index; i < items.length; i += 1) {
      if (cancelled.current) return
      const item = items[i]
      setAt(i)

      let run: CopyRunItem = { source_id: item.source_id, rel: item.rel, action: 'create', name: item.name }
      let status: Outcome['status'] = 'copied'
      let as: string | undefined

      if (item.conflict) {
        const allowed: Choice | null = rule && !(rule === 'overwrite' && !canOverwrite(item)) ? rule : null

        if (!allowed) {
          setConflict({ index: i, item })
          setChoice('rename')
          setNewTitle(nextTitle(item.title))
          setNewName(renamedName(item.name) ?? '')
          setForNext(false)
          setOutcomes(current)
          setStep('conflito')
          return
        }

        if (allowed === 'skip') {
          current = [...current, { item, status: 'skipped' }]
          setOutcomes(current)
          continue
        }

        if (allowed === 'overwrite') {
          run = { ...run, action: 'overwrite', dest_id: item.conflict.dest_id, expected_version: item.conflict.version ?? null }
          status = 'overwritten'
        } else {
          const title = nextTitle(item.title)
          run = { ...run, action: 'rename', title, name: renamedName(item.name) }
          status = 'renamed'
        }
      }

      try {
        const answer = await api.copyRun(scope, run)
        if (status === 'renamed') as = answer.name ?? answer.title
        current = [
          ...current,
          { item, status, destId: answer.dest_id, as, warning: answer.warning, filesOk: answer.files_ok },
        ]
        setOutcomes(current)
      } catch (cause) {
        setOutcomes(current)
        setStoppedAt({ index: i, message: cause instanceof ApiError ? cause.message : t('A cópia foi interrompida.') })
        setErrorKind(cause instanceof ApiError ? cause.aws : undefined)
        return
      }
    }

    setOutcomes(current)
    setAt(items.length)
    setStep('resumo')
  }

  async function decide() {
    if (!conflict) return
    const { index, item } = conflict
    const rule = forNext ? choice : standing
    if (forNext) setStanding(choice)
    setConflict(null)
    setStep('copiando')

    if (choice === 'skip') {
      const done = [...outcomes, { item, status: 'skipped' as const }]
      void runFrom(queue, index + 1, done, rule)
      return
    }

    const run: CopyRunItem =
      choice === 'overwrite'
        ? {
            source_id: item.source_id,
            rel: item.rel,
            action: 'overwrite',
            dest_id: item.conflict?.dest_id,
            expected_version: item.conflict?.version ?? null,
          }
        : { source_id: item.source_id, rel: item.rel, action: 'rename', title: newTitle.trim(), name: item.name ? newName.trim() : null }

    try {
      const answer = await api.copyRun(scope, run)
      if (cancelled.current) return
      const done: Outcome[] = [
        ...outcomes,
        {
          item,
          status: choice === 'overwrite' ? 'overwritten' : 'renamed',
          destId: answer.dest_id,
          as: choice === 'rename' ? (answer.name ?? answer.title ?? newTitle.trim()) : undefined,
          warning: answer.warning,
          filesOk: answer.files_ok,
        },
      ]
      void runFrom(queue, index + 1, done, rule)
    } catch (cause) {
      setStoppedAt({ index, message: cause instanceof ApiError ? cause.message : t('A cópia foi interrompida.') })
      setErrorKind(cause instanceof ApiError ? cause.aws : undefined)
    }
  }

  function resume() {
    if (!stoppedAt) return
    const index = stoppedAt.index
    setStoppedAt(null)
    setErrorKind(undefined)
    setRenewed(false)
    void runFrom(queue, index, outcomes, standing)
  }

  // Only what arrived whole, somewhere other than where it already was. The binary checks the same
  // again; this keeps the offer honest.
  const copiedIds = outcomes
    .filter((one) => one.status !== 'skipped' && one.filesOk !== false && one.destId && one.destId !== one.item.source_id)
    .map((one) => one.item.source_id)
  const notCopied = queue.length - outcomes.length
  const mustType = sourceAws

  async function removeOriginals() {
    setBusy(true)
    setError('')
    try {
      const answer = await api.copyOriginals(source.vaultId, copiedIds, mustType ? typed : undefined)
      const n = answer.removed
      const vault = sourceVault?.name ?? t('origem')
      setDeleted(
        answer.trash
          ? n === 1
            ? t('{n} original movido para a lixeira de {vault}.', { n, vault })
            : t('{n} originais movidos para a lixeira de {vault}.', { n, vault })
          : n === 1
            ? t('{n} parâmetro excluído permanentemente da AWS.', { n })
            : t('{n} parâmetros excluídos permanentemente da AWS.', { n }),
      )
      if (answer.failed.length > 0) {
        const failed = { n: answer.failed.length, error: answer.failed[0].error }
        setError(failed.n === 1 ? t('{n} não foi excluído: {error}', failed) : t('{n} não foram excluídos: {error}', failed))
      }
    } catch (cause) {
      fail(cause, t('Não foi possível excluir os originais.'))
    } finally {
      setBusy(false)
    }
  }

  const title = t('Copiar {what}', { what: source.what })

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (open || step === 'copiando') return
        // Something already arrived: the vault has to be read again, whatever step this is.
        if (step === 'resumo' || outcomes.length > 0) {
          cancelled.current = true
          onDone()
        } else {
          onCancel()
        }
      }}
    >
      {renewDialog}
      <DialogContent className="max-h-[calc(100vh-48px)] gap-0 overflow-hidden p-0 sm:max-w-[620px]">
        <DialogHeader className="border-b px-6 pt-5 pb-4">
          <DialogTitle className="flex items-center gap-2 text-lg">
            <Copy className="size-[18px] text-muted-foreground" />
            {title}
          </DialogTitle>
          <DialogDescription>
            {t('O original é mantido. Ao final, você decide se ele deve ser excluído.')}
          </DialogDescription>
          <Steps step={step} />
        </DialogHeader>

        <div className="min-h-0 space-y-4 overflow-y-auto px-6 py-5">
          {step === 'destino' && (
            <>
              <div className="grid gap-1.5">
                <span className="text-[13px] font-medium">{t('Cofre de destino')}</span>
                <div className="grid max-h-[240px] gap-1.5 overflow-y-auto">
                  {choices.map((vault) => (
                    <button
                      key={vault.id}
                      type="button"
                      aria-pressed={vault.id === destVault}
                      onClick={() => setDestVault(vault.id)}
                      className={cn(
                        'flex items-center gap-3 rounded-xl border px-3 py-2.5 text-left hover:bg-hover',
                        vault.id === destVault && 'border-primary bg-primary-soft hover:bg-primary-soft',
                      )}
                    >
                      <VaultInitial vault={vault} size="md" />
                      <span className="min-w-0 flex-1">
                        <b className="block truncate text-sm font-semibold">{vault.name}</b>
                        <span className="block truncate text-xs text-muted-foreground">
                          {vault.kind === 'aws'
                            ? `AWS Parameter Store${vault.aws ? `, ${vault.aws.region}` : ''}`
                            : t('Cofre local')}
                          {vault.id === source.vaultId ? t(', o mesmo da origem') : ''}
                        </span>
                      </span>
                      {vault.kind === 'aws' && <AwsMark className="size-5" />}
                      {vault.id === destVault && <Check className="size-4 text-primary-text" />}
                    </button>
                  ))}
                </div>
              </div>

              <label className="grid gap-1.5">
                <span className="text-[13px] font-medium">{destAws ? t('Caminho de destino') : t('Pasta de destino')}</span>
                <FolderCombobox
                  label={destAws ? t('Caminho de destino') : t('Pasta de destino')}
                  rootLabel={destAws ? t('/ (raiz da conta)') : t('Raiz do cofre')}
                  folders={folders}
                  value={destFolder === ROOT ? null : destFolder}
                  onChange={(id) => setDestFolder(id ?? ROOT)}
                />
                <span className="text-xs text-faint">
                  {source.folderId
                    ? t('A pasta é copiada inteira, com as subpastas, para dentro desta.')
                    : source.recordIds
                      ? t('Os registros são copiados diretamente para esta pasta.')
                      : t('As pastas do cofre são recriadas dentro desta.')}
                </span>
              </label>
            </>
          )}

          {step === 'revisao' && plan && (
            <>
              {!plan.checks.ok && (
                <div className="space-y-2 rounded-xl border border-rose/40 bg-rose-soft px-4 py-3 text-[13px]">
                  <b className="flex items-center gap-2 font-semibold">
                    <CircleAlert className="size-4 text-rose" />
                    {t('Resolva antes de copiar')}
                  </b>
                  <ul className="list-disc space-y-1 pl-5 text-foreground">
                    {plan.checks.problems.map((problem) => (
                      <li key={problem}>{problem}</li>
                    ))}
                  </ul>
                </div>
              )}

              <Summary
                rows={[
                  [t('Registros'), String(chosen.length)],
                  [t('Anexos'), String(files)],
                  [t('Nomes já ocupados'), conflicts > 0 ? t('{n}, decididos um a um', { n: conflicts }) : t('nenhum')],
                  [t('Histórico'), t('Apenas a versão atual. O histórico permanece no original')],
                ]}
              />

              {plan.metadata_visible ? (
                <>
                  <div className="flex gap-3 rounded-xl bg-amber-soft px-4 py-3 text-[13px] text-foreground">
                    <AlertTriangle className="mt-0.5 size-4 shrink-0 text-amber" />
                    <span>
                      {t('Na AWS, o nome do parâmetro e o caminho das pastas não são cifrados: quem possui ssm:DescribeParameters na conta visualiza esta lista, mesmo sem poder ler os valores. Remova da cópia o que não deve ficar visível.')}
                    </span>
                  </div>
                  <ul className="divide-y rounded-xl border">
                    {plan.items.map((item) => {
                      const off = excluded.has(item.source_id)
                      return (
                        <li key={item.source_id} className="flex items-start gap-3 px-3 py-2.5">
                          <Checkbox
                            className="mt-0.5"
                            checked={!off}
                            disabled={!!item.error}
                            aria-label={t('Copiar {title}', { title: item.title })}
                            onCheckedChange={(on) => {
                              const next = new Set(excluded)
                              if (on) next.delete(item.source_id)
                              else next.add(item.source_id)
                              setExcluded(next)
                            }}
                          />
                          <span className="min-w-0 flex-1">
                            <span className={cn('block truncate font-mono text-[12.5px]', off && 'text-faint line-through')}>
                              {item.name ?? item.path}
                            </span>
                            <span className="block truncate text-xs text-muted-foreground">{item.title}</span>
                            {item.error && <span className="block text-xs text-rose">{item.error}</span>}
                          </span>
                          {item.conflict && <ConflictBadge item={item} />}
                        </li>
                      )
                    })}
                  </ul>
                </>
              ) : (
                conflicts > 0 && (
                  <ul className="divide-y rounded-xl border">
                    {chosen
                      .filter((item) => item.conflict)
                      .map((item) => (
                        <li key={item.source_id} className="flex items-center gap-3 px-3 py-2.5 text-sm">
                          <span className="min-w-0 flex-1 truncate">{item.path}</span>
                          <ConflictBadge item={item} />
                        </li>
                      ))}
                  </ul>
                )
              )}
            </>
          )}

          {(step === 'copiando' || step === 'conflito') && (
            <div className="space-y-3">
              <div className="flex items-center justify-between text-sm" aria-live="polite">
                <span className="font-medium">
                  {stoppedAt ? t('Cópia interrompida') : step === 'conflito' ? t('Aguardando sua decisão') : t('Copiando…')}
                </span>
                <span className="text-muted-foreground tabular-nums">
                  {t('{done} de {total}', { done: outcomes.length, total: queue.length })}
                </span>
              </div>
              <div
                role="progressbar"
                aria-label={t('Progresso da cópia')}
                aria-valuemin={0}
                aria-valuemax={queue.length}
                aria-valuenow={outcomes.length}
                className="h-2 overflow-hidden rounded-full bg-raised"
              >
                <div
                  className={cn('h-full rounded-full transition-[width]', stoppedAt ? 'bg-rose' : 'bg-primary')}
                  style={{ width: `${queue.length ? (outcomes.length / queue.length) * 100 : 0}%` }}
                />
              </div>
              {!stoppedAt && step === 'copiando' && queue[at] && (
                <p className="flex items-center gap-2 truncate text-xs text-muted-foreground">
                  <Loader2 className="size-3.5 animate-spin" />
                  {queue[at].title}
                </p>
              )}

              {stoppedAt && (
                <div className="space-y-2 rounded-xl border border-rose/40 bg-rose-soft px-4 py-3 text-[13px]">
                  <b className="block font-semibold">{t('Interrompida em “{title}”', { title: queue[stoppedAt.index]?.title ?? '' })}</b>
                  <p>{stoppedAt.message}</p>
                  <p className="text-muted-foreground">
                    {outcomes.length === 1
                      ? t('{n} registro já copiado não será copiado novamente.', { n: 1 })
                      : t('{n} registros já copiados não serão copiados novamente.', { n: outcomes.length })}
                  </p>
                  {errorKind === 'session_expired' &&
                    (renewed ? (
                      <p className="text-mint">{t('Sessão renovada. Continue de onde a cópia parou.')}</p>
                    ) : (
                      <Button variant="outline" size="sm" disabled={renewing} onClick={() => void renewHere()}>
                        {renewing && <Loader2 className="animate-spin" />}
                        {t('Renovar a sessão AWS')}
                      </Button>
                    ))}
                </div>
              )}

              {step === 'conflito' && conflict && (
                <div className="space-y-3 rounded-xl border p-4">
                  <p className="text-sm">
                    {rich(
                      conflict.item.conflict?.reason === 'repeated'
                        ? t('Já existe {name} no destino, vindo de outro registro desta cópia.')
                        : conflict.item.conflict?.reason === 'self'
                          ? t('Já existe {name} no destino, e é o próprio registro em cópia.')
                          : t('Já existe {name} no destino.'),
                      { name: <b className="font-semibold">{conflict.item.name ?? conflict.item.path}</b> },
                    )}
                  </p>
                  <div className="grid grid-cols-3 gap-2">
                    {(['rename', 'overwrite', 'skip'] as const).map((one) => {
                      const blocked = one === 'overwrite' && !canOverwrite(conflict.item)
                      return (
                        <button
                          key={one}
                          type="button"
                          disabled={blocked}
                          aria-pressed={choice === one}
                          onClick={() => setChoice(one)}
                          className={cn(
                            'rounded-xl border px-3 py-2.5 text-left text-sm hover:bg-hover disabled:cursor-not-allowed disabled:opacity-45',
                            choice === one && 'border-primary bg-primary-soft hover:bg-primary-soft',
                          )}
                        >
                          <b className="block font-semibold">
                            {one === 'rename' ? t('Renomear') : one === 'overwrite' ? t('Sobrescrever') : t('Pular')}
                          </b>
                          <span className="block text-xs text-muted-foreground">
                            {one === 'rename'
                              ? t('Cria com outro nome')
                              : one === 'overwrite'
                                ? conflict.item.conflict?.reason === 'self'
                                  ? t('É o próprio registro')
                                  : conflict.item.conflict?.read_only
                                    ? t('Indisponível para parâmetro String')
                                    : t('Cria uma nova versão')
                                : t('Não copia este registro')}
                          </span>
                        </button>
                      )
                    })}
                  </div>

                  {choice === 'rename' && (
                    <div className="grid gap-3">
                      <label className="grid gap-1.5">
                        <span className="text-[13px] font-medium">{t('Novo título')}</span>
                        <Input
                          value={newTitle}
                          onChange={(event) => setNewTitle(event.target.value)}
                        />
                      </label>
                      {conflict.item.name && (
                        <label className="grid gap-1.5">
                          <span className="text-[13px] font-medium">{t('Nome do parâmetro')}</span>
                          <Input className="font-mono text-[13px]" value={newName} onChange={(event) => setNewName(event.target.value)} />
                        </label>
                      )}
                    </div>
                  )}

                  <label className="flex items-center gap-2 text-sm">
                    <Checkbox checked={forNext} onCheckedChange={(on) => setForNext(on === true)} />
                    {t('Aplicar aos próximos nomes ocupados')}
                  </label>
                </div>
              )}
            </div>
          )}

          {step === 'resumo' && (
            <>
              <Summary
                rows={[
                  [t('Copiados'), String(outcomes.filter((one) => one.status === 'copied').length)],
                  [t('Renomeados'), String(outcomes.filter((one) => one.status === 'renamed').length)],
                  [t('Sobrescritos'), String(outcomes.filter((one) => one.status === 'overwritten').length)],
                  [t('Pulados'), String(outcomes.filter((one) => one.status === 'skipped').length)],
                  ...(notCopied ? [notCopiedRow(notCopied)] : []),
                ]}
              />
              <p className="text-[13px] text-muted-foreground">
                {t('Apenas a versão atual de cada registro foi copiada. O histórico de versões permanece no original.')}
              </p>

              {outcomes.some((one) => one.warning || one.as) && (
                <ul className="space-y-1.5 text-[13px]">
                  {outcomes
                    .filter((one) => one.as)
                    .map((one) => (
                      <li key={`r-${one.item.source_id}`}>
                        {rich(t('“{title}” foi copiado como {as}'), { title: one.item.title, as: <b className="font-medium">{one.as}</b> })}
                      </li>
                    ))}
                  {outcomes
                    .filter((one) => one.warning)
                    .map((one) => (
                      <li key={`w-${one.item.source_id}`} className="text-amber">
                        {one.item.title}: {one.warning}
                      </li>
                    ))}
                </ul>
              )}

              {copiedIds.length > 0 && !deleted && (
                <div className="space-y-3 rounded-xl border p-4">
                  <label className="flex items-start gap-2 text-sm">
                    <Checkbox className="mt-0.5" checked={deleteOriginals} onCheckedChange={(on) => setDeleteOriginals(on === true)} />
                    <span>
                      <b className="block font-semibold">{t('Excluir os originais em {vault}', { vault: sourceVault?.name ?? '' })}</b>
                      <span className="block text-xs text-muted-foreground">
                        {mustType
                          ? t('Apenas os {n} copiados por completo. Na AWS não há lixeira: eles são excluídos permanentemente, com os anexos. Esta ação não pode ser desfeita.', { n: copiedIds.length })
                          : t('Apenas os {n} copiados por completo. Eles são movidos para a lixeira e podem ser restaurados.', { n: copiedIds.length })}
                      </span>
                    </span>
                  </label>

                  {deleteOriginals && (
                    <div className="grid gap-2">
                      {mustType && (
                        <label className="grid gap-1.5">
                          <span className="text-[13px]">
                            {rich(t('Digite {vault} para confirmar'), { vault: <b className="font-semibold">{sourceVault?.name}</b> })}
                          </span>
                          <Input value={typed} onChange={(event) => setTyped(event.target.value)} />
                        </label>
                      )}
                      <Button
                        variant="outline"
                        className="justify-self-start border-destructive/50 text-destructive hover:bg-destructive/10"
                        disabled={busy || (mustType && typed.trim() !== (sourceVault?.name ?? '').trim())}
                        onClick={() => void removeOriginals()}
                      >
                        {mustType ? t('Excluir permanentemente') : t('Excluir originais')}
                      </Button>
                    </div>
                  )}
                </div>
              )}

              {deleted && <p className="text-sm text-mint">{deleted}</p>}
            </>
          )}

          {error && (
            <div className="space-y-2">
              <p className="text-sm text-rose">{error}</p>
              {errorKind === 'session_expired' && (
                <Button variant="outline" size="sm" disabled={renewing} onClick={() => void renewHere()}>
                  {t('Renovar a sessão AWS')}
                </Button>
              )}
            </div>
          )}
        </div>

        <DialogFooter className="m-0 border-t px-6 py-4">
          {step === 'destino' && (
            <>
              <Button variant="outline" onClick={onCancel}>
                {t('Cancelar')}
              </Button>
              <Button disabled={!destVault || busy} onClick={() => void review()}>
                {busy && <Loader2 className="animate-spin" />}
                {t('Revisar')}
              </Button>
            </>
          )}

          {step === 'revisao' && plan && (
            <>
              <Button variant="outline" onClick={() => setStep('destino')}>
                {t('Voltar')}
              </Button>
              <Button disabled={!plan.checks.ok || chosen.length === 0} onClick={start}>
                {chosen.length === 1 ? t('Copiar {n} registro', { n: 1 }) : t('Copiar {n} registros', { n: chosen.length })}
              </Button>
            </>
          )}

          {step === 'copiando' && (
            <>
              {stoppedAt ? (
                <>
                  <Button variant="outline" onClick={() => setStep('resumo')}>
                    {t('Encerrar aqui')}
                  </Button>
                  <Button onClick={resume}>{t('Continuar de onde parou')}</Button>
                </>
              ) : (
                <>
                  {/* Parar sits away from where "Copiar" and "Continuar" were, so the second click of
                      a double click lands on the disabled button, never on Parar. */}
                  <Button
                    variant="ghost"
                    className="sm:mr-auto"
                    onClick={() => {
                      cancelled.current = true
                      setStep('resumo')
                    }}
                  >
                    {t('Parar')}
                  </Button>
                  <Button disabled>
                    <Loader2 className="animate-spin" />
                    {t('Copiando…')}
                  </Button>
                </>
              )}
            </>
          )}

          {step === 'conflito' && (
            <>
              <Button
                variant="outline"
                onClick={() => {
                  cancelled.current = true
                  setConflict(null)
                  setStep('resumo')
                }}
              >
                {t('Parar aqui')}
              </Button>
              <Button
                disabled={choice === 'rename' && (!newTitle.trim() || (!!conflict?.item.name && !newName.trim()))}
                onClick={() => void decide()}
              >
                {t('Continuar')}
              </Button>
            </>
          )}

          {step === 'resumo' && <Button onClick={onDone}>{t('Concluir')}</Button>}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function notCopiedRow(count: number): [string, string] {
  return [t('Não copiados'), String(count)]
}

function Steps({ step }: { step: Step }) {
  const order: Array<[Step[], string]> = [
    [['destino'], t('Destino')],
    [['revisao'], t('Revisão')],
    [['copiando', 'conflito'], t('Cópia')],
    [['resumo'], t('Resumo')],
  ]
  const current = order.findIndex(([steps]) => steps.includes(step))

  return (
    <ol className="mt-3 flex items-center gap-2 text-xs">
      {order.map(([, label], index) => (
        <li key={label} aria-current={index === current ? 'step' : undefined} className="flex items-center gap-2">
          <span
            className={cn(
              'grid size-5 place-items-center rounded-full border text-[11px] font-semibold',
              index < current && 'border-primary bg-primary text-primary-foreground',
              index === current && 'border-primary text-primary-text',
              index > current && 'text-faint',
            )}
          >
            {index < current ? <Check className="size-3" strokeWidth={3} /> : index + 1}
          </span>
          <span className={cn(index === current ? 'font-medium text-foreground' : 'text-muted-foreground')}>{label}</span>
          {index < order.length - 1 && <span className="h-px w-5 bg-border" />}
        </li>
      ))}
    </ol>
  )
}

function Summary({ rows }: { rows: Array<[string, string]> }) {
  return (
    <dl className="grid grid-cols-[140px_minmax(0,1fr)] gap-x-4 gap-y-2 rounded-xl bg-raised px-4 py-3 text-sm">
      {rows.map(([label, value]) => (
        <div key={label} className="contents">
          <dt className="text-muted-foreground">{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  )
}

function ConflictBadge({ item }: { item: CopyItem }) {
  const reason = item.conflict?.reason
  return (
    <span className="shrink-0 rounded-full bg-amber-soft px-2 py-0.5 text-[11px] font-medium text-amber">
      {reason === 'self'
        ? t('é o próprio')
        : reason === 'repeated'
          ? t('nome repetido')
          : item.conflict?.read_only
            ? t('já existe, String')
            : t('já existe')}
    </span>
  )
}
