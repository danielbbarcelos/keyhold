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
const ROLES: Record<
  InstanceProcess['role'],
  { label: string; what: string; icon: typeof Bot; cost: string }
> = {
  interface: {
    label: 'Interface do cofre',
    what: 'Serve uma janela e guarda a chave enquanto o cofre está aberto.',
    icon: MonitorSmartphone,
    cost: 'O cofre que ela mantém aberto tranca, a chave é esquecida, e qualquer janela apontada para ela para de responder.',
  },
  agent: {
    label: 'Ponte do assistente',
    what: 'Traduz os pedidos de um assistente local. Não guarda chave nenhuma.',
    icon: Bot,
    cost: 'O assistente perde a ponte até abrir outra — nada do cofre se perde.',
  },
  command: {
    label: 'Comando com um segredo',
    what: 'Um comando rodando com um segredo no ambiente dele.',
    icon: Terminal,
    cost: 'O comando em execução é interrompido no meio.',
  },
  other: {
    label: 'Processo do Keyhold',
    what: 'Um processo que esta versão não reconhece — provavelmente de uma build anterior.',
    icon: Square,
    cost: 'Ele é encerrado. O que estiver fazendo para no meio.',
  },
}

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
      setError(cause instanceof ApiError ? cause.message : 'Não foi possível falar com o Keyhold.')
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

  const close = async (targets: InstanceProcess[]) => {
    setBusy(true)
    setError('')

    try {
      for (const target of targets) {
        await api.closeInstance(target.pid)
      }
    } catch (cause) {
      setError(cause instanceof ApiError ? cause.message : 'Não foi possível falar com o Keyhold.')
    } finally {
      setBusy(false)
      setConfirming(null)
      await refresh()
    }
  }

  return (
    <div className="space-y-3 p-6">
      <div className="flex flex-wrap items-center gap-3">
        <p className="flex-1 text-sm text-muted-foreground">
          Um cofre, um processo. Abrir o Keyhold com outro já rodando pergunta antes de encerrá-lo —
          esta lista é para o que escapa disso, como um processo deixado por uma versão anterior.
        </p>

        {others.length > 1 && (
          <Button
            variant="destructive"
            size="sm"
            disabled={busy}
            onClick={() => setConfirming({ others })}
          >
            Encerrar as outras
          </Button>
        )}
      </div>

      {error && <p className="text-sm text-destructive">{error}</p>}

      {processes === null ? (
        <p className="py-8 text-center text-sm text-muted-foreground">Procurando…</p>
      ) : (
        <ul className="space-y-1">
          {processes.map((one) => {
            const role = ROLES[one.role]
            const Icon = role.icon

            return (
              <li
                key={one.pid}
                className={cn(
                  'flex items-start gap-3 rounded-lg border p-3',
                  one.is_this_one && 'border-primary/40 bg-primary/5',
                )}
              >
                <Icon className="mt-0.5 size-4 shrink-0 text-muted-foreground" />

                <div className="min-w-0 flex-1 space-y-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-sm font-medium">{role.label}</span>

                    {one.is_this_one && <Badge>esta janela</Badge>}
                    {one.holds_the_vault && !one.is_this_one && (
                      <Badge variant="secondary">dona do arquivo do cofre</Badge>
                    )}
                  </div>

                  <p className="text-sm text-muted-foreground">{role.what}</p>

                  <p className="text-xs text-muted-foreground tabular-nums">
                    PID {one.pid} · há {howLong(one.running_for)}
                    {one.is_this_one && ` · versão ${version}`}
                  </p>
                </div>

                {one.is_this_one ? (
                  <span className="text-xs text-muted-foreground">Trancar, no menu ao lado</span>
                ) : (
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={busy}
                    onClick={() => setConfirming(one)}
                  >
                    Encerrar
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
                ? `Encerrar ${confirming.others.length} processos?`
                : `Encerrar o PID ${confirming?.pid}?`}
            </AlertDialogTitle>

            <AlertDialogDescription asChild>
              <div className="space-y-2">
                {confirming && 'others' in confirming ? (
                  <>
                    <p>Tudo menos a instância que desenha esta janela:</p>
                    <ul className="space-y-1">
                      {confirming.others.map((one) => (
                        <li key={one.pid}>
                          <span className="text-foreground">{ROLES[one.role].label}</span> · PID{' '}
                          {one.pid} — {ROLES[one.role].cost}
                        </li>
                      ))}
                    </ul>
                  </>
                ) : (
                  confirming && <p>{ROLES[confirming.role].cost}</p>
                )}

                <p>Cada um recebe um pedido para encerrar por conta própria, nunca um tiro seco.</p>
              </div>
            </AlertDialogDescription>
          </AlertDialogHeader>

          <AlertDialogFooter>
            <AlertDialogCancel>Cancelar</AlertDialogCancel>
            <AlertDialogAction
              disabled={busy}
              onClick={(event) => {
                event.preventDefault()
                void close(confirming && 'others' in confirming ? confirming.others : confirming ? [confirming] : [])
              }}
            >
              {busy && <Loader2 className="animate-spin" />}
              Encerrar
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
