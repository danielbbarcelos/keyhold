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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
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
      fail(cause, 'Não consegui ler as contas AWS.')
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
      fail(cause, 'Não consegui planejar a cópia.')
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
        setStoppedAt({ index: i, message: cause instanceof ApiError ? cause.message : 'A cópia parou.' })
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
      setStoppedAt({ index, message: cause instanceof ApiError ? cause.message : 'A cópia parou.' })
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
      setDeleted(
        answer.trash
          ? `${answer.removed} original(is) na lixeira de ${sourceVault?.name ?? 'origem'}.`
          : `${answer.removed} parâmetro(s) apagado(s) da AWS.`,
      )
      if (answer.failed.length > 0) {
        setError(`${answer.failed.length} não saíram: ${answer.failed[0].error}`)
      }
    } catch (cause) {
      fail(cause, 'Não apaguei os originais.')
    } finally {
      setBusy(false)
    }
  }

  const title = `Copiar ${source.what}`

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
            O original fica onde está. No fim, você decide se quer apagá-lo.
          </DialogDescription>
          <Steps step={step} />
        </DialogHeader>

        <div className="min-h-0 space-y-4 overflow-y-auto px-6 py-5">
          {step === 'destino' && (
            <>
              <div className="grid gap-1.5">
                <span className="text-[13px] font-medium">Cofre de destino</span>
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
                            : 'Cofre local'}
                          {vault.id === source.vaultId ? ', o mesmo da origem' : ''}
                        </span>
                      </span>
                      {vault.kind === 'aws' && <AwsMark className="size-5" />}
                      {vault.id === destVault && <Check className="size-4 text-primary-text" />}
                    </button>
                  ))}
                </div>
              </div>

              <label className="grid gap-1.5">
                <span className="text-[13px] font-medium">{destAws ? 'Caminho de destino' : 'Pasta de destino'}</span>
                <Select value={destFolder} onValueChange={setDestFolder}>
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={ROOT}>{destAws ? '/ (raiz da conta)' : 'Raiz do cofre'}</SelectItem>
                    {folders.map((folder) => (
                      <SelectItem key={folder.id} value={folder.id}>
                        {`${'  '.repeat(folder.depth)}${folder.name}`}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <span className="text-xs text-faint">
                  {source.folderId
                    ? 'A pasta vai inteira, com as subpastas, para dentro desta.'
                    : source.recordIds
                      ? 'Os registros entram direto nesta pasta.'
                      : 'As pastas do cofre são recriadas dentro desta.'}
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
                    Antes de copiar, resolva isto
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
                  ['Registros', String(chosen.length)],
                  ['Anexos', String(files)],
                  ['Nomes já ocupados', conflicts > 0 ? `${conflicts}, você decide um a um` : 'nenhum'],
                  ['Histórico', 'só a versão atual. O histórico fica no original'],
                ]}
              />

              {plan.metadata_visible ? (
                <>
                  <div className="flex gap-3 rounded-xl bg-amber-soft px-4 py-3 text-[13px] text-foreground">
                    <AlertTriangle className="mt-0.5 size-4 shrink-0 text-amber" />
                    <span>
                      Na AWS, o nome do parâmetro e o caminho das pastas não são cifrados: quem tem
                      ssm:DescribeParameters na conta vê esta lista, mesmo sem poder ler os valores. Tire da cópia o
                      que não deve aparecer assim.
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
                            aria-label={`Copiar ${item.title}`}
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
                  {stoppedAt ? 'A cópia parou' : step === 'conflito' ? 'Esperando sua decisão' : 'Copiando…'}
                </span>
                <span className="text-muted-foreground tabular-nums">
                  {outcomes.length} de {queue.length}
                </span>
              </div>
              <div
                role="progressbar"
                aria-label="Progresso da cópia"
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
                  <b className="block font-semibold">Parou em “{queue[stoppedAt.index]?.title}”</b>
                  <p>{stoppedAt.message}</p>
                  <p className="text-muted-foreground">
                    {outcomes.length} registro(s) já chegaram e não serão copiados de novo.
                  </p>
                  {errorKind === 'session_expired' &&
                    (renewed ? (
                      <p className="text-mint">Sessão renovada. Continue de onde parou.</p>
                    ) : (
                      <Button variant="outline" size="sm" disabled={renewing} onClick={() => void renewHere()}>
                        {renewing && <Loader2 className="animate-spin" />}
                        Renovar a sessão AWS
                      </Button>
                    ))}
                </div>
              )}

              {step === 'conflito' && conflict && (
                <div className="space-y-3 rounded-xl border p-4">
                  <p className="text-sm">
                    Já existe <b className="font-semibold">{conflict.item.name ?? conflict.item.path}</b> no destino
                    {conflict.item.conflict?.reason === 'repeated' ? ', vindo de outro registro desta mesma cópia' : ''}
                    {conflict.item.conflict?.reason === 'self' ? ', e é o próprio registro que está sendo copiado' : ''}.
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
                            {one === 'rename' ? 'Renomear' : one === 'overwrite' ? 'Sobrescrever' : 'Pular'}
                          </b>
                          <span className="block text-xs text-muted-foreground">
                            {one === 'rename'
                              ? 'Cria com outro nome'
                              : one === 'overwrite'
                                ? conflict.item.conflict?.reason === 'self'
                                  ? 'É o próprio registro'
                                  : conflict.item.conflict?.read_only
                                    ? 'Parâmetro String: nunca'
                                    : 'Vira uma versão nova'
                                : 'Não copia este'}
                          </span>
                        </button>
                      )
                    })}
                  </div>

                  {choice === 'rename' && (
                    <div className="grid gap-3">
                      <label className="grid gap-1.5">
                        <span className="text-[13px] font-medium">Novo título</span>
                        <Input
                          value={newTitle}
                          onChange={(event) => setNewTitle(event.target.value)}
                        />
                      </label>
                      {conflict.item.name && (
                        <label className="grid gap-1.5">
                          <span className="text-[13px] font-medium">Nome do parâmetro</span>
                          <Input className="font-mono text-[13px]" value={newName} onChange={(event) => setNewName(event.target.value)} />
                        </label>
                      )}
                    </div>
                  )}

                  <label className="flex items-center gap-2 text-sm">
                    <Checkbox checked={forNext} onCheckedChange={(on) => setForNext(on === true)} />
                    Fazer o mesmo nos próximos nomes ocupados
                  </label>
                </div>
              )}
            </div>
          )}

          {step === 'resumo' && (
            <>
              <Summary
                rows={[
                  ['Copiados', String(outcomes.filter((one) => one.status === 'copied').length)],
                  ['Renomeados', String(outcomes.filter((one) => one.status === 'renamed').length)],
                  ['Sobrescritos', String(outcomes.filter((one) => one.status === 'overwritten').length)],
                  ['Pulados', String(outcomes.filter((one) => one.status === 'skipped').length)],
                  ...(notCopied > 0 ? ([['Não copiados', String(notCopied)]] as Array<[string, string]>) : []),
                ]}
              />
              <p className="text-[13px] text-muted-foreground">
                Foi só a versão atual de cada registro. O histórico de versões continua no original.
              </p>

              {outcomes.some((one) => one.warning || one.as) && (
                <ul className="space-y-1.5 text-[13px]">
                  {outcomes
                    .filter((one) => one.as)
                    .map((one) => (
                      <li key={`r-${one.item.source_id}`}>
                        “{one.item.title}” entrou como <b className="font-medium">{one.as}</b>
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
                      <b className="block font-semibold">Apagar os originais em {sourceVault?.name}</b>
                      <span className="block text-xs text-muted-foreground">
                        {mustType
                          ? `Só os ${copiedIds.length} que chegaram inteiros. Na AWS não há lixeira: apaga de vez, com os anexos.`
                          : `Só os ${copiedIds.length} que chegaram inteiros. Vão para a lixeira e dá para restaurar.`}
                      </span>
                    </span>
                  </label>

                  {deleteOriginals && (
                    <div className="grid gap-2">
                      {mustType && (
                        <label className="grid gap-1.5">
                          <span className="text-[13px]">
                            Digite <b className="font-semibold">{sourceVault?.name}</b> para confirmar
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
                        {mustType ? 'Apagar da AWS' : 'Mandar para a lixeira'}
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
                  Renovar a sessão AWS
                </Button>
              )}
            </div>
          )}
        </div>

        <DialogFooter className="m-0 border-t px-6 py-4">
          {step === 'destino' && (
            <>
              <Button variant="outline" onClick={onCancel}>
                Cancelar
              </Button>
              <Button disabled={!destVault || busy} onClick={() => void review()}>
                {busy && <Loader2 className="animate-spin" />}
                Revisar
              </Button>
            </>
          )}

          {step === 'revisao' && plan && (
            <>
              <Button variant="outline" onClick={() => setStep('destino')}>
                Voltar
              </Button>
              <Button disabled={!plan.checks.ok || chosen.length === 0} onClick={start}>
                Copiar {chosen.length} {chosen.length === 1 ? 'registro' : 'registros'}
              </Button>
            </>
          )}

          {step === 'copiando' && (
            <>
              {stoppedAt ? (
                <>
                  <Button variant="outline" onClick={() => setStep('resumo')}>
                    Encerrar aqui
                  </Button>
                  <Button onClick={resume}>Continuar de onde parou</Button>
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
                    Parar
                  </Button>
                  <Button disabled>
                    <Loader2 className="animate-spin" />
                    Copiando…
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
                Parar aqui
              </Button>
              <Button
                disabled={choice === 'rename' && (!newTitle.trim() || (!!conflict?.item.name && !newName.trim()))}
                onClick={() => void decide()}
              >
                Continuar
              </Button>
            </>
          )}

          {step === 'resumo' && <Button onClick={onDone}>Concluir</Button>}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function Steps({ step }: { step: Step }) {
  const order: Array<[Step[], string]> = [
    [['destino'], 'Destino'],
    [['revisao'], 'Revisão'],
    [['copiando', 'conflito'], 'Cópia'],
    [['resumo'], 'Resumo'],
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
        ? 'é o próprio'
        : reason === 'repeated'
          ? 'nome repetido'
          : item.conflict?.read_only
            ? 'já existe, String'
            : 'já existe'}
    </span>
  )
}
