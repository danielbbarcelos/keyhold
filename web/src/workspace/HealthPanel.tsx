import { AlertTriangle, ShieldAlert, ShieldCheck } from 'lucide-react'

import { Badge } from '@/components/ui/badge'
import { cn } from '@/lib/utils'

import type { HealthFinding, HealthIssue, VaultHealth } from '../api'

/**
 * The health lens: what is wrong with the passwords in this vault, and why.
 *
 * Written as a calm signal rather than an alarm. Everything in this list is a password somebody
 * chose and is attached to, and a screen that shouts equally at "you marked this one as leaked" and
 * "this one has not changed in a year" teaches people to ignore both.
 *
 * The two urgencies are kept apart in colour: red means somebody knows this secret, everything else
 * is a suggestion.
 */
const LABELS: Record<HealthIssue, string> = {
  compromised: 'Comprometida',
  rotation: 'Troca vencida',
  reused: 'Repetida',
  weak: 'Fraca',
  stale: 'Antiga',
}

function isUrgent(issue: HealthIssue): boolean {
  return issue === 'compromised'
}

interface HealthPanelProps {
  health: VaultHealth | null
  onOpen: (recordId: string) => void
}

export function HealthPanel({ health, onOpen }: HealthPanelProps) {
  if (!health) {
    return <p className="p-6 text-sm text-muted-foreground">Conferindo as senhas…</p>
  }

  if (health.findings.length === 0) {
    return (
      <div className="grid flex-1 place-items-center p-8 text-center">
        <div className="max-w-sm space-y-2">
          <ShieldCheck className="mx-auto size-6 text-muted-foreground" />
          <p className="text-sm">
            {health.examined === 0
              ? 'Nada guardado neste cofre ainda.'
              : `Nada a dizer sobre ${health.examined === 1 ? 'a senha' : 'as senhas'} deste cofre.`}
          </p>
          <p className="text-sm text-muted-foreground">
            O keyhold não dá nota a senha: ele só aponta o que dá para conferir — curta demais, um
            só tipo de caractere, repetida, previsível, ou parada há tempo demais.
          </p>
        </div>
      </div>
    )
  }

  return (
    <div className="space-y-3 p-6">
      <p className="text-sm text-muted-foreground">
        {health.findings.length} de {health.examined}{' '}
        {health.examined === 1 ? 'registro pede' : 'registros pedem'} atenção.
      </p>

      <ul className="space-y-2">
        {health.findings.map((finding) => (
          <li key={finding.record_id}>
            <FindingRow finding={finding} onOpen={() => onOpen(finding.record_id)} />
          </li>
        ))}
      </ul>
    </div>
  )
}

function FindingRow({ finding, onOpen }: { finding: HealthFinding; onOpen: () => void }) {
  const urgent = finding.issues.some(isUrgent)

  return (
    <button
      type="button"
      onClick={onOpen}
      className={cn(
        'w-full rounded-lg border p-4 text-left transition-colors hover:bg-muted/40',
        urgent && 'border-destructive/40',
      )}
    >
      <span className="flex flex-wrap items-center gap-2">
        {urgent ? (
          <ShieldAlert className="size-4 shrink-0 text-destructive" />
        ) : (
          <AlertTriangle className="size-4 shrink-0 text-muted-foreground" />
        )}

        <span className="font-medium">{finding.title}</span>

        {finding.issues.map((issue) => (
          <Badge key={issue} variant={isUrgent(issue) ? 'destructive' : 'secondary'}>
            {LABELS[issue]}
          </Badge>
        ))}
      </span>

      <span className="mt-2 block space-y-0.5 text-sm text-muted-foreground">
        {finding.reasons.map((reason) => (
          <span key={reason} className="block">
            {reason}
          </span>
        ))}
      </span>
    </button>
  )
}
