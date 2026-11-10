import { useMemo, useRef, useState } from 'react'
import { ImageUp, Search, Trash2 } from 'lucide-react'

import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'

import type { RecordIcon } from '../api'
import { ICON_COLOURS, ICONS, iconOf, RecordTile } from './icons'
import { toast } from './toast'

/**
 * A record's icon: a lucide icon in one of the interface's colours, or a picture of the person's own.
 *
 * The picture is dropped, pasted or chosen from disk, shrunk to a small square here, and sealed in
 * the record with everything else. Nothing is fetched from the web: a vault that looked up a logo
 * would tell somebody's DNS which services that person has accounts with.
 */
const SIDE = 96
const LIMIT = 200 * 1024

export function IconPicker({
  icon,
  kind,
  onChange,
  label = 'Trocar o ícone',
}: {
  icon: RecordIcon | null | undefined
  kind?: string
  onChange: (icon: RecordIcon) => void
  label?: string
}) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [dragging, setDragging] = useState(false)
  const file = useRef<HTMLInputElement>(null)
  const shown = iconOf(icon, kind)

  const found = useMemo(() => {
    const term = query.trim().toLowerCase()

    return term
      ? ICONS.filter((one) => one.name.includes(term) || one.words.includes(term))
      : ICONS.slice(0, 32)
  }, [query])

  async function usePicture(blob: Blob | null | undefined) {
    if (!blob || !blob.type.startsWith('image/')) {
      toast('Isso não é uma imagem', 'Use PNG, JPEG, WebP ou SVG.', 'error')

      return
    }

    try {
      const image = await shrink(blob)
      onChange({ name: shown.name, colour: shown.colour, image })
      setOpen(false)
    } catch {
      toast('Não consegui usar essa imagem', 'Tente outra, ou um arquivo menor.', 'error')
    }
  }

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={label}
          title={label}
          className="relative rounded-[10px] outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
        >
          <RecordTile icon={icon} kind={kind} size="md" />
          <span className="absolute -right-1 -bottom-1 size-3.5 rounded-full border bg-card" />
        </button>
      </PopoverTrigger>

      <PopoverContent
        className="w-[340px] p-0"
        onPaste={(event) => {
          const item = [...event.clipboardData.items].find((one) => one.type.startsWith('image/'))

          if (item) {
            event.preventDefault()
            void usePicture(item.getAsFile())
          }
        }}
      >
        <div className="flex h-12 items-center px-4 text-sm font-semibold">Ícone</div>

        <div className="relative px-3.5">
          <Search className="pointer-events-none absolute top-1/2 left-6 size-4 -translate-y-1/2 text-faint" />
          <Input
            autoFocus
            className="h-[34px] pl-9"
            placeholder="Buscar ícone"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
        </div>

        <div className="grid max-h-[176px] grid-cols-8 gap-1 overflow-y-auto px-3.5 py-2">
          {found.map(({ name, icon: Icon }) => (
            <button
              key={name}
              type="button"
              title={name}
              aria-label={name}
              onClick={() => onChange({ name, colour: shown.colour })}
              className={cn(
                'grid h-8 place-items-center rounded-lg text-muted-foreground hover:bg-hover hover:text-foreground',
                !shown.image && shown.name === name && 'bg-primary-soft text-primary-text ring-1 ring-primary',
              )}
            >
              <Icon className="size-4" />
            </button>
          ))}
          {found.length === 0 && (
            <p className="col-span-8 py-3 text-center text-xs text-faint">Nenhum ícone com esse nome.</p>
          )}
        </div>

        <div className="px-4 pt-1 text-xs text-faint">Cor</div>
        <div className="flex gap-2 px-3.5 pt-2 pb-3">
          {ICON_COLOURS.map((colour) => (
            <button
              key={colour.value}
              type="button"
              aria-label={colour.label}
              title={colour.label}
              onClick={() => onChange({ name: shown.name, colour: colour.value })}
              className={cn(
                'size-[22px] rounded-full',
                colour.dot,
                !shown.image && shown.colour === colour.value && 'ring-2 ring-foreground/60 ring-offset-2 ring-offset-popover',
              )}
            />
          ))}
        </div>

        <div className="border-t p-3.5">
          <button
            type="button"
            onClick={() => file.current?.click()}
            onDragOver={(event) => {
              event.preventDefault()
              setDragging(true)
            }}
            onDragLeave={() => setDragging(false)}
            onDrop={(event) => {
              event.preventDefault()
              setDragging(false)
              void usePicture(event.dataTransfer.files[0])
            }}
            className={cn(
              'flex w-full flex-col items-center gap-1 rounded-[10px] border border-dashed border-line-2 px-3 py-3.5 text-center text-xs text-muted-foreground hover:bg-hover',
              dragging && 'border-primary bg-primary-soft',
            )}
          >
            <ImageUp className="size-[18px] text-faint" />
            <b className="text-[13px] font-semibold text-foreground">Usar uma imagem sua</b>
            Solte, cole ou escolha do disco.
          </button>

          {shown.image && (
            <button
              type="button"
              onClick={() => onChange({ name: shown.name, colour: shown.colour })}
              className="mt-2 flex w-full items-center justify-center gap-1.5 rounded-lg py-1.5 text-xs text-muted-foreground hover:bg-hover hover:text-foreground"
            >
              <Trash2 className="size-3.5" />
              Tirar a imagem e voltar ao ícone
            </button>
          )}

          <input
            ref={file}
            type="file"
            accept="image/*"
            hidden
            onChange={(event) => {
              void usePicture(event.target.files?.[0])
              event.target.value = ''
            }}
          />
        </div>
      </PopoverContent>
    </Popover>
  )
}

/**
 * Draw the picture into a small square, cropped from the centre, and hand back a data address.
 *
 * Read through a data address rather than a `blob:` one: the page's policy lets images come from
 * `data:` and nowhere else, which is the point of the policy.
 */
async function shrink(blob: Blob): Promise<string> {
  const source = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve(String(reader.result))
    reader.onerror = reject
    reader.readAsDataURL(blob)
  })

  const image = await new Promise<HTMLImageElement>((resolve, reject) => {
    const one = new Image()
    one.onload = () => resolve(one)
    one.onerror = reject
    one.src = source
  })

  const canvas = document.createElement('canvas')
  canvas.width = SIDE
  canvas.height = SIDE

  const context = canvas.getContext('2d')!
  const side = Math.min(image.naturalWidth, image.naturalHeight) || SIDE
  const x = ((image.naturalWidth || SIDE) - side) / 2
  const y = ((image.naturalHeight || SIDE) - side) / 2

  context.drawImage(image, x, y, side, side, 0, 0, SIDE, SIDE)

  const data = canvas.toDataURL('image/png')

  return data.length > LIMIT ? canvas.toDataURL('image/jpeg', 0.85) : data
}
