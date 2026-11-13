import { useEffect, useRef, useState } from 'react'
import {
  ArrowDown,
  ArrowUp,
  ImageUp,
  Images,
  LayoutList,
  Palette,
  Pencil,
  Plus,
  RotateCcw,
  Search,
  ShieldAlert,
  Trash2,
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { cn } from '@/lib/utils'

import type { CustomIcon, FieldKind, RecordType, Status } from '../api'
import { addIcon, resetTypes, saveIcons, saveTypes, useCustom } from './custom'
import { FIELD_TYPES, fieldType } from './fields'
import { IconPicker, shrink } from './IconPicker'
import { RecordTile } from './icons'
import { linkProps, TabMenu } from './nav'
import { AlertPresetsPanel } from './record/Alerts'
import { AppearanceSettings, FLAT } from './Settings'
import { toast, toastError } from './toast'

/**
 * Personalização: how Keyhold looks and what it offers when something new is made.
 *
 * Ajustes decides how the vault is protected; this decides what it feels like to use. The line
 * between them is whether a choice changes who can read something. Nothing here does.
 */
const SECTIONS: Array<{ id: string; label: string; icon: LucideIcon }> = [
  { id: 'aparencia', label: 'Aparência', icon: Palette },
  { id: 'tipos', label: 'Tipos de registro', icon: LayoutList },
  { id: 'icones', label: 'Ícones', icon: Images },
  { id: 'alertas', label: 'Alertas padrão', icon: ShieldAlert },
]

export function Customise({
  status,
  initial,
  request,
  onChanged,
  onSection,
}: {
  status: Status
  /** The section to open on; the first one when not said. */
  initial?: string
  /** Changes with every request to open a section, so asking twice for the same one still lands there. */
  request?: number
  onChanged: () => void
  /** Tells the top bar which section is being read. */
  onSection?: (label: string) => void
}) {
  const known = (id?: string) => SECTIONS.some((one) => one.id === id)
  const [active, setActive] = useState(known(initial) ? (initial as string) : SECTIONS[0].id)
  const page = useRef<HTMLDivElement>(null)

  function open(id: string) {
    setActive(id)
    page.current?.scrollTo({ top: 0 })
  }

  useEffect(() => {
    onSection?.(SECTIONS.find((one) => one.id === active)?.label ?? '')
  }, [active, onSection])

  useEffect(() => {
    if (known(initial)) open(initial as string)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [request])

  return (
    <div className="kh-wide grid min-h-0 flex-1 grid-cols-[232px_minmax(0,1fr)] grid-rows-[minmax(0,1fr)] gap-5 p-6 max-xl:grid-cols-[200px_minmax(0,1fr)]">
      <nav aria-label="Seções da personalização" className="h-fit space-y-0.5 rounded-[14px] border bg-card p-2.5">
        {SECTIONS.map(({ id, label, icon: Icon }) => (
          <TabMenu key={id} to={{ place: 'custom', section: id }}>
            <button
              type="button"
              aria-current={active === id}
              {...linkProps({ place: 'custom', section: id }, () => open(id))}
              className={cn(
                'relative flex h-9 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-sm whitespace-nowrap text-muted-foreground transition-colors hover:bg-hover hover:text-foreground',
                active === id &&
                  'bg-raised text-foreground ring-1 ring-border before:absolute before:top-2 before:bottom-2 before:-left-2.5 before:w-[3px] before:rounded-r before:bg-primary',
              )}
            >
              <Icon className="size-4" />
              {label}
            </button>
          </TabMenu>
        ))}
      </nav>

      <div ref={page} className="min-h-0 overflow-y-auto">
        <div className={cn('pb-24', FLAT)}>
          {active === 'aparencia' && <AppearanceSettings appearance={status.appearance} onChanged={onChanged} />}
          {active === 'tipos' && <RecordTypes />}
          {active === 'icones' && <IconLibrary />}
          {active === 'alertas' && <AlertPresetsPanel />}
        </div>
      </div>
    </div>
  )
}

function Section({
  title,
  hint,
  actions,
  children,
}: {
  title: string
  hint: React.ReactNode
  actions?: React.ReactNode
  children: React.ReactNode
}) {
  return (
    <section className="rounded-[14px] border bg-card">
      <div className="flex flex-wrap items-start gap-3 border-b px-5 py-4">
        <div className="min-w-[240px] flex-1">
          <h3 className="text-[15px] font-semibold">{title}</h3>
          <p className="mt-1 max-w-2xl text-[13px] text-muted-foreground">{hint}</p>
        </div>
        {actions && <div className="flex shrink-0 flex-wrap gap-2">{actions}</div>}
      </div>
      {children}
    </section>
  )
}

function IconAction({
  label,
  icon: Icon,
  disabled,
  onClick,
}: {
  label: string
  icon: LucideIcon
  disabled?: boolean
  onClick: () => void
}) {
  return (
    <Button variant="ghost" size="icon" aria-label={label} title={label} disabled={disabled} onClick={onClick}>
      <Icon />
    </Button>
  )
}

/** Swap an item with its neighbour. */
function moved<T>(list: T[], index: number, by: -1 | 1): T[] {
  const next = [...list]
  const to = index + by
  if (to < 0 || to >= next.length) return next
  ;[next[index], next[to]] = [next[to], next[index]]

  return next
}

// ── Tipos de registro ────────────────────────────────────────────────────────────────────────────

function RecordTypes() {
  const { types, loaded } = useCustom()
  const [editing, setEditing] = useState<RecordType | 'new' | null>(null)
  const [deleting, setDeleting] = useState<RecordType | null>(null)
  const [resetting, setResetting] = useState(false)

  const save = (next: RecordType[], done?: string) =>
    saveTypes(next)
      .then(() => done && toast(done))
      .catch((cause) => toastError(cause, 'Não foi possível salvar os tipos de registro'))

  return (
    <Section
      title="Tipos de registro"
      hint={
        <>
          O ponto de partida de um registro novo, em “Começar com”: um ícone e os campos que ele já traz. O registro
          não fica preso ao tipo. Alterar ou excluir um tipo não muda os registros que já existem.
        </>
      }
      actions={
        <>
          <Button variant="outline" size="sm" onClick={() => setResetting(true)}>
            <RotateCcw />
            Restaurar os padrões
          </Button>
          <Button size="sm" onClick={() => setEditing('new')}>
            <Plus />
            Novo tipo
          </Button>
        </>
      }
    >
      <div className="divide-y">
        {types.map((type, index) => (
          <div key={type.id} className="flex items-center gap-3 px-5 py-3 transition-colors hover:bg-hover">
            <RecordTile icon={type.icon} size="md" />
            <div className="min-w-0 flex-1">
              <b className="block truncate text-sm font-medium">{type.label}</b>
              <span className="block truncate text-xs text-muted-foreground">
                {type.fields.length === 0
                  ? 'Sem campos'
                  : type.fields.map((field) => field.label || fieldType(field.kind).label).join(', ')}
              </span>
            </div>
            <IconAction label="Mover para cima" icon={ArrowUp} disabled={index === 0} onClick={() => void save(moved(types, index, -1))} />
            <IconAction
              label="Mover para baixo"
              icon={ArrowDown}
              disabled={index === types.length - 1}
              onClick={() => void save(moved(types, index, 1))}
            />
            <IconAction label={`Editar ${type.label}`} icon={Pencil} onClick={() => setEditing(type)} />
            <IconAction label={`Excluir ${type.label}`} icon={Trash2} onClick={() => setDeleting(type)} />
          </div>
        ))}
        {loaded && types.length === 0 && (
          <p className="px-5 py-8 text-center text-sm text-muted-foreground">
            Nenhum tipo de registro. Um registro novo começa em branco.
          </p>
        )}
      </div>

      {editing && (
        <TypeDialog
          initial={editing === 'new' ? null : editing}
          onCancel={() => setEditing(null)}
          onDone={(made) => {
            const exists = types.some((one) => one.id === made.id)
            setEditing(null)
            void save(exists ? types.map((one) => (one.id === made.id ? made : one)) : [...types, made], exists ? 'Tipo salvo' : 'Tipo criado')
          }}
        />
      )}

      <AlertDialog open={deleting !== null} onOpenChange={(open) => !open && setDeleting(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Excluir o tipo {deleting?.label}</AlertDialogTitle>
            <AlertDialogDescription>
              Ele deixa de aparecer em “Começar com”. Os registros criados com ele continuam como estão.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancelar</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                const gone = deleting
                setDeleting(null)
                if (gone) void save(types.filter((one) => one.id !== gone.id), 'Tipo excluído')
              }}
            >
              Excluir tipo
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={resetting} onOpenChange={setResetting}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Restaurar os tipos padrão</AlertDialogTitle>
            <AlertDialogDescription>
              A lista volta a ter Login, Cartão, Chave SSH e Nota, como o Keyhold traz. Os tipos que você criou ou
              alterou são excluídos. Os registros não mudam.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancelar</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                setResetting(false)
                void resetTypes()
                  .then(() => toast('Tipos padrão restaurados'))
                  .catch((cause) => toastError(cause, 'Não foi possível restaurar os tipos'))
              }}
            >
              Restaurar padrões
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Section>
  )
}

