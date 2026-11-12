import { useState } from 'react'
import { Calendar, ChevronLeft, ChevronRight, X } from 'lucide-react'

import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { cn } from '@/lib/utils'
import { Tip } from '@/components/ui/tip'

/**
 * A date, picked from a small calendar in the interface's own look rather than the browser's.
 *
 * The value is `YYYY-MM-DD` (or empty), the same string a native date input would give, so it can
 * stand in anywhere one was. Typing is still possible: the field accepts `dd/mm/aaaa`.
 */
const MONTHS = ['janeiro', 'fevereiro', 'março', 'abril', 'maio', 'junho', 'julho', 'agosto', 'setembro', 'outubro', 'novembro', 'dezembro']
const DAYS = ['D', 'S', 'T', 'Q', 'Q', 'S', 'S']

function toIso(date: Date): string {
  const pad = (n: number) => String(n).padStart(2, '0')

  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`
}

function fromIso(value: string): Date | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})/.exec(value)

  return match ? new Date(Number(match[1]), Number(match[2]) - 1, Number(match[3])) : null
}

function shown(value: string): string {
  const date = fromIso(value)

  return date ? date.toLocaleDateString('pt-BR') : ''
}

export function DatePicker({
  value,
  onChange,
  placeholder = 'Sem data',
  className,
}: {
  value: string
  onChange: (value: string) => void
  placeholder?: string
  className?: string
}) {
  const [open, setOpen] = useState(false)
  const [typed, setTyped] = useState<string | null>(null)
  const chosen = fromIso(value)
  const [month, setMonth] = useState(() => {
    const at = chosen ?? new Date()

    return new Date(at.getFullYear(), at.getMonth(), 1)
  })

  const today = toIso(new Date())
  const first = new Date(month.getFullYear(), month.getMonth(), 1)
  const start = new Date(first)
  start.setDate(1 - first.getDay())
  const cells = Array.from({ length: 42 }, (_, index) => {
    const day = new Date(start)
    day.setDate(start.getDate() + index)

    return day
  })

  const shift = (months: number) => setMonth(new Date(month.getFullYear(), month.getMonth() + months, 1))

  function commitTyped() {
    if (typed === null) return
    const match = /^(\d{1,2})\/(\d{1,2})\/(\d{4})$/.exec(typed.trim())

    if (typed.trim() === '') onChange('')
    else if (match) {
      const date = new Date(Number(match[3]), Number(match[2]) - 1, Number(match[1]))
      // 31/02 would roll into March without a word; a date that is not the one typed is refused.
      if (date.getDate() === Number(match[1]) && date.getMonth() === Number(match[2]) - 1) onChange(toIso(date))
    }
    setTyped(null)
  }

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next)
        if (next && chosen) setMonth(new Date(chosen.getFullYear(), chosen.getMonth(), 1))
      }}
    >
      <div className={cn('relative', className)}>
        <input
          aria-label="Data"
          value={typed ?? shown(value)}
          placeholder={placeholder}
          onChange={(event) => setTyped(event.target.value)}
          onBlur={commitTyped}
          onKeyDown={(event) => event.key === 'Enter' && commitTyped()}
          className="h-9 w-full rounded-[9px] border border-border bg-field pr-16 pl-3 text-sm outline-none placeholder:text-faint focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50"
        />
        <span className="absolute top-1/2 right-1 flex -translate-y-1/2 items-center">
          {value && (
            <Tip label="Remover a data"><button
              type="button"
             
              aria-label="Remover a data"
              onClick={() => onChange('')}
              className="grid size-7 place-items-center rounded-md text-faint hover:bg-hover hover:text-foreground"
            >
              <X className="size-3.5" />
            </button></Tip>
          )}
          <Tip label="Escolher no calendário">
          <PopoverTrigger asChild>
            <button
              type="button"
              aria-label="Escolher no calendário"
              className="grid size-7 place-items-center rounded-md text-faint hover:bg-hover hover:text-foreground"
            >
              <Calendar className="size-4" />
            </button>
          </PopoverTrigger>
          </Tip>
        </span>
      </div>

      <PopoverContent align="end" className="w-[268px] p-3">
        <div className="mb-2 flex items-center gap-1">
          <Tip label="Mês anterior"><button type="button" aria-label="Mês anterior" onClick={() => shift(-1)} className="grid size-7 place-items-center rounded-md text-muted-foreground hover:bg-hover">
            <ChevronLeft className="size-4" />
          </button></Tip>
          <b className="flex-1 text-center text-sm font-semibold first-letter:uppercase">
            {MONTHS[month.getMonth()]} de {month.getFullYear()}
          </b>
          <Tip label="Próximo mês"><button type="button" aria-label="Próximo mês" onClick={() => shift(1)} className="grid size-7 place-items-center rounded-md text-muted-foreground hover:bg-hover">
            <ChevronRight className="size-4" />
          </button></Tip>
        </div>

        <div className="grid grid-cols-7 gap-0.5 text-center">
          {DAYS.map((day, index) => (
            <span key={index} className="py-1 text-[11px] text-faint">
              {day}
            </span>
          ))}
          {cells.map((day) => {
            const iso = toIso(day)
            const outside = day.getMonth() !== month.getMonth()

            return (
              <button
                key={iso}
                type="button"
                onClick={() => {
                  onChange(iso)
                  setOpen(false)
                }}
                className={cn(
                  'h-8 rounded-lg text-[13px] hover:bg-hover',
                  outside && 'text-faint',
                  iso === today && 'font-semibold text-primary-text',
                  iso === value && 'bg-primary text-primary-foreground hover:bg-primary',
                )}
              >
                {day.getDate()}
              </button>
            )
          })}
        </div>

        <div className="mt-2 flex justify-between border-t pt-2 text-xs">
          <button
            type="button"
            className="rounded-md px-2 py-1 text-muted-foreground hover:bg-hover hover:text-foreground"
            onClick={() => {
              onChange(today)
              setOpen(false)
            }}
          >
            Hoje
          </button>
          {[
            ['Em 3 meses', 3],
            ['Em 1 ano', 12],
          ].map(([label, months]) => (
            <button
              key={label}
              type="button"
              className="rounded-md px-2 py-1 text-muted-foreground hover:bg-hover hover:text-foreground"
              onClick={() => {
                const at = new Date()
                at.setMonth(at.getMonth() + Number(months))
                onChange(toIso(at))
                setOpen(false)
              }}
            >
              {label}
            </button>
          ))}
        </div>
      </PopoverContent>
    </Popover>
  )
}
