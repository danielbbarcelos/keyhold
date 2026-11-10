import { useEffect, useRef, useState } from 'react'
import { Bot, KeyRound, Terminal, TriangleAlert } from 'lucide-react'

import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'

import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'

import { api, type AgentRequest, type GrantWindow } from '../api'
import { knock } from '../notify'

/**
 * The question an agent's request becomes.
 *
 * It names the process and the PID, because "something asked for your password" is not a question
 * anybody can answer. The two intents are told apart in the strongest terms the interface has:
 * `run` keeps the value out of the assistant's conversation, `get` does not, and the difference is
 * the whole reason the tool descriptions push towards the first.
 *
 * Polled rather than pushed. The daemon times a request out on its own, so a closed window cannot
 * leave an agent hanging — or leave a question to be answered by whoever opens the window next.
 *
 * A request also knocks on the desktop, because the person driving the assistant is usually looking
 * at a terminal rather than at this window, and ninety seconds of not noticing is a request that
 * expires for no reason. The knock carries no secret — see `notify.ts`.
 */
/**
 * How long a yes lasts.
 *
 * Five minutes used to be the only answer, and it was the wrong one: an assistant that has to be
 * re-approved every few minutes is one somebody turns off, and a permission nobody is willing to
 * grant protects nothing. The list runs from "just this once" to "for good", and the two ends are
 * genuinely different decisions rather than the same one at different lengths.
 *
 * `process` is the one worth reaching for. It ends when the assistant does, which is the window
 * somebody actually means when they say "stop asking me during this session".
 */
const WINDOWS: Array<{ value: string; label: string; window: GrantWindow }> = [
  { value: 'once', label: 'Só desta vez', window: { kind: 'once' } },
  { value: '15', label: '15 minutos', window: { kind: 'minutes', minutes: 15 } },
  { value: '30', label: '30 minutos', window: { kind: 'minutes', minutes: 30 } },
  { value: '60', label: '1 hora', window: { kind: 'minutes', minutes: 60 } },
  { value: '120', label: '2 horas', window: { kind: 'minutes', minutes: 120 } },
  { value: '240', label: '4 horas', window: { kind: 'minutes', minutes: 240 } },
  { value: '480', label: '8 horas', window: { kind: 'minutes', minutes: 480 } },
  { value: '720', label: '12 horas', window: { kind: 'minutes', minutes: 720 } },
  { value: '1440', label: '24 horas', window: { kind: 'minutes', minutes: 1440 } },
  { value: '10080', label: '7 dias', window: { kind: 'minutes', minutes: 10080 } },
  { value: '43200', label: '30 dias', window: { kind: 'minutes', minutes: 43200 } },
  { value: 'always', label: 'Para sempre', window: { kind: 'always' } },
]

