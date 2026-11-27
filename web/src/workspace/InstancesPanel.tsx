import { useCallback, useEffect, useState } from 'react'
import { Bot, Loader2, MonitorSmartphone, Square, Terminal } from 'lucide-react'

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

import { api, ApiError, type InstanceProcess } from '../api'
import { t } from '../i18n'

/**
 * Every Keyhold running on this machine.
 *
 * Two of them over one vault file is a failure that hides: both open the same database, and the
 * agent socket has one fixed path, so the last one to start owns the integration while the others
 * keep an unlocked key in memory behind a window nobody is looking at. Starting a second one now
 * asks first — but a leftover from an older build claimed nothing and would never be found, so this
 * is where it becomes visible.
 *
 * Ending one is never one click. Closing the interface locks a vault somebody left open; closing
 * the bridge cuts an assistant off mid-sentence. The dialog says which of those is about to happen.
 */
const roles = (): Record<
  InstanceProcess['role'],
  { label: string; what: string; icon: typeof Bot; cost: string }
> => ({
  interface: {
    label: t('Interface do cofre'),
    what: t('Serve uma janela e mantém a chave enquanto o cofre está aberto.'),
    icon: MonitorSmartphone,
    cost: t('O cofre mantido aberto por ela é trancado, a chave é descartada e as janelas conectadas a ela deixam de responder.'),
  },
  agent: {
    label: t('Ponte do assistente'),
    what: t('Traduz os pedidos de um assistente local. Não armazena nenhuma chave.'),
    icon: Bot,
    cost: t('O assistente perde a ponte até abrir outra. O conteúdo do cofre é preservado.'),
  },
  command: {
    label: t('Comando com um segredo'),
    what: t('Um comando em execução com um segredo no ambiente.'),
    icon: Terminal,
    cost: t('O comando em execução é interrompido imediatamente.'),
  },
  other: {
    label: t('Processo do Keyhold'),
    what: t('Um processo que esta versão não reconhece, provavelmente de uma versão anterior.'),
    icon: Square,
    cost: t('O processo é encerrado e o que estiver em andamento é interrompido.'),
  },
})

