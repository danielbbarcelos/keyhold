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
import { t } from '../i18n'
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
 *
 * Few steps on purpose: nobody weighs two hours against four, and a list that needs scrolling hides
 * its longest windows, which are the ones somebody came looking for. Thirty days is the daemon's own
 * ceiling for a timed window.
 */
const windows = (): Array<{ value: string; label: string; window: GrantWindow }> => [
  { value: 'once', label: t('Somente desta vez'), window: { kind: 'once' } },
  { value: '15', label: t('15 minutos'), window: { kind: 'minutes', minutes: 15 } },
  { value: '60', label: t('1 hora'), window: { kind: 'minutes', minutes: 60 } },
  { value: '480', label: t('8 horas'), window: { kind: 'minutes', minutes: 480 } },
  { value: '1440', label: t('24 horas'), window: { kind: 'minutes', minutes: 1440 } },
  { value: '10080', label: t('7 dias'), window: { kind: 'minutes', minutes: 10080 } },
  { value: '21600', label: t('15 dias'), window: { kind: 'minutes', minutes: 21600 } },
  { value: '43200', label: t('30 dias'), window: { kind: 'minutes', minutes: 43200 } },
  { value: 'always', label: t('Para sempre'), window: { kind: 'always' } },
]

export function AgentApproval({ onAnswered }: { onAnswered: () => void }) {
  // Every request waiting from the same process for the same record, answered as one question.
  // An assistant that needs host, user and password asks for them in a burst, and four dialogs in
  // a row for one decision is how people learn to click "Permitir" without reading.
  const [group, setGroup] = useState<AgentRequest[]>([])
  const [window_, setWindow] = useState('once')
  const [everyField, setEveryField] = useState(false)
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
          title: t('Keyhold: pedido de um assistente'),
          body: t('{who} pede acesso a {record}. Abra o Keyhold para permitir ou negar.', {
            who: next.peer.parent ?? next.peer.process,
            record: next.record_title,
          }),
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

  const WINDOWS = windows()
  const choices = [
    WINDOWS[0],
    // Not offered for a process started by the system itself (PID 1): that one never closes, and
    // "while it is open" would quietly mean "until you log out".
    ...(request.peer.parent_pid !== null && request.peer.parent_pid > 1
      ? [
          {
            value: 'process',
            label: t('Enquanto {who} estiver aberto', { who: request.peer.parent ?? request.peer.process }),
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
  const copies = group.some((one) => one.intent === 'copy')
  const reads = group.some((one) => one.intent === 'get')
  const kinds = [runs, copies, reads].filter(Boolean).length
  const commands = group.filter((one) => one.command && one.command.length > 0)
  const fields = [...new Set(group.map((one) => one.field))]
  const who = request.peer.parent ?? request.peer.process

  return (
    <AlertDialog open>
      <AlertDialogContent className="max-h-[calc(100dvh-2rem)] overflow-y-auto data-[size=default]:max-w-[calc(100%-2rem)] data-[size=default]:sm:max-w-md">
        <AlertDialogHeader className="min-w-0">
          <AlertDialogTitle className="break-words">
            {asks(who, kinds > 1 ? 'access' : runs ? 'use' : copies ? 'copy' : 'read', fields.length)}
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
                    {request.peer.parent
                      ? t('PID {pid}, pela ponte {bridge}', { pid: request.peer.pid, bridge: request.peer.process })
                      : t('PID {pid}', { pid: request.peer.pid })}
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
                        {one.intent === 'run' ? t('executar comando') : one.intent === 'copy' ? t('copiar') : t('ler valor')}
                      </span>
                    </li>
                  ))}
                </ul>
              </div>

              {commands.map((one) => (
                <div key={one.id} className="space-y-1 text-left">
                  <span className="text-[13px] font-medium text-foreground">{t('Comando com {field}', { field: one.field })}</span>
                  <pre className="max-h-32 overflow-auto rounded-xl border bg-raised p-3 font-mono text-xs break-all whitespace-pre-wrap text-foreground">
                    {one.command!.join(' ')}
                  </pre>
                </div>
              ))}

              {runs && (
                <p className="flex items-start gap-2 rounded-xl bg-amber-soft p-3 text-left text-foreground">
                  <Terminal className="mt-0.5 size-4 shrink-0 text-amber" />
                  <span>
                    {t(
                      'O comando recebe o valor e pode fazer qualquer coisa com ele, inclusive mostrar ou enviar. Leia o comando antes de permitir. O Keyhold só apaga da saída o valor escrito por extenso.',
                    )}
                  </span>
                </p>
              )}

              {copies && (
                <p className="flex items-start gap-2 rounded-xl bg-amber-soft p-3 text-left text-foreground">
                  <TriangleAlert className="mt-0.5 size-4 shrink-0 text-amber" />
                  <span>
                    {t(
                      'O valor vai para a sua área de transferência por alguns segundos. Qualquer programa aberto com o seu usuário consegue ler o que está nela, inclusive o assistente.',
                    )}
                  </span>
                </p>
              )}

              {reads && (
                <p className="flex items-start gap-2 rounded-xl bg-amber-soft p-3 text-left text-foreground">
                  <TriangleAlert className="mt-0.5 size-4 shrink-0 text-amber" />
                  <span>
                    {t(
                      'O valor de um campo comum entra na conversa do assistente. Senhas e outros campos secretos nunca são entregues por leitura.',
                    )}
                  </span>
                </p>
              )}

              <div className="space-y-1.5 pt-1 text-left">
                <span className="text-[13px] font-medium text-foreground">{t('Permitir por')}</span>

                <Popover open={picking} onOpenChange={setPicking}>
                  <PopoverTrigger asChild>
                    <Button
                      variant="outline"
                      role="combobox"
                      aria-expanded={picking}
                      aria-label={t('Por quanto tempo')}
                      className="w-full justify-between font-normal"
                    >
                      <span className="truncate">{picked.label}</span>
                      <ChevronsUpDown className="size-4 shrink-0 opacity-50" />
                    </Button>
                  </PopoverTrigger>

                  <PopoverContent className="w-(--radix-popover-trigger-width) p-0" align="start">
                    <Command>
                      <CommandInput placeholder={t('Buscar período')} />
                      <CommandList className="max-h-[min(23rem,55dvh)]">
                        <CommandEmpty>{t('Nenhum período com esse nome.')}</CommandEmpty>
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
                  {t('Todos os campos deste registro')}
                </label>

                <span className="block text-xs">
                  {describeWindow(picked.value, fields, wholeRecord)}
                  {lasting && runs ? ` ${t('Só para comandos de {programs}: outro programa pede de novo.', { programs: programs(group) })}` : ''}
                </span>
              </div>
            </div>
          </AlertDialogDescription>
        </AlertDialogHeader>

        <AlertDialogFooter>
          <Button variant="outline" onClick={() => answer(false)}>
            {t('Negar')}
          </Button>
          <Button onClick={() => answer(true)}>{t('Permitir')}</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}

/** What the chosen window actually costs, said before it is chosen rather than after. */
function describeWindow(value: string, fields: string[], wholeRecord: boolean): string {
  if (value === 'once') {
    return t('O próximo pedido exige uma nova aprovação.')
  }

  const what = wholeRecord
    ? t('todos os campos deste registro')
    : fields.length > 1
      ? t('estes campos deste registro')
      : t('"{field}" deste registro', { field: fields[0] })

  if (value === 'process') {
    return t('Vale para {what} enquanto este processo estiver em execução. Um novo processo com o mesmo PID não herda a permissão.', { what })
  }

  if (value === 'always') {
    return t('Vale para {what} até ser revogada em Ajustes, em Acesso por agente.', { what })
  }

  return t('Vale para {what}. A permissão pode ser revogada antes em Ajustes.', { what })
}

/** The dialog's title, one sentence per verb and count so each reads naturally. */
function asks(who: string, verb: 'access' | 'use' | 'copy' | 'read', count: number): string {
  const n = count
  if (count > 1) {
    if (verb === 'access') return t('{who} pede para acessar {n} campos', { who, n })
    if (verb === 'use') return t('{who} pede para usar {n} campos', { who, n })
    if (verb === 'copy') return t('{who} pede para copiar {n} campos', { who, n })
    return t('{who} pede para ler {n} campos', { who, n })
  }
  if (verb === 'access') return t('{who} pede para acessar um campo', { who })
  if (verb === 'use') return t('{who} pede para usar um campo', { who })
  if (verb === 'copy') return t('{who} pede para copiar um campo', { who })

  return t('{who} pede para ler um campo', { who })
}

/** The programs a lasting yes to these commands would cover, as the person reads them. */
function programs(group: AgentRequest[]): string {
  const names = [...new Set(group.map((one) => one.command?.[0]).filter((name): name is string => !!name))]

  return names.join(', ')
}
