import { useEffect, useState } from 'react'
import { Bot, CircleCheck, CircleX, ShieldQuestion, User } from 'lucide-react'

import { Badge } from '@/components/ui/badge'
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
  'secret.revealed': 'Segredo revelado',
  'secret.copied': 'Segredo copiado',
  'instance.closed': 'Instância encerrada',
  'agent.granted': 'Permissão dada a um agente',
  'agent.grant.revoked': 'Permissão de agente revogada',
  'identity.protection.changed': 'Proteção do cofre alterada',
  'identity.second-factor.changed': 'Segundo fator alterado',
}

export function AuditPanel({ onOpenRecord }: { onOpenRecord: (id: string) => void }) {
  const [events, setEvents] = useState<AuditEvent[] | null>(null)
  const [integrity, setIntegrity] = useState<AuditIntegrity | null>(null)
  const [checking, setChecking] = useState(false)

  useEffect(() => {
    void api
      .audit()
      .then((answer) => setEvents(answer.events))
      .catch(() => setEvents([]))
  }, [])

  if (!events) {
    return <p className="p-6 text-sm text-muted-foreground">Lendo a trilha…</p>
  }

  return (
    <div className="space-y-4 p-6">
      <div className="flex flex-wrap items-center gap-3">
        <p className="flex-1 text-sm text-muted-foreground">
          Cada linha carrega a impressão digital da anterior, então editar ou remover uma quebra
          todas as seguintes. O que está aqui nunca inclui o valor de um segredo.
        </p>

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
      </div>

      {integrity && (
        <p
          className={cn(
            'flex items-center gap-2 rounded-lg border p-3 text-sm',
            integrity.intact ? 'border-border' : 'border-destructive/40 bg-destructive/10',
          )}
        >
          {integrity.intact ? (
            <>
              <CircleCheck className="size-4 text-primary" />
              {integrity.checked} {integrity.checked === 1 ? 'linha confere' : 'linhas conferem'}.
              Nada foi editado nem removido.
            </>
          ) : (
            <>
              <CircleX className="size-4 text-destructive" />
              A corrente quebra na linha {integrity.broken_at}. As {integrity.checked} anteriores
              conferem; dali em diante não dá para confiar no que está escrito.
            </>
          )}
        </p>
      )}

      {events.length === 0 ? (
        <p className="py-8 text-center text-sm text-muted-foreground">Nada registrado ainda.</p>
      ) : (
        <ul className="space-y-1">
          {events.map((event) => (
            <li
              key={event.seq}
              className="flex items-center gap-3 rounded-lg border px-3 py-2 text-sm"
            >
              {event.actor.kind === 'agent' ? (
                <Bot className="size-4 shrink-0 text-muted-foreground" />
              ) : (
                <User className="size-4 shrink-0 text-muted-foreground" />
              )}

              <span className="min-w-0 flex-1">
                <span className="block truncate">{LABELS[event.action] ?? event.action}</span>

                <span className="block text-xs text-muted-foreground">
                  {event.at.replace('T', ' ').replace('Z', '')}
                  {event.actor.kind === 'agent' && ` · ${event.actor.process} (${event.actor.pid})`}
                  {typeof event.detail.field === 'string' && ` · campo "${event.detail.field}"`}
                  {typeof event.detail.kind === 'string' && ` · ${event.detail.kind}`}
                </span>
              </span>

              {event.result === 'denied' && <Badge variant="destructive">negado</Badge>}

              {event.scope && (
                <Button
                  variant="ghost"
                  size="xs"
                  onClick={() => onOpenRecord(event.scope as string)}
                >
                  abrir
                </Button>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
