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
  /** A soft square with the colour on it: a vault's initial, a folder's icon. */
  tile: string
}

export const COLOURS: Colour[] = [
  { value: 'default', label: 'Padrão', dot: 'bg-stone', text: 'text-muted-foreground', tile: 'bg-stone-soft text-muted-foreground' },
  { value: 'blue', label: 'Azul', dot: 'bg-sky', text: 'text-sky', tile: 'bg-sky-soft text-sky' },
  { value: 'green', label: 'Verde', dot: 'bg-mint', text: 'text-mint', tile: 'bg-mint-soft text-mint' },
  { value: 'amber', label: 'Âmbar', dot: 'bg-amber', text: 'text-amber', tile: 'bg-amber-soft text-amber' },
  { value: 'red', label: 'Vermelho', dot: 'bg-rose', text: 'text-rose', tile: 'bg-rose-soft text-rose' },
  { value: 'violet', label: 'Violeta', dot: 'bg-lilac', text: 'text-lilac', tile: 'bg-lilac-soft text-lilac' },
]

/** Anything unknown — including the `brand` written by earlier versions — reads as the default. */
export function colourOf(value: string | null | undefined): Colour {
  return COLOURS.find((colour) => colour.value === value) ?? COLOURS[0]
}
