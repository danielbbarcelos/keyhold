import { useEffect, useState } from 'react'
import { Bot, CircleCheck, CircleX, ShieldQuestion, User } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

import { api, type AuditEvent, type AuditIntegrity } from '../api'

/**
 * The trail.
 *
 * Cryptography says what somebody *can* open; this is the only record of what they *did*. It is
 * also the only part of the file keyhold can read without a key, which is deliberate — a log you
 * need the vault open to inspect is a log you cannot consult when the vault is the problem.
 *
 * The integrity check is offered rather than run on load: recomputing a chain of thousands of rows
 * is work, and it answers a question somebody asks on purpose ("has this been edited?") rather than
 * one that needs answering every time the screen opens.
 */
const LABELS: Record<string, string> = {
  'vault.unlock': 'Cofre destrancado',
  'vault.unlock.failed': 'Tentativa de destrancar falhou',
  'vault.lock': 'Cofre trancado',
  'vault.created': 'Cofre criado',
  'vault.deleted': 'Cofre excluído',
  'vault.rotated': 'Chave do cofre trocada',
  'vault.exported': 'Cofre exportado',
  'vault.imported': 'Registros importados',
  'record.created': 'Registro criado',
  'record.updated': 'Registro alterado',
  'record.deleted': 'Registro excluído',
  'record.destroyed': 'Registro apagado para sempre',
  'record.restored': 'Registro restaurado',
  'record.copied': 'Registro copiado de outro cofre',
  'secret.revealed': 'Segredo revelado',
  'secret.copied': 'Segredo copiado',
  'instance.closed': 'Instância encerrada',
  'agent.granted': 'Permissão dada a um agente',
  'agent.grant.revoked': 'Permissão de agente revogada',
  'identity.protection.changed': 'Proteção do cofre alterada',
  'identity.second-factor.changed': 'Segundo fator alterado',
}

const GROUPS: Array<{ id: string; label: string; test: (event: AuditEvent) => boolean }> = [
  { id: 'tudo', label: 'Tudo', test: () => true },
  { id: 'leituras', label: 'Leituras', test: (event) => event.action.startsWith('secret.') },
  { id: 'alteracoes', label: 'Alterações', test: (event) => event.action.startsWith('record.') || event.action.startsWith('vault.') && !event.action.startsWith('vault.unlock') && event.action !== 'vault.lock' },
  { id: 'agente', label: 'Agente', test: (event) => event.actor.kind === 'agent' || event.action.startsWith('agent.') },
  { id: 'seguranca', label: 'Segurança', test: (event) => event.action.startsWith('vault.unlock') || event.action === 'vault.lock' || event.action.startsWith('identity.') || event.action === 'instance.closed' },
]

const SHORT: Record<string, string> = {
  'secret.copied': 'Copiou',
  'secret.revealed': 'Revelou',
  'vault.unlock': 'Destrancou',
  'vault.unlock.failed': 'Senha errada',
  'vault.lock': 'Trancou',
  'record.created': 'Criou',
  'record.updated': 'Alterou',
  'record.moved': 'Moveu',
  'record.deleted': 'Excluiu',
  'record.destroyed': 'Apagou de vez',
  'record.restored': 'Restaurou',
  'record.copied': 'Copiou',
  'agent.granted': 'Permitiu',
  'agent.grant.revoked': 'Revogou',
}

/**
 * `names` turns a record id into "Cofre, Título" for the "Onde" column; the trail itself only keeps
 * the id, and the names live in the sealed payloads.
 */
