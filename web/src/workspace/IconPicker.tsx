import { useMemo, useRef, useState } from 'react'
import { BookmarkPlus, ImageUp, Search, Settings2, Trash2 } from 'lucide-react'

import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'

import type { RecordIcon } from '../api'
import { addIcon, useCustom } from './custom'
import { ICON_COLOURS, ICONS, iconOf, RecordTile } from './icons'
import { toast, toastError } from './toast'

/**
 * An icon: one of the system's (a lucide icon in one of the interface's colours) or a picture from
 * the person's own library.
 *
 * A picture is dropped, pasted or chosen from disk, shrunk to a small square here, kept in the
 * library to be used again, and sealed in the record with everything else: the record carries its
 * own copy, so it never depends on the library. Nothing is fetched from the web: a vault that
 * looked up a logo would tell somebody's DNS which services that person has accounts with.
 */
const SIDE = 96
const LIMIT = 200 * 1024

/** Asks the workspace to open Personalização on a section. The picker lives too deep to be handed a way there. */
export function goCustomise(section: string) {
  window.dispatchEvent(new CustomEvent('keyhold:customise', { detail: section }))
}

export function IconPicker({
  icon,
  kind,
  onChange,
  label = 'Trocar ícone',
  trigger,
  onClear,
  clearLabel = 'Voltar ao padrão',
}: {
  icon: RecordIcon | null | undefined
  kind?: string
  onChange: (icon: RecordIcon) => void
  label?: string
  /** What the button shows instead of the record tile (a vault shows its initials until it has an icon). */
  trigger?: React.ReactNode
  /** Offered when something can go back to having no icon of its own. */
  onClear?: () => void
  clearLabel?: string
}) {
  const shown = iconOf(icon, kind)
  const [open, setOpen] = useState(false)
  const [tab, setTab] = useState<'system' | 'custom'>(shown.image ? 'custom' : 'system')
  const [query, setQuery] = useState('')
  const [dragging, setDragging] = useState(false)
  const file = useRef<HTMLInputElement>(null)
  const { icons: library } = useCustom()

  const term = query.trim().toLowerCase()
  const found = useMemo(
    () => (term ? ICONS.filter((one) => one.name.includes(term) || one.words.includes(term)) : ICONS),
    [term],
  )
  const mine = term ? library.filter((one) => one.name.toLowerCase().includes(term)) : library
  const kept = !shown.image || library.some((one) => one.image === shown.image)

  async function usePicture(blob: Blob | null | undefined, name = 'Imagem colada') {
    if (!blob || !blob.type.startsWith('image/')) {
      toast('O arquivo não é uma imagem', 'Use PNG, JPEG, WebP ou SVG.', 'error')

      return
    }

    try {
      const image = await shrink(blob)
      onChange({ name: shown.name, colour: shown.colour, image })
      setTab('custom')
      // Kept for the next record that needs it. A failure here costs the reuse, not the icon.
      void addIcon(name.replace(/\.[a-z0-9]+$/i, ''), image).catch((cause) =>
        toastError(cause, 'O ícone foi aplicado, mas não foi guardado na biblioteca'),
      )
    } catch {
      toast('Não foi possível usar a imagem', 'Tente outra imagem ou um arquivo menor.', 'error')
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
          {trigger ?? <RecordTile icon={icon} kind={kind} size="md" />}
          <span className="absolute -right-1 -bottom-1 size-3.5 rounded-full border bg-card" />
        </button>
      </PopoverTrigger>

      <PopoverContent
        className="w-[360px] p-0"
        onPaste={(event) => {
          const item = [...event.clipboardData.items].find((one) => one.type.startsWith('image/'))

          if (item) {
            event.preventDefault()
            void usePicture(item.getAsFile())
          }
        }}
      >
        <div role="tablist" aria-label="Origem do ícone" className="flex gap-1 border-b px-2.5 pt-2.5">
          {(
            [
              ['system', 'Do sistema'],
              ['custom', `Personalizados${library.length ? ` (${library.length})` : ''}`],
            ] as const
          ).map(([id, text]) => (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={tab === id}
              onClick={() => {
                setTab(id)
                setQuery('')
              }}
              className={cn(
                '-mb-px h-9 rounded-t-lg border-b-2 border-transparent px-3 text-[13px] font-medium text-muted-foreground hover:text-foreground',
                tab === id && 'border-primary text-foreground',
              )}
            >
              {text}
            </button>
          ))}
        </div>

        <div className="relative px-3.5 pt-3">
          <Search className="pointer-events-none absolute top-[calc(50%+6px)] left-6 size-4 -translate-y-1/2 text-faint" />
          <Input
            autoFocus
            className="h-[34px] pl-9"
            placeholder={tab === 'system' ? 'Buscar ícone' : 'Buscar na sua biblioteca'}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
        </div>

        {tab === 'system' ? (
          <>
            <div className="grid max-h-[216px] grid-cols-9 gap-1 overflow-y-auto px-3.5 py-2">
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
                <p className="col-span-9 py-3 text-center text-xs text-faint">Nenhum ícone com esse nome.</p>
              )}
            </div>

            <div className="border-t px-4 pt-2.5 text-xs text-faint">Cor</div>
            <div className="grid grid-cols-7 justify-items-center gap-y-2 px-3.5 pt-2 pb-3.5">
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
          </>
        ) : (
          <>
            <div className="grid max-h-[216px] grid-cols-6 gap-1.5 overflow-y-auto px-3.5 py-2.5">
              {mine.map((one) => (
                <button
                  key={one.id}
                  type="button"
                  title={one.name}
                  aria-label={one.name}
                  onClick={() => onChange({ name: shown.name, colour: shown.colour, image: one.image })}
                  className={cn(
                    'grid aspect-square place-items-center rounded-[10px] p-1 hover:bg-hover',
                    shown.image === one.image && 'bg-primary-soft ring-1 ring-primary',
                  )}
                >
                  <img src={one.image} alt="" className="size-full rounded-lg object-cover" />
                </button>
              ))}
              {mine.length === 0 && (
                <p className="col-span-6 py-4 text-center text-xs text-faint">
                  {library.length === 0
                    ? 'Sua biblioteca está vazia. Adicione uma imagem para usar em vários registros.'
                    : 'Nenhum ícone com esse nome.'}
                </p>
              )}
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
                  const dropped = event.dataTransfer.files[0]
                  void usePicture(dropped, dropped?.name)
                }}
                className={cn(
                  'flex w-full flex-col items-center gap-1 rounded-[10px] border border-dashed border-line-2 px-3 py-3 text-center text-xs text-muted-foreground hover:bg-hover',
                  dragging && 'border-primary bg-primary-soft',
                )}
              >
                <ImageUp className="size-[18px] text-faint" />
                <b className="text-[13px] font-semibold text-foreground">Adicionar imagem</b>
                Arraste, cole ou escolha um arquivo. Ela fica na sua biblioteca.
              </button>

              <input
                ref={file}
                type="file"
                accept="image/*"
                hidden
                onChange={(event) => {
                  const chosen = event.target.files?.[0]
                  void usePicture(chosen, chosen?.name)
                  event.target.value = ''
                }}
              />
            </div>
          </>
        )}

        {(shown.image || tab === 'custom' || (onClear && icon && (icon.image || icon.name))) && (
          <div className="grid gap-0.5 border-t p-2">
            {shown.image && !kept && (
              <Row
                icon={BookmarkPlus}
                onClick={() =>
                  void addIcon('Sem nome', shown.image!)
                    .then(() => toast('Ícone guardado na biblioteca'))
                    .catch((cause) => toastError(cause, 'Não foi possível guardar o ícone'))
                }
              >
                Guardar esta imagem na biblioteca
              </Row>
            )}
            {shown.image && (
              <Row icon={Trash2} onClick={() => onChange({ name: shown.name, colour: shown.colour })}>
                Remover a imagem e voltar ao ícone
              </Row>
            )}
            {onClear && icon && (icon.image || icon.name) && (
              <Row
                onClick={() => {
                  onClear()
                  setOpen(false)
                }}
              >
                {clearLabel}
              </Row>
            )}
            {tab === 'custom' && (
              <Row
                icon={Settings2}
                onClick={() => {
                  setOpen(false)
                  goCustomise('icones')
                }}
              >
                Gerenciar a biblioteca
              </Row>
            )}
          </div>
        )}
      </PopoverContent>
    </Popover>
  )
}

function Row({
  icon: Icon,
  onClick,
  children,
}: {
  icon?: typeof Trash2
  onClick: () => void
  children: React.ReactNode
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex h-8 w-full items-center justify-center gap-1.5 rounded-lg text-xs text-muted-foreground hover:bg-hover hover:text-foreground"
    >
      {Icon && <Icon className="size-3.5" />}
      {children}
    </button>
  )
}

/**
 * Draw the picture into a small square, cropped from the centre, and hand back a data address.
 *
 * Read through a data address rather than a `blob:` one: the page's policy lets images come from
 * `data:` and nowhere else, which is the point of the policy.
 */
export async function shrink(blob: Blob): Promise<string> {
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
