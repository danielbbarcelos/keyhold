import { useEffect, useRef, useState } from 'react'
import { Check, Copy, Eye, EyeOff } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'

import { api } from '../api'
import { copyForSeconds } from '../clipboard'

/**
 * A secret field — the component this app will be remembered by.
 *
 * **Revealing is an event, not a state you toggle and forget.** The value comes into view for a
 * fixed number of seconds while a bar drains across the row, then goes back under the dots on its
 * own. That is not decoration: a value left on screen is a value somebody can photograph, and a
 * vault that quietly leaves a password visible on a second monitor has failed at the one thing it
 * does.
 *
 * **Copying says when it will stop being true.** The countdown is shown rather than promised
 * silently, and the tooltip admits what the interface cannot control — an operating system with
 * clipboard history keeps a copy no page can reach.
 */

const REVEAL_SECONDS = 20
const CLIPBOARD_SECONDS = 30

interface SecretLineProps {
  label: string
  value: string
  /** `false` for the fields that are not secret — a username is not worth hiding. */
  masked?: boolean
  /**
   * The record this field belongs to, so uncovering and copying reach the trail.
   *
   * Absent where there is nothing to record against — an old version being previewed, for
   * instance, which is a read of something that no longer is.
   */
  recordId?: string
}

export function SecretLine({ label, value, masked = true, recordId }: SecretLineProps) {
  const [showing, setShowing] = useState(!masked)
  const [remaining, setRemaining] = useState(0)
  const [copied, setCopied] = useState(0)
  const timer = useRef<number | undefined>(undefined)

  // Put the value back under the dots when its time is up, and whenever this row goes away.
  useEffect(() => {
    if (!showing || !masked) {
      return
    }

    setRemaining(REVEAL_SECONDS)

    timer.current = window.setInterval(() => {
      setRemaining((left) => {
        if (left <= 1) {
          setShowing(false)

          return 0
        }

        return left - 1
      })
    }, 1000)

    return () => window.clearInterval(timer.current)
  }, [showing, masked])

  useEffect(() => {
    if (copied <= 0) {
      return
    }

    const tick = window.setInterval(() => setCopied((left) => Math.max(0, left - 1)), 1000)

    return () => window.clearInterval(tick)
  }, [copied])

  return (
    <div className="group relative grid grid-cols-[10rem_1fr_auto] items-center gap-3 border-b py-2.5 last:border-b-0">
      <span className="truncate text-sm text-muted-foreground">{label}</span>

      <span
        className={
          showing
            ? 'truncate text-sm'
            : 'truncate text-sm tracking-[0.2em] text-muted-foreground select-none'
        }
      >
        {showing ? value || '—' : '•'.repeat(Math.min(value.length || 8, 20))}
      </span>

      <span className="flex items-center gap-1 opacity-60 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
        {masked && (
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={showing ? 'Esconder' : 'Revelar'}
                onClick={() => {
                  if (!showing && masked && recordId) {
                    // Fire and forget: a trail that could block a reveal would be a trail that
                    // stands between somebody and their own password.
                    void api.reportAccess(recordId, 'reveal', label).catch(() => {})
                  }

                  setShowing(!showing)
                }}
              >
                {showing ? <EyeOff /> : <Eye />}
              </Button>
            </TooltipTrigger>
            <TooltipContent>
              {showing ? `Esconde em ${remaining}s` : 'Revelar por 20 segundos'}
            </TooltipContent>
          </Tooltip>
        )}

        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="Copiar"
              onClick={() => {
                if (recordId) {
                  void api.reportAccess(recordId, 'copy', label).catch(() => {})
                }

                void copyForSeconds(value, CLIPBOARD_SECONDS, () => setCopied(0)).then(() =>
                  setCopied(CLIPBOARD_SECONDS),
                )
              }}
            >
              {copied > 0 ? <Check className="text-primary" /> : <Copy />}
            </Button>
          </TooltipTrigger>
          <TooltipContent>
            {copied > 0
              ? `A área de transferência é limpa em ${copied}s`
              : 'Copiar, e limpar depois de 30s'}
          </TooltipContent>
        </Tooltip>
      </span>

      {showing && masked && (
        <span
          aria-hidden="true"
          data-seconds={remaining}
          className="absolute bottom-0 left-0 h-px bg-primary transition-[width] duration-1000 ease-linear"
        />
      )}
    </div>
  )
}
