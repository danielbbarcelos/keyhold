import { cn } from '@/lib/utils'

/** A shape standing where content is about to be: the foreground at a whisper, pulsing unless motion is reduced. */
export function Skeleton({ className, ...props }: React.ComponentProps<'div'>) {
  return <div aria-hidden="true" className={cn('rounded-md bg-foreground/[0.07] motion-safe:animate-pulse', className)} {...props} />
}