export function InstancesPanel() {
  const [processes, setProcesses] = useState<InstanceProcess[] | null>(null)
  const [version, setVersion] = useState('')
  const [confirming, setConfirming] = useState<InstanceProcess | { others: InstanceProcess[] } | null>(
    null,
  )
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  const refresh = useCallback(async () => {
    try {
      const answer = await api.instances()
      setProcesses(answer.processes)
      setVersion(answer.version)
    } catch (cause) {
      setError(cause instanceof ApiError ? cause.message : t('O Keyhold não respondeu. Tente novamente.'))
    }
  }, [])

  // Processes come and go without telling this window — an assistant opens a bridge, a command
  // finishes. A list that only refreshes on open would be wrong within a minute.
  useEffect(() => {
    void refresh()
    const timer = window.setInterval(() => void refresh(), 5000)

    return () => window.clearInterval(timer)
  }, [refresh])

  const others = (processes ?? []).filter((one) => !one.is_this_one)
  const ROLES = roles()

  const close = async (targets: InstanceProcess[]) => {
    setBusy(true)
    setError('')

    try {
      for (const target of targets) {
        await api.closeInstance(target.pid)
      }
    } catch (cause) {
      setError(cause instanceof ApiError ? cause.message : t('O Keyhold não respondeu. Tente novamente.'))
    } finally {
      setBusy(false)
      setConfirming(null)
      await refresh()
    }
  }

  return (
    <div className="overflow-hidden rounded-[14px] border bg-card">
      <div className="flex flex-wrap items-center gap-3 border-b px-5 py-4">
        <div className="min-w-0 flex-1">
          <h3 className="text-[15px] font-semibold">{t('Processos do Keyhold nesta máquina')}</h3>
          <p className="mt-0.5 text-[13px] text-muted-foreground">
            {t(
              'Cada cofre usa um processo. Ao abrir o Keyhold com outro processo em execução, o Keyhold pergunta antes de encerrá-lo. Esta lista exibe os demais casos, como um processo deixado por uma versão anterior.',
            )}
          </p>
        </div>

        {others.length > 1 && (
          <Button
            variant="destructive"
            size="sm"
            disabled={busy}
            onClick={() => setConfirming({ others })}
          >
            {t('Encerrar os outros')}
          </Button>
        )}
      </div>

      {error && <p className="px-5 pt-3 text-sm text-destructive">{error}</p>}

      {processes === null ? (
        <p className="py-8 text-center text-sm text-muted-foreground">{t('Carregando…')}</p>
      ) : (
        <ul className="divide-y">
          {processes.map((one) => {
            const role = ROLES[one.role]
            const Icon = role.icon

            return (
              <li
                key={one.pid}
                className={cn(
                  'flex items-start gap-3 px-5 py-4 transition-colors',
                  one.is_this_one ? 'bg-primary-soft/40' : 'hover:bg-hover',
                )}
              >
                <Icon className="mt-0.5 size-4 shrink-0 text-muted-foreground" />

                <div className="min-w-0 flex-1 space-y-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-sm font-medium">{role.label}</span>

                    {one.is_this_one && <Badge>{t('esta janela')}</Badge>}
                    {one.holds_the_vault && !one.is_this_one && (
                      <Badge variant="secondary">{t('dona do arquivo do cofre')}</Badge>
                    )}
                  </div>

                  <p className="text-sm text-muted-foreground">{role.what}</p>

                  <p className="text-xs text-muted-foreground tabular-nums">
                    {one.is_this_one
                      ? t('PID {pid}, há {time}, versão {version}', { pid: one.pid, time: howLong(one.running_for), version })
                      : t('PID {pid}, há {time}', { pid: one.pid, time: howLong(one.running_for) })}
                  </p>
                </div>

                {one.is_this_one ? (
                  <span className="text-xs text-muted-foreground">{t('Use Trancar, no menu lateral')}</span>
                ) : (
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={busy}
                    onClick={() => setConfirming(one)}
                  >
                    {t('Encerrar')}
                  </Button>
                )}
              </li>
            )
          })}
        </ul>
      )}

      <AlertDialog open={confirming !== null} onOpenChange={(open) => !open && setConfirming(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {confirming && 'others' in confirming
                ? t('Encerrar {n} processos', { n: confirming.others.length })
                : t('Encerrar o PID {pid}', { pid: confirming?.pid ?? '' })}
            </AlertDialogTitle>

            <AlertDialogDescription asChild>
              <div className="space-y-2">
                {confirming && 'others' in confirming ? (
                  <>
                    <p>{t('Todos os processos, exceto a instância desta janela:')}</p>
                    <ul className="space-y-1">
                      {confirming.others.map((one) => (
                        <li key={one.pid}>
                          <span className="text-foreground">{ROLES[one.role].label}</span> · PID{' '}
                          {one.pid}: {ROLES[one.role].cost}
                        </li>
                      ))}
                    </ul>
                  </>
                ) : (
                  confirming && <p>{ROLES[confirming.role].cost}</p>
                )}

                <p>{t('Cada processo recebe um pedido de encerramento e finaliza por conta própria.')}</p>
              </div>
            </AlertDialogDescription>
          </AlertDialogHeader>

          <AlertDialogFooter>
            <AlertDialogCancel>{t('Cancelar')}</AlertDialogCancel>
            <AlertDialogAction
              disabled={busy}
              onClick={(event) => {
                event.preventDefault()
                void close(confirming && 'others' in confirming ? confirming.others : confirming ? [confirming] : [])
              }}
            >
              {busy && <Loader2 className="animate-spin" />}
              {t('Encerrar')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

function howLong(seconds: number): string {
  if (seconds < 60) {
    return `${seconds}s`
  }

  if (seconds < 3600) {
    return `${Math.floor(seconds / 60)} min`
  }

  if (seconds < 86400) {
    const hours = Math.floor(seconds / 3600)

    return `${hours} h ${Math.floor((seconds % 3600) / 60)} min`
  }

  return `${Math.floor(seconds / 86400)} d`
}
