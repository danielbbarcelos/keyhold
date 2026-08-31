import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { cn } from '@/lib/utils'

import { COLOURS } from './colours'

/** Six swatches, named out loud so the choice is not a guess about what a colour means. */
export function ColourPicker({
  value,
  onChange,
}: {
  value: string
  onChange: (colour: string) => void
}) {
  return (
    <div className="space-y-2">
      <Label>Cor</Label>
      <div className="flex flex-wrap gap-1.5">
        {COLOURS.map((colour) => (
          <Button
            key={colour.value}
            type="button"
            variant={value === colour.value ? 'secondary' : 'ghost'}
            size="sm"
            aria-pressed={value === colour.value}
            className={cn(value === colour.value && 'ring-1 ring-ring')}
            onClick={() => onChange(colour.value)}
          >
            <span className={cn('size-2.5 rounded-full', colour.dot)} aria-hidden="true" />
            {colour.label}
          </Button>
        ))}
      </div>
    </div>
  )
}
