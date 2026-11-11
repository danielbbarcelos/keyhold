import { cn } from '@/lib/utils'

/**
 * Keyhold's mark, as the pictures the program carries. Two of each, one per theme: the "-dark"
 * one is drawn for a dark background and the "-light" one for a light background, and CSS shows
 * whichever matches the theme in use (including "sistema", which follows the `.dark` class).
 */
export function LogoIcon({ className }: { className?: string }) {
  return (
    <>
      <img src="/brand/keyhold-icon-light.png" alt="Keyhold" className={cn('dark:hidden', className)} />
      <img src="/brand/keyhold-icon-dark.png" alt="Keyhold" className={cn('hidden dark:block', className)} />
    </>
  )
}

export function LogoLockup({ className }: { className?: string }) {
  return (
    <>
      <img src="/brand/keyhold-lockup-light.png" alt="Keyhold" className={cn('dark:hidden', className)} />
      <img src="/brand/keyhold-lockup-dark.png" alt="Keyhold" className={cn('hidden dark:block', className)} />
    </>
  )
}
