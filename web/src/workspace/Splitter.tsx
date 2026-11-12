import { useRef, useState } from 'react'

import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'

import { api, type Layout } from '../api'

/**
 * The widths a person drags, kept by Keyhold itself (the interface never writes to the browser's
 * storage): as CSS variables on the page while dragging, and in the preferences when let go.
 */
export const LAYOUT_DEFAULTS: Layout = { tree: 300, label: 180 }
const LIMITS: Record<keyof Layout, [number, number]> = { tree: [220, 640], label: [110, 480] }
const VARIABLE: Record<keyof Layout, string> = { tree: '--kh-tree', label: '--kh-label' }

export function applyLayout(layout: Layout | undefined) {
  const root = document.documentElement.style
  const shown = { ...LAYOUT_DEFAULTS, ...(layout ?? {}) }
  root.setProperty(VARIABLE.tree, `${shown.tree}px`)
  root.setProperty(VARIABLE.label, `${shown.label}px`)
}

function current(which: keyof Layout): number {
  const raw = getComputedStyle(document.documentElement).getPropertyValue(VARIABLE[which])
  return Number.parseInt(raw, 10) || LAYOUT_DEFAULTS[which]
}

function save() {
  void api.saveLayout({ tree: current('tree'), label: current('label') }).catch(() => undefined)
}

/**
 * A handle to drag a width: the vault's tree, or the column of field names. Double-click puts the
 * width back to where it started.
 */
export function Splitter({ which, className }: { which: keyof Layout; className?: string }) {
  const start = useRef<{ x: number; width: number } | null>(null)
  const [dragging, setDragging] = useState(false)

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={which === 'tree' ? 'Largura da árvore' : 'Largura dos nomes dos campos'}
      title="Arraste para ajustar. Clique duas vezes para voltar ao padrão"
      onPointerDown={(event) => {
        event.preventDefault()
        event.stopPropagation()
        ;(event.target as HTMLElement).setPointerCapture(event.pointerId)
        start.current = { x: event.clientX, width: current(which) }
        setDragging(true)
      }}
      onPointerMove={(event) => {
        if (!start.current) return
        const [min, max] = LIMITS[which]
        const width = Math.min(max, Math.max(min, start.current.width + event.clientX - start.current.x))
        document.documentElement.style.setProperty(VARIABLE[which], `${width}px`)
      }}
      onPointerUp={() => {
        if (!start.current) return
        start.current = null
        setDragging(false)
        save()
      }}
      onDoubleClick={() => {
        document.documentElement.style.setProperty(VARIABLE[which], `${LAYOUT_DEFAULTS[which]}px`)
        save()
      }}
      className={cn(
        'group/split absolute top-0 bottom-0 z-10 flex w-3 cursor-col-resize touch-none justify-center',
        className,
      )}
    >
      <span
        className={cn(
          'h-full w-px bg-transparent transition-colors group-hover/split:bg-primary/60',
          dragging && 'bg-primary',
        )}
      />
    </div>
  )
}

/**
 * Text that may not fit: cut with an ellipsis, and shown whole in a tooltip, only when it was cut.
 */
export function Clip({ children, className }: { children: React.ReactNode; className?: string }) {
  const ref = useRef<HTMLSpanElement>(null)
  const [open, setOpen] = useState(false)

  return (
    <Tooltip
      open={open}
      onOpenChange={(next) => {
        const el = ref.current
        setOpen(next && !!el && el.scrollWidth > el.clientWidth)
      }}
    >
      <TooltipTrigger asChild>
        <span ref={ref} className={cn('block min-w-0 truncate', className)}>
          {children}
        </span>
      </TooltipTrigger>
      <TooltipContent className="max-w-[420px] break-words">{children}</TooltipContent>
    </Tooltip>
  )
}
