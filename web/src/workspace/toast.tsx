import { useEffect, useState } from 'react'
import { CircleAlert, CircleCheck, X } from 'lucide-react'

import { cn } from '@/lib/utils'

import { ApiError } from '../api'

/**
 * Short notices: "Copiado", "Registro salvo", and failures that are not worth a dialog.
 *
 * Top centre of the working area, under the top bar, newest on top: the place the eye already is
 * after pressing a button in the top bar or in a record's header. A module-level store rather than
 * a context, because the things that raise a toast are often gone (a closed dialog, a row that was
 * deleted) by the time it shows.
 */
export interface Toast {
  id: number
  tone: 'ok' | 'error'
  title: string
  detail?: string
}

let next = 1
let toasts: Toast[] = []
const listeners = new Set<(all: Toast[]) => void>()

function publish() {
  listeners.forEach((listener) => listener(toasts))
}

export function toast(title: string, detail?: string, tone: Toast['tone'] = 'ok') {
  const id = next++
  toasts = [{ id, tone, title, detail }, ...toasts].slice(0, 4)
  publish()
  window.setTimeout(() => dismiss(id), tone === 'error' ? 7000 : 3500)
}

export function toastError(cause: unknown, title = 'Não deu certo') {
  toast(title, describe(cause), 'error')
}

export function dismiss(id: number) {
  toasts = toasts.filter((one) => one.id !== id)
  publish()
}

export function describe(cause: unknown): string {
  return cause instanceof ApiError ? cause.message : 'Não foi possível falar com o Keyhold.'
}

export function Toaster() {
  const [all, setAll] = useState<Toast[]>(toasts)

  useEffect(() => {
    listeners.add(setAll)

    return () => void listeners.delete(setAll)
  }, [])

  return (
    <div
      aria-live="polite"
      className="pointer-events-none fixed top-[72px] left-[calc(76px+(100vw-76px)/2)] z-[60] flex w-[380px] max-w-[calc(100vw-96px)] -translate-x-1/2 flex-col gap-2.5"
    >
      {all.map((one) => (
        <div
          key={one.id}
          role={one.tone === 'error' ? 'alert' : 'status'}
          className="pointer-events-auto flex items-start gap-3 rounded-xl border bg-popover px-4 py-3 text-popover-foreground shadow-pop animate-in fade-in slide-in-from-top-2"
        >
          {one.tone === 'error' ? (
            <CircleAlert className="mt-0.5 size-[18px] shrink-0 text-rose" />
          ) : (
            <CircleCheck className="mt-0.5 size-[18px] shrink-0 text-mint" />
          )}
          <span className="min-w-0 flex-1">
            <b className="block text-sm font-semibold">{one.title}</b>
            {one.detail && <span className="block text-xs text-muted-foreground">{one.detail}</span>}
          </span>
          <button
            type="button"
            aria-label="Fechar"
            onClick={() => dismiss(one.id)}
            className={cn('grid size-6 place-items-center rounded-md text-faint hover:bg-hover hover:text-foreground')}
          >
            <X className="size-3.5" />
          </button>
        </div>
      ))}
    </div>
  )
}
