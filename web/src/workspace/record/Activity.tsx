import { useEffect, useState } from 'react'
import { Laptop, Loader2 } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { t } from '@/i18n'

import { api, ApiError, type Activity as ActivityAnswer, type ActivityEvent } from '../../api'
import { kindOf, momentOf, whenShort } from '../activity'
import { AwsMark } from '../AwsMark'

/**
 * The record's own events: in an AWS vault, from the account's trail (who is the person's name in
 * Identity Center, when is when AWS received it); in a local vault, from this machine's Trilha.
 */
export function Activity({ recordId }: { recordId: string }) {
  const [answer, setAnswer] = useState<ActivityAnswer | null>(null)
  const [events, setEvents] = useState<ActivityEvent[]>([])
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(false)

  useEffect(() => {
    let alive = true
    setAnswer(null)
    setEvents([])
    setError('')

    api
      .activity(recordId)
      .then((found) => {
        if (!alive) return
        setAnswer(found)
        setEvents(found.events)
      })
      .catch((cause) => alive && setError(cause instanceof ApiError ? cause.message : t('Não foi possível ler a atividade.')))

    return () => {
      alive = false
    }
  }, [recordId])

  async function more() {
    const oldest = events[events.length - 1]?.at_ms
    if (!oldest) return

    setLoading(true)
    try {
      const page = await api.activity(recordId, oldest - 1)
      setEvents((current) => [...current, ...page.events])
      setAnswer((current) => (current ? { ...current, more: page.more } : page))
    } catch (cause) {
      setError(cause instanceof ApiError ? cause.message : t('Não foi possível ler a atividade.'))
    } finally {
      setLoading(false)
    }
  }

  if (error) return <p className="px-5 py-6 text-sm text-rose">{error}</p>
  if (!answer) return <p className="px-5 py-6 text-sm text-faint">{t('Lendo a atividade…')}</p>

  const aws = answer.source === 'aws'

  if (aws && (answer.denied || answer.missing)) {
    return (
      <p className="px-5 py-6 text-sm text-muted-foreground">
        {answer.denied
          ? t('Sua permissão na AWS não inclui ler a trilha deste cofre. Peça a quem administra a conta a política audit-read.')
          : t('O log group {group} não existe nesta conta, então não há trilha para mostrar.', { group: answer.group ?? '' })}
      </p>
    )
  }

  return (
    <div>
      <div className="flex items-center gap-3 border-b px-5 py-3 text-sm">
        {aws ? <AwsMark size="xs" className="size-4" /> : <Laptop className="size-4 text-faint" />}
        <span className="font-medium">{aws ? t('Na AWS') : t('Neste computador')}</span>
        {aws && <span className="text-faint">{t('últimos {n} dias', { n: answer.days ?? 30 })}</span>}
        <span className="flex-1" />
        {answer.group && <span className="font-mono text-[11px] text-faint">{answer.group}</span>}
      </div>

      {events.map((event, index) => {
        const kind = kindOf(event.action)
        const Icon = kind.icon

        return (
          <div key={`${event.at_ms ?? event.at}-${index}`} className="flex items-center gap-3 border-b px-5 py-3 text-sm">
            <span className="grid size-8 shrink-0 place-items-center rounded-lg bg-stone-soft text-muted-foreground">
              <Icon className="size-4" />
            </span>
            <span className="min-w-0 flex-1 text-muted-foreground">
              <b className="font-medium text-foreground">{event.person ?? t('Você')}</b> {kind.sentence(event.detail)}
            </span>
            <span className="shrink-0 text-xs whitespace-nowrap text-faint">{whenShort(momentOf(event))}</span>
          </div>
        )
      })}

      {events.length === 0 && (
        <p className="border-b px-5 py-6 text-sm text-faint">
          {aws
            ? t('Nada registrado nos últimos {n} dias.', { n: answer.days ?? 30 })
            : t('Nada registrado para este registro neste computador.')}
        </p>
      )}

      {answer.more && (
        <div className="border-b px-5 py-3">
          <Button variant="outline" size="sm" disabled={loading} onClick={() => void more()}>
            {loading && <Loader2 className="animate-spin" />}
            {t('Carregar mais')}
          </Button>
        </div>
      )}

      <p className="px-5 py-4 text-[13px] text-faint">
        {aws
          ? t('A hora é a de quando a AWS recebeu cada evento. O que foi feito fora do Keyhold, pelo console da AWS ou pela CLI, fica no AWS CloudTrail da conta.')
          : t('Revelações e cópias só aparecem aqui quando a Trilha registra leituras (Ajustes, Trilha).')}
      </p>
    </div>
  )
}