export function AgentApproval({ onAnswered }: { onAnswered: () => void }) {
  const [request, setRequest] = useState<AgentRequest | null>(null)
  const [window_, setWindow] = useState('once')

  // The knock currently on the desktop, and the request it belongs to. Both are refs rather than
  // state: they are bookkeeping for something outside React, and changing them must not re-render.
  const knocked = useRef<string | null>(null)
  const showing = useRef<Notification | null>(null)

  useEffect(() => {
    let stopped = false

    const withdraw = () => {
      showing.current?.close()
      showing.current = null
      knocked.current = null
    }

    const poll = async () => {
      let next: AgentRequest | null = null

      try {
        const looking = document.visibilityState === 'visible' && document.hasFocus()
        const answer = await api.agentRequests(looking)
        next = answer.requests[0] ?? null
      } catch {
        next = null
      }

      if (stopped) {
        return
      }

      if (!next) {
        // Answered here, answered elsewhere, or expired. Either way the desktop should stop
        // asking about it.
        withdraw()
      } else if (next.id !== knocked.current) {
        withdraw()
        knocked.current = next.id
        showing.current = knock({
          title: 'Keyhold: um assistente está pedindo um segredo',
          body: `${next.peer.parent ?? next.peer.process} quer ${next.field} de ${next.record_title}. Abra o Keyhold para decidir.`,
          tag: `keyhold-agent-${next.id}`,
        })
      }

      setRequest(next)
    }

    void poll()
    const timer = window.setInterval(() => void poll(), 1000)

    return () => {
      stopped = true
      window.clearInterval(timer)
      withdraw()
    }
  }, [])

  if (!request) {
    return null
  }

  const chosen =
    window_ === 'process'
      ? ({ kind: 'process' } as GrantWindow)
      : (WINDOWS.find((one) => one.value === window_)?.window ?? { kind: 'once' })

  const answer = (allowed: boolean) =>
    void api.answerAgentRequest(request.id, allowed, allowed ? chosen : { kind: 'once' }).then(() => {
      // Take the knock back at once rather than a poll later: the question is settled, and a
      // notification for a settled question is a second chance to answer something twice.
      showing.current?.close()
      showing.current = null
      setRequest(null)
      onAnswered()
    })

  const runs = request.intent === 'run'

  return (
    <AlertDialog open>
      <AlertDialogContent className="sm:max-w-[440px]">
        <AlertDialogHeader>
          <AlertDialogTitle>
            {request.peer.parent ?? request.peer.process} quer {runs ? 'usar' : 'ler'} um segredo
          </AlertDialogTitle>

          <AlertDialogDescription asChild>
            <div className="space-y-3">
              <div className="flex items-center gap-3 rounded-xl border bg-raised px-4 py-3 text-left">
                <span className="grid size-8 place-items-center rounded-lg bg-stone-soft text-muted-foreground">
                  <Bot className="size-4" />
                </span>
                <span className="min-w-0">
                  <b className="block truncate text-sm font-medium text-foreground">{request.peer.parent ?? request.peer.process}</b>
                  <span className="block truncate text-xs">
                    pid {request.peer.pid}
                    {request.peer.parent ? `, pela ponte ${request.peer.process}` : ''}
                  </span>
                </span>
              </div>

              <div className="flex items-center gap-3 rounded-xl border bg-raised px-4 py-3 text-left">
                <span className="grid size-8 place-items-center rounded-lg bg-sky-soft text-sky">
                  <KeyRound className="size-4" />
                </span>
                <span className="min-w-0 flex-1">
                  <b className="block truncate text-sm font-medium text-foreground">
                    {request.record_title}, {request.field}
                  </b>
                </span>
                <span className="rounded-full bg-stone-soft px-2.5 py-0.5 text-xs">{runs ? 'rodar comando' : 'ler o valor'}</span>
              </div>

              {runs ? (
                <p className="flex items-start gap-2 rounded-xl border p-3 text-left">
                  <Terminal className="mt-0.5 size-4 shrink-0" />
                  <span>
                    O valor vai para uma variável de ambiente de um comando e{' '}
                    <strong className="text-foreground">não entra na conversa</strong> do assistente.
                  </span>
                </p>
              ) : (
                <p className="flex items-start gap-2 rounded-xl bg-amber-soft p-3 text-left text-foreground">
                  <TriangleAlert className="mt-0.5 size-4 shrink-0 text-amber" />
                  <span>
                    Um valor entregue ao assistente sai desta máquina. Se der para resolver rodando um comando com a senha
                    no ambiente, prefira isso.
                  </span>
                </p>
              )}

              <div className="space-y-1.5 pt-1 text-left">
                <span className="text-[13px] font-medium text-foreground">Permitir por</span>
                <Select value={window_} onValueChange={setWindow}>
                  <SelectTrigger className="w-full" aria-label="Por quanto tempo">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="once">Só desta vez</SelectItem>

                    {request.peer.parent_pid !== null && (
                      <SelectItem value="process">
                        Enquanto {request.peer.parent ?? request.peer.process} estiver aberto
                      </SelectItem>
                    )}

                    {WINDOWS.filter((one) => one.value !== 'once').map((one) => (
                      <SelectItem key={one.value} value={one.value}>
                        {one.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>

                <span className="block text-xs">
                  {describeWindow(window_, request)}
                </span>
              </div>
            </div>
          </AlertDialogDescription>
        </AlertDialogHeader>

        <AlertDialogFooter>
          <Button variant="outline" onClick={() => answer(false)}>
            Negar
          </Button>
          <Button onClick={() => answer(true)}>Permitir</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}

/** What the chosen window actually costs, said before it is chosen rather than after. */
function describeWindow(value: string, request: AgentRequest): string {
  if (value === 'once') {
    return 'O próximo pedido deste campo pergunta de novo.'
  }

  if (value === 'process') {
    return `Vale só enquanto esse processo viver. Fechou, acabou — e um processo novo com o mesmo PID não herda nada.`
  }

  if (value === 'always') {
    return `Fica valendo até você revogar em Ajustes → Acesso por agente. Vale para "${request.field}" deste registro, e só.`
  }

  return `Vale para "${request.field}" deste registro, e para nenhum outro campo. Dá para revogar antes em Ajustes.`
}
