import { useState } from 'react'
import { Loader2, RotateCw, ShieldOff } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'

import { api, ApiError, type AuditSummary } from '../../api'

/**
 * "sem trilha na AWS", next to an AWS vault's name: shown whenever any permission boundary of the
 * vault has no trail in its account, so nobody assumes they are being recorded when they are not.
 * The popover says why, per log group, and asks the account again.
 */
export function AuditBadge({
  vaultId,
  audit,
  onChange,
}: {
  vaultId: string
  audit: AuditSummary | null
  onChange: (audit: AuditSummary) => void
}) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  const off = (audit?.groups ?? []).filter((group) => group.state === 'off')
  if (!audit || off.length === 0) return null

  async function retry() {
    setBusy(true)
    setError('')
    try {
      const answer = await api.retryAudit(vaultId)
      onChange(answer.audit)
    } catch (cause) {
      setError(cause instanceof ApiError ? cause.message : 'Não foi possível consultar a AWS.')
    } finally {
      setBusy(false)
    }
  }

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="inline-flex h-6 shrink-0 items-center gap-1.5 rounded-full bg-amber-soft px-2.5 text-xs text-amber"
        >
          <ShieldOff className="size-3" />
          sem trilha na AWS
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-[360px] space-y-3 text-sm">
        <p className="font-medium">O que se faz neste cofre não fica registrado na AWS</p>
        <ul className="space-y-2 text-[13px] text-muted-foreground">
          {off.map((group) => (
            <li key={group.group}>
              <span className="block font-mono text-[11px] text-faint">{group.group}</span>
              {group.reason}
            </li>
          ))}
        </ul>
        <p className="text-[13px] text-muted-foreground">
          {audit.required
            ? 'Este cofre exige a trilha: revelar, copiar, mover, excluir e compartilhar ficam bloqueados até ela existir.'
            : 'O Keyhold funciona normalmente. Quem administra a conta resolve isso seguindo o docs/AWS.md do Keyhold.'}
        </p>
        {error && <p className="text-[13px] text-rose">{error}</p>}
        <Button variant="outline" size="sm" disabled={busy} onClick={() => void retry()}>
          {busy ? <Loader2 className="animate-spin" /> : <RotateCw />}
          Tentar de novo
        </Button>
      </PopoverContent>
    </Popover>
  )
}