function TypeDialog({
  initial,
  onDone,
  onCancel,
}: {
  initial: RecordType | null
  onDone: (type: RecordType) => void
  onCancel: () => void
}) {
  const [draft, setDraft] = useState<RecordType>(
    () => structuredClone(initial) ?? { id: crypto.randomUUID(), label: '', icon: { name: 'file-text', colour: 'stone' }, fields: [] },
  )
  const setFields = (fields: RecordType['fields']) => setDraft({ ...draft, fields })

  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="max-h-[calc(100dvh-48px)] overflow-y-auto sm:max-w-[560px]">
        <DialogHeader>
          <DialogTitle>{initial ? 'Editar tipo de registro' : 'Novo tipo de registro'}</DialogTitle>
          <DialogDescription>
            O nome, o ícone e os campos com que um registro deste tipo começa. Deixe o nome de um campo vazio para
            usar o nome do próprio tipo de campo.
          </DialogDescription>
        </DialogHeader>

        <div className="flex items-end gap-3">
          <div className="grid gap-1.5">
            <span className="text-[13px] font-medium">Ícone</span>
            <IconPicker icon={draft.icon} onChange={(icon) => setDraft({ ...draft, icon })} />
          </div>
          <label className="grid flex-1 gap-1.5">
            <span className="text-[13px] font-medium">Nome</span>
            <Input
              autoFocus
              value={draft.label}
              placeholder="Ex.: Banco de dados"
              onChange={(event) => setDraft({ ...draft, label: event.target.value })}
            />
          </label>
        </div>

        <div className="grid gap-2">
          <span className="text-[13px] font-medium">Campos</span>
          {draft.fields.map((field, index) => (
            <div key={index} className="flex items-center gap-1.5">
              <Select
                value={field.kind}
                onValueChange={(kind) => setFields(draft.fields.map((one, at) => (at === index ? { ...one, kind: kind as FieldKind } : one)))}
              >
                <SelectTrigger className="w-[210px] shrink-0" aria-label="Tipo do campo">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {FIELD_TYPES.map(({ kind, label, icon: Icon }) => (
                    <SelectItem key={kind} value={kind}>
                      <Icon className="size-4 text-muted-foreground" />
                      {label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <Input
                className="min-w-0 flex-1"
                aria-label="Nome do campo"
                value={field.label}
                placeholder={fieldType(field.kind).label}
                onChange={(event) => setFields(draft.fields.map((one, at) => (at === index ? { ...one, label: event.target.value } : one)))}
              />
              <IconAction label="Mover para cima" icon={ArrowUp} disabled={index === 0} onClick={() => setFields(moved(draft.fields, index, -1))} />
              <IconAction
                label="Mover para baixo"
                icon={ArrowDown}
                disabled={index === draft.fields.length - 1}
                onClick={() => setFields(moved(draft.fields, index, 1))}
              />
              <IconAction label="Remover campo" icon={Trash2} onClick={() => setFields(draft.fields.filter((_, at) => at !== index))} />
            </div>
          ))}
          {draft.fields.length === 0 && <p className="text-xs text-faint">Sem campos: o registro começa só com o título.</p>}
          <Button
            variant="outline"
            size="sm"
            className="justify-self-start"
            onClick={() => setFields([...draft.fields, { kind: 'text', label: '' }])}
          >
            <Plus />
            Adicionar campo
          </Button>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onCancel}>
            Cancelar
          </Button>
          <Button disabled={!draft.label.trim()} onClick={() => onDone({ ...draft, label: draft.label.trim() })}>
            {initial ? 'Salvar tipo' : 'Criar tipo'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

// ── Ícones ───────────────────────────────────────────────────────────────────────────────────────

function IconLibrary() {
  const { icons, loaded } = useCustom()
  const [query, setQuery] = useState('')
  const [dragging, setDragging] = useState(false)
  const [deleting, setDeleting] = useState<CustomIcon | null>(null)
  const file = useRef<HTMLInputElement>(null)

  const term = query.trim().toLowerCase()
  const shown = term ? icons.filter((one) => one.name.toLowerCase().includes(term)) : icons

  async function add(files: FileList | File[] | null | undefined) {
    const pictures = [...(files ?? [])].filter((one) => one.type.startsWith('image/'))
    if (pictures.length === 0) {
      toast('Nenhuma imagem para adicionar', 'Use PNG, JPEG, WebP ou SVG.', 'error')

      return
    }

    let added = 0
    let repeated = 0
    for (const picture of pictures) {
      try {
        if ((await addIcon(picture.name.replace(/\.[a-z0-9]+$/i, ''), await shrink(picture))).added) added += 1
        else repeated += 1
      } catch (cause) {
        toastError(cause, `Não foi possível adicionar ${picture.name}`)
      }
    }
    if (added) toast(added === 1 ? 'Ícone adicionado' : `${added} ícones adicionados`)
    if (repeated) toast(repeated === 1 ? 'Uma imagem já estava na biblioteca' : `${repeated} imagens já estavam na biblioteca`)
  }

  const rename = (icon: CustomIcon, name: string) => {
    const next = name.trim()
    if (!next || next === icon.name) return

    void saveIcons(icons.map((one) => (one.id === icon.id ? { ...one, name: next } : one))).catch((cause) =>
      toastError(cause, 'Não foi possível renomear o ícone'),
    )
  }

  return (
    <Section
      title="Ícones personalizados"
      hint={
        <>
          Imagens suas (o logotipo de um serviço, por exemplo) para usar em qualquer registro ou cofre, na aba
          Personalizados do seletor de ícones. Ficam cifradas com a sua chave e nada é buscado na internet. Cada
          registro guarda a própria cópia: excluir um ícone daqui não altera os registros que o usam.
        </>
      }
      actions={
        <Button size="sm" onClick={() => file.current?.click()}>
          <ImageUp />
          Adicionar imagens
        </Button>
      }
    >
      <div
        className={cn('p-5', dragging && 'bg-primary-soft')}
        onDragOver={(event) => {
          event.preventDefault()
          setDragging(true)
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={(event) => {
          event.preventDefault()
          setDragging(false)
          void add(event.dataTransfer.files)
        }}
        onPaste={(event) => {
          const pasted = [...event.clipboardData.files]
          if (pasted.length) {
            event.preventDefault()
            void add(pasted)
          }
        }}
      >
        {icons.length > 8 && (
          <div className="relative mb-4 max-w-xs">
            <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-faint" />
            <Input className="pl-9" placeholder="Buscar ícone" value={query} onChange={(event) => setQuery(event.target.value)} />
          </div>
        )}

        <div className="grid grid-cols-[repeat(auto-fill,minmax(210px,1fr))] gap-2">
          {shown.map((icon) => (
            <div key={icon.id} className="flex items-center gap-2.5 rounded-xl border p-2 transition-colors hover:bg-hover">
              <img src={icon.image} alt="" className="size-10 shrink-0 rounded-[10px] object-cover" />
              <Input
                key={icon.name}
                className="h-8 min-w-0 flex-1 border-transparent bg-transparent px-1.5 shadow-none hover:border-input focus-visible:bg-field"
                aria-label="Nome do ícone"
                defaultValue={icon.name}
                onBlur={(event) => rename(icon, event.target.value)}
                onKeyDown={(event) => event.key === 'Enter' && event.currentTarget.blur()}
              />
              <IconAction label={`Excluir ${icon.name}`} icon={Trash2} onClick={() => setDeleting(icon)} />
            </div>
          ))}
        </div>

        {loaded && icons.length === 0 && (
          <button
            type="button"
            onClick={() => file.current?.click()}
            className="flex w-full flex-col items-center gap-1.5 rounded-xl border border-dashed border-line-2 px-4 py-10 text-center text-[13px] text-muted-foreground hover:bg-hover"
          >
            <ImageUp className="size-5 text-faint" />
            <b className="text-sm font-semibold text-foreground">Sua biblioteca está vazia</b>
            Arraste imagens para cá, cole ou escolha arquivos.
          </button>
        )}
        {icons.length > 0 && shown.length === 0 && (
          <p className="py-6 text-center text-sm text-muted-foreground">Nenhum ícone com esse nome.</p>
        )}
        {icons.length > 0 && (
          <p className="mt-4 text-xs text-faint">Arraste ou cole imagens aqui para adicionar. Clique em um nome para alterar.</p>
        )}

        <input
          ref={file}
          type="file"
          accept="image/*"
          multiple
          hidden
          onChange={(event) => {
            void add(event.target.files)
            event.target.value = ''
          }}
        />
      </div>

      <AlertDialog open={deleting !== null} onOpenChange={(open) => !open && setDeleting(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Excluir o ícone {deleting?.name}</AlertDialogTitle>
            <AlertDialogDescription>
              Ele sai da sua biblioteca. Os registros e cofres que já o usam continuam com a imagem.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancelar</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                const gone = deleting
                setDeleting(null)
                if (gone) {
                  void saveIcons(icons.filter((one) => one.id !== gone.id))
                    .then(() => toast('Ícone excluído'))
                    .catch((cause) => toastError(cause, 'Não foi possível excluir o ícone'))
                }
              }}
            >
              Excluir ícone
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Section>
  )
}
