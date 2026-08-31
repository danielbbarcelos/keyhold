import { useEffect, useState } from 'react'
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
 */
export function AgentApproval({ onAnswered }: { onAnswered: () => void }) {
  const [request, setRequest] = useState<AgentRequest | null>(null)

  useEffect(() => {
    let stopped = false

    const poll = async () => {
      try {
        const answer = await api.agentRequests()

        if (!stopped) {
          setRequest(answer.requests[0] ?? null)
        }
      } catch {
        if (!stopped) {
          setRequest(null)
        }
      }
    }

    void poll()
    const timer = window.setInterval(() => void poll(), 1000)

    return () => {
      stopped = true
      window.clearInterval(timer)
    }
  }, [])

  if (!request) {
    return null
  }

  const answer = (allowed: boolean, minutes = 0) =>
    void api.answerAgentRequest(request.id, allowed, minutes).then(() => {
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
