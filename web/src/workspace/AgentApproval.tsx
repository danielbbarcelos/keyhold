import { useEffect, useRef, useState } from 'react'
import { Bot, Terminal } from 'lucide-react'

import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'

import { api, type AgentRequest } from '../api'
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
export function AgentApproval({ onAnswered }: { onAnswered: () => void }) {
  const [request, setRequest] = useState<AgentRequest | null>(null)

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
        const answer = await api.agentRequests()
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
          title: 'Keyhold — um assistente está pedindo um segredo',
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

  const answer = (allowed: boolean, minutes = 0) =>
    void api.answerAgentRequest(request.id, allowed, minutes).then(() => {
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
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle className="flex items-center gap-2">
            <Bot className="size-4" />
            {request.peer.parent ?? request.peer.process} está pedindo um segredo
          </AlertDialogTitle>

          <AlertDialogDescription asChild>
            <div className="space-y-3">
              <p>
                Campo <strong className="text-foreground">{request.field}</strong> de{' '}
                <strong className="text-foreground">{request.record_title}</strong>.
              </p>

              {runs ? (
                <p className="flex items-start gap-2 rounded-lg border p-3">
                  <Terminal className="mt-0.5 size-4 shrink-0" />
                  <span>
                    O valor vai para uma variável de ambiente de um comando e{' '}
                    <strong className="text-foreground">não entra na conversa</strong> do
                    assistente.
                  </span>
                </p>
              ) : (
                <p className="rounded-lg border border-destructive/40 bg-destructive/10 p-3">
                  O valor entra no contexto do modelo e{' '}
                  <strong className="text-foreground">sai desta máquina</strong>. Se der para
                  resolver executando um comando, prefira isso.
                </p>
              )}

              <p className="text-xs">
                {request.peer.parent
                  ? `${request.peer.parent} → ${request.peer.process}`
                  : request.peer.process}{' '}
                · PID {request.peer.pid}
              </p>
            </div>
          </AlertDialogDescription>
        </AlertDialogHeader>

        <AlertDialogFooter>
          <Button variant="ghost" onClick={() => answer(false)}>
            Negar
          </Button>
          <Button variant="outline" onClick={() => answer(true, 5)}>
            Permitir por 5 min
          </Button>
          <Button onClick={() => answer(true, 0)}>Permitir uma vez</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
