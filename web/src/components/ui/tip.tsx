import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'

/** A tooltip around one element (a bare `<button>` with only an icon): the name of its action. */
export function Tip({ label, children }: { label: string; children: React.ReactElement }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}
