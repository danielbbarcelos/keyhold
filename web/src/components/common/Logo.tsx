import { cn } from '@/lib/utils'

/**
 * The mark, from the same file the desktop shortcut and the browser tab use.
 *
 * Served by the binary out of the embedded bundle, like everything else here — there is no remote
 * origin in the policy that would let it come from anywhere else, and a vault that fetches its own
 * logo over the network is a vault that has told somebody it is open.
 *
 * `alt=""` on purpose: wherever this appears the name is already written beside it, and a screen
 * reader announcing "Keyhold logo, Keyhold" is worse than silence.
 */
export function Logo({ className }: { className?: string }) {
  return (
    <img
      src="/icon-512.png"
      alt=""
      draggable={false}
      className={cn('shrink-0 select-none rounded-[4px] object-contain', className)}
    />
  )
}
