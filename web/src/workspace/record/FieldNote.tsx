import { useRef, useState } from 'react'
import { Info } from 'lucide-react'

import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { t } from '@/i18n'
import { cn } from '@/lib/utils'

/**
 * A field's note, as an info mark right after its label.
 *
 * Hovering shows it like a tooltip; a click pins it open (which is why the mark has a pointer), and
 * a click outside or Escape lets it go. The text keeps the lines it was written with. The mark
 * never shrinks: a long label truncates on its own and the mark stays after the ellipsis.
 */
export function FieldNote({ label, note }: { label: string; note: string }) {
  const [hover, setHover] = useState(false)
  const [pinned, setPinned] = useState(false)
  const leaving = useRef(0)

  const enter = () => {
    window.clearTimeout(leaving.current)
    setHover(true)
  }
  const leave = () => {
    leaving.current = window.setTimeout(() => setHover(false), 120)
  }

  return (
    <Popover
      open={hover || pinned}
      onOpenChange={(open) => {
        if (!open) {
          setPinned(false)
          setHover(false)
        }
      }}
    >
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={t('Observação do campo {label}', { label })}
          onPointerEnter={enter}
          onPointerLeave={leave}
          onClick={() => setPinned((now) => !now)}
          className={cn(
            'grid size-5 shrink-0 cursor-pointer place-items-center rounded text-faint hover:bg-hover hover:text-foreground',
            pinned && 'text-foreground',
          )}
        >
          <Info className="size-3.5" />
        </button>
      </PopoverTrigger>
      <PopoverContent
        side="bottom"
        align="center"
        sideOffset={6}
        onPointerEnter={enter}
        onPointerLeave={leave}
        onOpenAutoFocus={(event) => event.preventDefault()}
        className="w-auto max-w-xs rounded-md border-0 bg-foreground px-3 py-1.5 text-xs leading-[1.45] break-words whitespace-pre-line text-background shadow-pop"
      >
        {note}
      </PopoverContent>
    </Popover>
  )
}
