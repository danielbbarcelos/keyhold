import { useEffect, useRef, useState } from 'react'
import { Bot, Check, ChevronsUpDown, KeyRound, Terminal, TriangleAlert } from 'lucide-react'

import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'

import { Checkbox } from '@/components/ui/checkbox'
import { Command, CommandEmpty, CommandInput, CommandItem, CommandList } from '@/components/ui/command'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'

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
  { value: 'once', label: 'Somente desta vez', window: { kind: 'once' } },
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
  // Every request waiting from the same process for the same record, answered as one question.
  // An assistant that needs host, user and password asks for them in a burst, and four dialogs in
  // a row for one decision is how people learn to click "Permitir" without reading.
  const [group, setGroup] = useState<AgentRequest[]>([])
  const [window_, setWindow] = useState('once')
  const [everyField, setEveryField] = useState(true)
  const [picking, setPicking] = useState(false)

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
      let waiting: AgentRequest[] = []

      try {
        const looking = document.visibilityState === 'visible' && document.hasFocus()
        const answer = await api.agentRequests(looking)
        waiting = answer.requests
      } catch {
        waiting = []
      }

      if (stopped) {
        return
      }

      const next = waiting[0] ?? null
      const together = next
        ? waiting.filter((one) => one.peer.pid === next.peer.pid && one.record_id === next.record_id)
        : []

      if (!next) {
        // Answered here, answered elsewhere, or expired. Either way the desktop should stop
        // asking about it.
        withdraw()
      } else if (next.id !== knocked.current) {
        withdraw()
        knocked.current = next.id
        showing.current = knock({
          title: 'Keyhold: pedido de um assistente',
          body: `${next.peer.parent ?? next.peer.process} pede acesso a ${next.record_title}. Abra o Keyhold para permitir ou negar.`,
          tag: `keyhold-agent-${next.id}`,
        })
      }

      setGroup(together)
    }

    void poll()
    const timer = window.setInterval(() => void poll(), 1000)

    return () => {
      stopped = true
      window.clearInterval(timer)
      withdraw()
    }
  }, [])

  const request = group[0]

  if (!request) {
    return null
  }

  const choices = [
    WINDOWS[0],
    ...(request.peer.parent_pid !== null
      ? [
          {
            value: 'process',
            label: `Enquanto ${request.peer.parent ?? request.peer.process} estiver aberto`,
            window: { kind: 'process' } as GrantWindow,
          },
        ]
      : []),
    ...WINDOWS.slice(1),
  ]

  const picked = choices.find((one) => one.value === window_) ?? choices[0]
  const lasting = picked.value !== 'once'
  const wholeRecord = lasting && everyField

  const answer = (allowed: boolean) =>
    void Promise.all(
      group.map((one) =>
        api.answerAgentRequest(one.id, allowed, allowed ? picked.window : { kind: 'once' }, allowed && wholeRecord),
      ),
    ).then(() => {
      // Take the knock back at once rather than a poll later: the question is settled, and a
      // notification for a settled question is a second chance to answer something twice.
      showing.current?.close()
      showing.current = null
      setGroup([])
      onAnswered()
    })

  const runs = group.some((one) => one.intent === 'run')
  const reads = group.some((one) => one.intent !== 'run')
  const fields = [...new Set(group.map((one) => one.field))]
  const who = request.peer.parent ?? request.peer.process

  return (
    <AlertDialog open>
      <AlertDialogContent className="data-[size=default]:max-w-[calc(100%-2rem)] data-[size=default]:sm:max-w-md">
        <AlertDialogHeader className="min-w-0">
          <AlertDialogTitle className="break-words">
            {who} pede para {runs && reads ? 'acessar' : runs ? 'usar' : 'ler'} {fields.length > 1 ? `${fields.length} campos` : 'um campo'}
          </AlertDialogTitle>

          <AlertDialogDescription asChild>
            <div className="w-full min-w-0 space-y-3">
              <div className="flex items-center gap-3 rounded-xl border bg-raised px-4 py-3 text-left">
                <span className="grid size-8 shrink-0 place-items-center rounded-lg bg-stone-soft text-muted-foreground">
                  <Bot className="size-4" />
                </span>
                <span className="min-w-0">
                  <b className="block truncate text-sm font-medium text-foreground">{who}</b>
                  <span className="block truncate text-xs">
                    PID {request.peer.pid}
                    {request.peer.parent ? `, pela ponte ${request.peer.process}` : ''}
                  </span>
                </span>
              </div>

              <div className="rounded-xl border bg-raised px-4 py-3 text-left">
                <div className="flex items-center gap-3">
                  <span className="grid size-8 shrink-0 place-items-center rounded-lg bg-sky-soft text-sky">
                    <KeyRound className="size-4" />
                  </span>
                  <b className="min-w-0 flex-1 truncate text-sm font-medium text-foreground">{request.record_title}</b>
                </div>

                <ul className="mt-2 space-y-1 pl-11">
                  {group.map((one) => (
                    <li key={one.id} className="flex min-w-0 items-center gap-2">
                      <span className="min-w-0 flex-1 truncate text-sm text-foreground">{one.field}</span>
                      <span className="shrink-0 rounded-full bg-stone-soft px-2.5 py-0.5 text-xs">
                        {one.intent === 'run' ? 'executar comando' : 'ler valor'}
                      </span>
                    </li>
                  ))}
                </ul>
              </div>

              {reads ? (
                <p className="flex items-start gap-2 rounded-xl bg-amber-soft p-3 text-left text-foreground">
                  <TriangleAlert className="mt-0.5 size-4 shrink-0 text-amber" />
                  <span>
                    O valor de um campo comum entra na conversa do assistente. Senhas e outros campos secretos nunca são
                    entregues: só podem ser usados em um comando.
                  </span>
                </p>
              ) : (
                <p className="flex items-start gap-2 rounded-xl border p-3 text-left">
                  <Terminal className="mt-0.5 size-4 shrink-0" />
                  <span>
                    O valor é enviado a uma variável de ambiente do comando e{' '}
                    <strong className="text-foreground">não entra na conversa</strong> do assistente.
                  </span>
                </p>
              )}

              <div className="space-y-1.5 pt-1 text-left">
                <span className="text-[13px] font-medium text-foreground">Permitir por</span>

                <Popover open={picking} onOpenChange={setPicking}>
                  <PopoverTrigger asChild>
                    <Button
                      variant="outline"
                      role="combobox"
                      aria-expanded={picking}
                      aria-label="Por quanto tempo"
                      className="w-full justify-between font-normal"
                    >
                      <span className="truncate">{picked.label}</span>
                      <ChevronsUpDown className="size-4 shrink-0 opacity-50" />
                    </Button>
                  </PopoverTrigger>

                  <PopoverContent className="w-(--radix-popover-trigger-width) p-0" align="start">
                    <Command>
                      <CommandInput placeholder="Buscar período" />
                      <CommandList>
                        <CommandEmpty>Nenhum período com esse nome.</CommandEmpty>
                        {choices.map((one) => (
                          <CommandItem
                            key={one.value}
                            value={one.label}
                            onSelect={() => {
                              setWindow(one.value)
                              setPicking(false)
                            }}
                          >
                            <Check className={one.value === picked.value ? 'size-4' : 'size-4 opacity-0'} />
                            <span className="truncate">{one.label}</span>
                          </CommandItem>
                        ))}
                      </CommandList>
                    </Command>
                  </PopoverContent>
                </Popover>

                <label
                  className={`flex items-center gap-2 pt-1 text-sm ${lasting ? 'text-foreground' : 'opacity-50'}`}
                >
                  <Checkbox
                    checked={wholeRecord}
                    disabled={!lasting}
                    onCheckedChange={(on) => setEveryField(on === true)}
                  />
                  Todos os campos deste registro
                </label>

                <span className="block text-xs">{describeWindow(picked.value, fields, wholeRecord)}</span>
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
function describeWindow(value: string, fields: string[], wholeRecord: boolean): string {
  if (value === 'once') {
    return 'O próximo pedido exige uma nova aprovação.'
  }

  const what = wholeRecord
    ? 'todos os campos deste registro'
    : fields.length > 1
      ? 'estes campos deste registro'
      : `"${fields[0]}" deste registro`

  if (value === 'process') {
    return `Vale para ${what} enquanto este processo estiver em execução. Um novo processo com o mesmo PID não herda a permissão.`
  }

  if (value === 'always') {
    return `Vale para ${what} até ser revogada em Ajustes, em Acesso por agente.`
  }

  return `Vale para ${what}. A permissão pode ser revogada antes em Ajustes.`
}
