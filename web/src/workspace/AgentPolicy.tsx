import { useState } from 'react'
import { Calendar, Clock, Infinity as Forever, MessageCircleQuestion } from 'lucide-react'

import { DatePicker } from '@/components/common/DatePicker'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Select, SelectContent, SelectItem, SelectSeparator, SelectTrigger, SelectValue } from '@/components/ui/select'

import { api, type AgentPolicy, type VaultSummary } from '../api'
import { locale, t } from '../i18n'
import { toastError } from './toast'

const DAY = 24 * 60 * 60

const moment = (at: number) =>
  new Date(at * 1000).toLocaleString(locale(), { dateStyle: 'short', timeStyle: 'short' })
const day = (at: number) => new Date(at * 1000).toLocaleDateString(locale(), { day: '2-digit', month: '2-digit' })

/** Whether the vault lets an agent through right now. */
export function letsThrough(policy?: AgentPolicy): boolean {
  return policy?.mode === 'always' || (policy?.mode === 'until' && policy.at * 1000 > Date.now())
}

/**
 * What an agent asking in this vault gets: asked every time, or let through for a while or for
 * good. Next to the vault's switch in Ajustes → Agentes; only drawn when the switch is on.
 */
export function AgentPolicySelect({ vault, onChanged }: { vault: VaultSummary; onChanged: () => void }) {
  const policy = vault.agent_policy ?? { mode: 'ask' }
  const expired = policy.mode === 'until' && policy.at * 1000 <= Date.now()
  const [picking, setPicking] = useState(false)
  const [date, setDate] = useState('')

  const save = (agent: AgentPolicy) =>
    void api
      .vaultPrefs(vault.id, { agent })
      .then(onChanged)
      .catch((cause) => toastError(cause, t('Não foi possível salvar a liberação')))

  const value = expired ? 'ask' : policy.mode === 'until' ? 'until' : policy.mode

  const choose = (choice: string) => {
    const now = Math.floor(Date.now() / 1000)
    if (choice === 'ask') save({ mode: 'ask' })
    else if (choice === 'always') save({ mode: 'always' })
    else if (choice === 'date') setPicking(true)
    else if (choice.startsWith('days:')) save({ mode: 'until', at: now + Number(choice.slice(5)) * DAY })
  }

  return (
    <>
      <Select value={value} onValueChange={choose}>
        <SelectTrigger size="sm" className="w-[210px]" aria-label={t('Liberação do cofre {name}', { name: vault.name })}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent position="popper" align="end" className="w-[260px]">
          <SelectItem value="ask">
            <MessageCircleQuestion />
            {t('Sempre perguntar')}
          </SelectItem>
          {value === 'until' && policy.mode === 'until' && (
            <SelectItem value="until">
              <Clock className="text-mint" />
              {t('Liberado até {day}', { day: day(policy.at) })}
            </SelectItem>
          )}
          <SelectItem value="days:1">
            <Clock />
            {t('Liberar por 1 dia')}
          </SelectItem>
          <SelectItem value="days:7">
            <Clock />
            {t('Liberar por 7 dias')}
          </SelectItem>
          <SelectItem value="days:30">
            <Clock />
            {t('Liberar por 30 dias')}
          </SelectItem>
          <SelectItem value="date">
            <Calendar />
            {t('Liberar até uma data…')}
          </SelectItem>
          <SelectItem value="always">
            <Forever className="text-mint" />
            {t('Liberar sem prazo')}
          </SelectItem>
          <SelectSeparator />
          <p className="px-2 py-1.5 text-xs text-muted-foreground">
            {t('Liberado, o assistente lê, copia e roda comandos com qualquer registro deste cofre sem perguntar. Tudo fica na trilha.')}
          </p>
        </SelectContent>
      </Select>

      <Dialog open={picking} onOpenChange={setPicking}>
        <DialogContent className="sm:max-w-sm">
          <DialogHeader>
            <DialogTitle>{t('Liberar {name} até', { name: vault.name })}</DialogTitle>
            <DialogDescription>{t('Até o fim do dia escolhido o assistente não pergunta neste cofre.')}</DialogDescription>
          </DialogHeader>
          <DatePicker value={date} placeholder={t('dd/mm/aaaa')} onChange={setDate} />
          <DialogFooter>
            <Button variant="outline" onClick={() => setPicking(false)}>
              {t('Cancelar')}
            </Button>
            <Button
              disabled={!date}
              onClick={() => {
                const [year, month, dayOf] = date.split('-').map(Number)
                const end = new Date(year, month - 1, dayOf, 23, 59, 59).getTime() / 1000
                if (end * 1000 <= Date.now()) {
                  toastError(new Error(t('A data precisa estar no futuro.')), t('Não foi possível salvar a liberação'))
                  return
                }
                setPicking(false)
                setDate('')
                save({ mode: 'until', at: Math.floor(end) })
              }}
            >
              {t('Liberar')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}

/** The line under a vault's name that says how long it is let through, or that it ran out. */
export function AgentPolicyLine({ vault }: { vault: VaultSummary }) {
  const policy = vault.agent_policy
  if (!vault.agent_access || !policy) return null

  if (policy.mode === 'until' && policy.at * 1000 <= Date.now()) {
    return (
      <span className="mt-1 block text-xs text-amber">
        {t('A liberação venceu em {when} e voltou a perguntar.', { when: moment(policy.at) })}
      </span>
    )
  }
  if (policy.mode === 'until') {
    return <span className="mt-1 block text-xs text-faint">{t('sem perguntar até {when}', { when: moment(policy.at) })}</span>
  }
  if (policy.mode === 'always') {
    return <span className="mt-1 block text-xs text-faint">{t('sem perguntar, sem prazo')}</span>
  }
  return null
}
