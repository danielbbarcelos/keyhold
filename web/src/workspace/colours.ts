/**
 * The colours a vault or folder can be marked with.
 *
 * Kept to six, named for what they look like rather than for a semantic role: nobody is marking a
 * folder as "destructive", they are marking the one with the client work in it so their eye finds
 * it. The token is stored inside the sealed metadata, so even the colour of a folder is not
 * readable in the file.
 *
 * The classes are written out rather than composed, because Tailwind only ships the class names it
 * can see in the source — an interpolated `text-${colour}-500` would simply not exist.
 */
export interface Colour {
  value: string
  label: string
  dot: string
  text: string
}

export const COLOURS: Colour[] = [
  { value: 'default', label: 'Padrão', dot: 'bg-muted-foreground', text: 'text-muted-foreground' },
  { value: 'blue', label: 'Azul', dot: 'bg-sky-500', text: 'text-sky-500' },
  { value: 'green', label: 'Verde', dot: 'bg-emerald-500', text: 'text-emerald-500' },
  { value: 'amber', label: 'Âmbar', dot: 'bg-amber-500', text: 'text-amber-500' },
  { value: 'red', label: 'Vermelho', dot: 'bg-rose-500', text: 'text-rose-500' },
  { value: 'violet', label: 'Violeta', dot: 'bg-violet-500', text: 'text-violet-500' },
]

/** Anything unknown — including the `brand` written by earlier versions — reads as the default. */
export function colourOf(value: string | null | undefined): Colour {
  return COLOURS.find((colour) => colour.value === value) ?? COLOURS[0]
}
