import { useState } from 'react'
import { AlertTriangle, Pencil, ShieldCheck } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'

import type { HealthIssue } from '../../api'

/** What a record needing attention needs: the issues found, and each said in words. */
export interface Concern {
  issues: HealthIssue[]
  reasons: string[]
}

export type Attention = Map<string, Concern>

/** What to do about each kind of issue, in the order the person would do it. */
const ACTION: Record<HealthIssue, string> = {
  compromised:
    'Troque a senha no próprio serviço e salve a nova em Editar. Depois, remova a marca de comprometido.',
  rotation: 'Troque a senha no serviço, salve a nova em Editar e escolha a próxima data em “Trocar até”.',
  reused: 'Use uma senha diferente neste serviço. Em Editar, o gerador do campo de senha cria uma nova.',
  weak: 'Troque por uma senha mais forte. Em Editar, o gerador do campo de senha cria uma.',
  stale: 'Recomenda-se trocar a senha. Se ela não precisa mudar, defina uma data em “Trocar até” como lembrete.',
}

/**
 * The red dot beside a record that needs attention, which opens what was found and what to do.
 * A click on the dot never opens the record underneath.
 */
export function AttentionDot({
  concern,
  onEdit,
  onClearCompromised,
}: {
  concern: Concern
  onEdit: () => void
  onClearCompromised: () => void
}) {
  const [open, setOpen] = useState(false)
  const issues = [...new Set(concern.issues)]

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label="Pede atenção: visualizar o motivo"
          title="Pede atenção"
          onClick={(event) => event.stopPropagation()}
          onKeyDown={(event) => event.stopPropagation()}
          className="grid size-5 shrink-0 place-items-center rounded-full hover:bg-rose-soft"
        >
          <span className="size-1.5 rounded-full bg-rose" />
        </button>
      </PopoverTrigger>
      <PopoverContent
        side="right"
        align="start"
        className="w-[320px] p-0"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="flex items-center gap-2 border-b px-4 py-3">
          <AlertTriangle className="size-4 text-rose" />
          <b className="text-sm font-semibold">Pede atenção</b>
        </div>
        <div className="space-y-3 px-4 py-3">
          {concern.reasons.length > 0 && (
            <ul className="space-y-1 text-[13px]">
              {concern.reasons.map((reason) => (
                <li key={reason}>{reason}</li>
              ))}
            </ul>
          )}
          <div className="space-y-1.5">
            <span className="text-xs font-medium text-faint">O que fazer</span>
            {issues.map((issue) => (
              <p key={issue} className="text-[13px] text-muted-foreground">
                {ACTION[issue]}
              </p>
            ))}
          </div>
        </div>
        <div className="flex flex-wrap gap-2 border-t px-4 py-3">
          <Button
            size="sm"
            onClick={() => {
              setOpen(false)
              onEdit()
            }}
          >
            <Pencil />
            Editar
          </Button>
          {issues.includes('compromised') && (
            <Button
              size="sm"
              variant="outline"
              onClick={() => {
                setOpen(false)
                onClearCompromised()
              }}
            >
              <ShieldCheck />
              Remover marca de comprometido
            </Button>
          )}
        </div>
      </PopoverContent>
    </Popover>
  )
}