export function AuditPanel({
  names,
  onOpenRecord,
}: {
  names: Record<string, string>
  onOpenRecord: (id: string) => void
}) {
  const [events, setEvents] = useState<AuditEvent[] | null>(null)
  const [integrity, setIntegrity] = useState<AuditIntegrity | null>(null)
  const [checking, setChecking] = useState(false)
  const [group, setGroup] = useState('tudo')

  useEffect(() => {
    void api
      .audit(500)
      .then((answer) => setEvents(answer.events))
      .catch(() => setEvents([]))
  }, [])

  const shown = (events ?? []).filter(GROUPS.find((one) => one.id === group)!.test)

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden rounded-[14px] border bg-card">
      <div className="flex flex-wrap items-center gap-3 border-b px-5 py-3.5">
        <div className="flex rounded-lg border bg-raised p-0.5">
          {GROUPS.map((one) => (
            <button
              key={one.id}
              type="button"
              aria-pressed={group === one.id}
              onClick={() => setGroup(one.id)}
              className={cn(
                'h-7 rounded-md px-2.5 text-[13px] font-medium text-muted-foreground hover:text-foreground',
                group === one.id && 'bg-card text-foreground shadow-sm',
              )}
            >
              {one.label}
            </button>
          ))}
        </div>
        <span className="flex-1" />
        {integrity ? (
          <span
            className={cn(
              'inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-medium',
              integrity.intact ? 'bg-mint-soft text-mint' : 'bg-rose-soft text-rose',
            )}
          >
            {integrity.intact ? <CircleCheck className="size-3.5" /> : <CircleX className="size-3.5" />}
            {integrity.intact
              ? `Corrente íntegra, ${integrity.checked} ${integrity.checked === 1 ? 'linha' : 'linhas'}`
              : `A corrente quebra na linha ${integrity.broken_at}`}
          </span>
        ) : (
          <Button
            variant="outline"
            size="sm"
            disabled={checking}
            onClick={() => {
              setChecking(true)
              void api
                .auditIntegrity()
                .then((answer) => setIntegrity(answer.integrity))
                .finally(() => setChecking(false))
            }}
          >
            <ShieldQuestion />
            Conferir a corrente
          </Button>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        {!events ? (
          <p className="p-6 text-sm text-faint">Lendo a trilha…</p>
        ) : shown.length === 0 ? (
          <p className="px-5 py-10 text-center text-sm text-faint">Nada registrado aqui ainda.</p>
        ) : (
          <table className="w-full text-sm">
            <thead className="sticky top-0 bg-card">
              <tr className="border-b text-left text-xs text-faint">
                <th className="px-5 py-2.5 font-normal">Hora</th>
                <th className="px-3 py-2.5 font-normal">Evento</th>
                <th className="px-3 py-2.5 font-normal">Quem</th>
                <th className="px-3 py-2.5 font-normal">Onde</th>
              </tr>
            </thead>
            <tbody>
              {shown.map((event) => {
                const at = new Date(event.at.endsWith('Z') ? event.at : `${event.at}Z`)
                const today = at.toDateString() === new Date().toDateString()
                const where = [event.scope ? names[event.scope] : '', typeof event.detail.field === 'string' ? event.detail.field : '']
                  .filter(Boolean)
                  .join(', ')

                return (
                  <tr key={event.seq} className="border-b last:border-b-0">
                    <td className="px-5 py-3 font-mono text-[13px] whitespace-nowrap text-muted-foreground">
                      {today ? at.toLocaleTimeString('pt-BR') : at.toLocaleString('pt-BR', { dateStyle: 'short', timeStyle: 'short' })}
                    </td>
                    <td className="px-3 py-3">
                      <span
                        title={LABELS[event.action] ?? event.action}
                        className={cn(
                          'inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-xs font-medium',
                          event.result === 'denied' || event.action.endsWith('failed')
                            ? 'bg-rose-soft text-rose'
                            : event.action === 'agent.granted'
                              ? 'bg-mint-soft text-mint'
                              : event.actor.kind === 'agent'
                                ? 'bg-amber-soft text-amber'
                                : 'bg-stone-soft text-muted-foreground',
                        )}
                      >
                        {event.result === 'denied' ? 'Negado: ' : ''}
                        {SHORT[event.action] ?? LABELS[event.action] ?? event.action}
                      </span>
                    </td>
                    <td className="px-3 py-3">
                      {event.actor.kind === 'agent' ? (
                        <span className="inline-flex items-center gap-1.5">
                          <Bot className="size-3.5 text-faint" />
                          {event.actor.process} (pid {event.actor.pid})
                        </span>
                      ) : (
                        <span className="inline-flex items-center gap-1.5">
                          <User className="size-3.5 text-faint" />
                          Você
                        </span>
                      )}
                    </td>
                    <td className="px-3 py-3 text-muted-foreground">
                      {event.scope && names[event.scope] ? (
                        <button type="button" className="hover:text-foreground hover:underline" onClick={() => onOpenRecord(event.scope as string)}>
                          {where}
                        </button>
                      ) : (
                        where || (event.scope ? 'registro que não existe mais' : '')
                      )}
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        )}
      </div>
    </div>
  )
}
