import { useEffect, useState } from 'react'
import {
  Ban,
  Bell,
  Building2,
  Clock,
  EyeOff,
  Flame,
  Info,
  Lock,
  Plus,
  ShieldAlert,
  Star,
  TriangleAlert,
  Users,
  X,
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { cn } from '@/lib/utils'

import { api, type Alert, type AlertPreset } from '../../api'
import { ICON_COLOURS } from '../icons'
import { toastError } from '../toast'

/** The icons an alert can wear: ones that say "careful", and a few that say who it is for. */
export const ALERT_ICONS: Array<{ name: string; icon: LucideIcon; label: string }> = [
  { name: 'shield-alert', icon: ShieldAlert, label: 'Proteção' },
  { name: 'triangle-alert', icon: TriangleAlert, label: 'Cuidado' },
  { name: 'info', icon: Info, label: 'Informação' },
  { name: 'ban', icon: Ban, label: 'Proibido' },
  { name: 'eye-off', icon: EyeOff, label: 'Sigilo' },
  { name: 'lock', icon: Lock, label: 'Restrito' },
  { name: 'bell', icon: Bell, label: 'Lembrete' },
  { name: 'clock', icon: Clock, label: 'Prazo' },
  { name: 'building-2', icon: Building2, label: 'Empresa' },
  { name: 'users', icon: Users, label: 'Equipe' },
  { name: 'flame', icon: Flame, label: 'Urgente' },
  { name: 'star', icon: Star, label: 'Importante' },
]

const iconOf = (name: string) => ALERT_ICONS.find((one) => one.name === name)?.icon ?? TriangleAlert
const toneOf = (colour: string) => ICON_COLOURS.find((one) => one.value === colour) ?? ICON_COLOURS[5]

// ── the standard alerts, read once and shared ───────────────────────────────────────────────────

const CHANGED = 'keyhold:alert-presets'
let cache: AlertPreset[] | null = null

/** The standard alerts. Read from the binary once, and again whenever Ajustes saves them. */
export function usePresets(): AlertPreset[] {
  const [presets, setPresets] = useState<AlertPreset[]>(cache ?? [])

  useEffect(() => {
    const load = () =>
      void api
        .alertPresets()
        .then((found) => {
          cache = found.presets
          setPresets(found.presets)
        })
        .catch(() => undefined)

    if (cache === null) load()
    else setPresets(cache)

    const again = () => setPresets(cache ?? [])
    window.addEventListener(CHANGED, again)
    return () => window.removeEventListener(CHANGED, again)
  }, [])

  return presets
}

export async function savePresets(presets: AlertPreset[]): Promise<AlertPreset[]> {
  const saved = (await api.saveAlertPresets(presets)).presets
  cache = saved
  window.dispatchEvent(new Event(CHANGED))
  return saved
}

/** What an alert shows: its standard alert as it is now, when this Keyhold knows it; its own copy otherwise. */
export function shown(alert: Alert, presets: AlertPreset[]): Alert {
  const preset = alert.preset ? presets.find((one) => one.id === alert.preset) : undefined
  return preset ? { ...preset, preset: preset.id } : alert
}

export const fromPreset = (preset: AlertPreset): Alert => ({ ...preset, preset: preset.id })

// ── drawing one ──────────────────────────────────────────────────────────────────────────────────

/** One alert, as a soft coloured band with its icon, title and text; with an × when it can go. */
export function AlertBanner({ alert, onRemove, className }: { alert: Alert; onRemove?: () => void; className?: string }) {
  const Icon = iconOf(alert.icon)
  const tone = toneOf(alert.colour)

  return (
    <div role="note" className={cn('flex items-start gap-3 rounded-xl border px-3.5 py-3', className)}>
      <span className={cn('grid size-8 shrink-0 place-items-center rounded-lg', tone.tile)}>
        <Icon className="size-4" />
      </span>
      <span className="min-w-0 flex-1 pt-0.5">
        <b className="block text-[13px] font-semibold">{alert.title}</b>
        {alert.text && <span className="mt-0.5 block text-[13px] text-muted-foreground">{alert.text}</span>}
      </span>
      {onRemove && (
        <Button variant="ghost" size="icon-xs" aria-label={`Remover o alerta ${alert.title}`} title="Remover alerta" onClick={onRemove}>
          <X />
        </Button>
      )}
    </div>
  )
}

/** A record's alerts, at the top of the record. */
export function RecordAlerts({ alerts, onRemove }: { alerts: Alert[]; onRemove?: (index: number) => void }) {
  const presets = usePresets()
  if (alerts.length === 0) return null

  return (
    <div className="grid gap-2">
      {alerts.map((alert, index) => (
        <AlertBanner key={index} alert={shown(alert, presets)} onRemove={onRemove ? () => onRemove(index) : undefined} />
      ))}
    </div>
  )
}

// ── writing one ──────────────────────────────────────────────────────────────────────────────────

/** Title, text, colour and icon of an alert: for a standard one in Ajustes, or a one-off. */
export function AlertForm({
  initial,
  offerSave = false,
  onDone,
  onCancel,
}: {
  initial?: Partial<AlertPreset>
  /** A one-off alert can also become a standard one. */
  offerSave?: boolean
  onDone: (alert: AlertPreset, keep: boolean) => void
  onCancel: () => void
}) {
  const [title, setTitle] = useState(initial?.title ?? '')
  const [text, setText] = useState(initial?.text ?? '')
  const [colour, setColour] = useState(initial?.colour ?? 'rose')
  const [icon, setIcon] = useState(initial?.icon ?? 'shield-alert')
  const [keep, setKeep] = useState(false)

  return (
    <div className="grid gap-3 rounded-xl border bg-raised p-3.5">
      <AlertBanner alert={{ title: title || 'Título do alerta', text, colour, icon }} className="bg-card" />
      <Input value={title} autoFocus placeholder="Ex.: Não compartilhar com terceiros" onChange={(event) => setTitle(event.target.value)} />
      <Textarea className="min-h-[60px]" value={text} placeholder="Descrição (opcional)" onChange={(event) => setText(event.target.value)} />
      <div className="flex flex-wrap items-center gap-1.5">
        {ICON_COLOURS.map((one) => (
          <button
            key={one.value}
            type="button"
            aria-label={one.label}
            aria-pressed={colour === one.value}
            title={one.label}
            onClick={() => setColour(one.value)}
            className={cn('size-6 rounded-full ring-offset-2 ring-offset-raised', one.dot, colour === one.value && 'ring-2 ring-foreground')}
          />
        ))}
      </div>
      <div className="flex flex-wrap gap-1">
        {ALERT_ICONS.map((one) => (
          <button
            key={one.name}
            type="button"
            aria-label={one.label}
            aria-pressed={icon === one.name}
            title={one.label}
            onClick={() => setIcon(one.name)}
            className={cn(
              'grid size-8 place-items-center rounded-lg border text-muted-foreground hover:bg-hover',
              icon === one.name && 'border-transparent bg-primary-soft text-primary-text',
            )}
          >
            <one.icon className="size-4" />
          </button>
        ))}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        {offerSave && (
          <label className="mr-auto flex items-center gap-2 text-[13px]">
            <Checkbox checked={keep} onCheckedChange={(on) => setKeep(on === true)} />
            Salvar como alerta padrão
          </label>
        )}
        <span className={cn('flex gap-2', !offerSave && 'ml-auto')}>
          <Button variant="ghost" size="sm" onClick={onCancel}>
            Cancelar
          </Button>
          <Button
            size="sm"
            disabled={!title.trim()}
            onClick={() =>
              onDone({ id: initial?.id ?? newId(title), title: title.trim(), text: text.trim(), colour, icon }, keep)
            }
          >
            {initial?.id ? 'Salvar' : 'Adicionar'}
          </Button>
        </span>
      </div>
    </div>
  )
}

function newId(title: string): string {
  const base = title
    .normalize('NFD')
    .replace(/[̀-ͯ]/g, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '')
    .slice(0, 40)
  return `${base || 'alerta'}-${Math.random().toString(36).slice(2, 7)}`
}

/** The alerts of a record being edited: add from the standard ones, write a one-off, take any out. */
export function AlertsEditor({ alerts, onChange }: { alerts: Alert[]; onChange: (alerts: Alert[]) => void }) {
  const presets = usePresets()
  const [writing, setWriting] = useState(false)
  const used = new Set(alerts.map((alert) => alert.preset).filter(Boolean))

  return (
    <div className="grid gap-2 px-5 pt-4 pb-5">
      {alerts.map((alert, index) => (
        <AlertBanner
          key={index}
          alert={shown(alert, presets)}
          onRemove={() => onChange(alerts.filter((_, at) => at !== index))}
        />
      ))}
      {writing ? (
        <AlertForm
          offerSave
          onCancel={() => setWriting(false)}
          onDone={(made, keep) => {
            setWriting(false)
            if (keep) {
              void savePresets([...presets, made])
                .then(() => onChange([...alerts, fromPreset(made)]))
                .catch((cause) => toastError(cause, 'Não foi possível salvar o alerta padrão'))
            } else {
              onChange([...alerts, { colour: made.colour, icon: made.icon, title: made.title, text: made.text }])
            }
          }}
        />
      ) : (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline" size="sm" className="justify-self-start">
              <Plus />
              Adicionar alerta
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="w-72">
            {presets.map((preset) => {
              const Icon = iconOf(preset.icon)
              return (
                <DropdownMenuItem key={preset.id} disabled={used.has(preset.id)} onSelect={() => onChange([...alerts, fromPreset(preset)])}>
                  <span className={cn('grid size-6 place-items-center rounded-md', toneOf(preset.colour).tile)}>
                    <Icon className="size-3.5" />
                  </span>
                  <span className="truncate">{preset.title}</span>
                </DropdownMenuItem>
              )
            })}
            {presets.length > 0 && <DropdownMenuSeparator />}
            <DropdownMenuItem onSelect={() => setWriting(true)}>
              <Plus />
              Alerta personalizado
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      )}
    </div>
  )
}

/** Ajustes → Alertas: the standard alerts, each editable, removable, and a new one. */
export function AlertPresetsPanel() {
  const presets = usePresets()
  const [editing, setEditing] = useState<string | 'new' | null>(null)

  const save = (next: AlertPreset[]) => void savePresets(next).catch((cause) => toastError(cause, 'Não foi possível salvar os alertas'))

  return (
    <section className="rounded-[14px] border bg-card">
      <div className="border-b px-5 py-4">
        <h3 className="text-[15px] font-semibold">Alertas padrão</h3>
        <p className="mt-1 max-w-2xl text-[13px] text-muted-foreground">
          Avisos exibidos no topo de um registro, como “Não compartilhar com terceiros”. Alterar um alerta padrão
          altera todos os registros que o usam. Os alertas são cifrados com a sua chave.
        </p>
      </div>
      <div className="grid gap-2 p-5">
        {presets.map((preset) =>
          editing === preset.id ? (
            <AlertForm
              key={preset.id}
              initial={preset}
              onCancel={() => setEditing(null)}
              onDone={(made) => {
                setEditing(null)
                save(presets.map((one) => (one.id === made.id ? made : one)))
              }}
            />
          ) : (
            <div key={preset.id} className="flex items-center gap-2">
              <AlertBanner alert={preset} className="flex-1" />
              <Button variant="outline" size="sm" onClick={() => setEditing(preset.id)}>
                Editar
              </Button>
              <Button variant="ghost" size="sm" onClick={() => save(presets.filter((one) => one.id !== preset.id))}>
                Remover
              </Button>
            </div>
          ),
        )}
        {editing === 'new' ? (
          <AlertForm
            onCancel={() => setEditing(null)}
            onDone={(made) => {
              setEditing(null)
              save([...presets, made])
            }}
          />
        ) : (
          <Button variant="outline" size="sm" className="justify-self-start" onClick={() => setEditing('new')}>
            <Plus />
            Novo alerta padrão
          </Button>
        )}
        <p className="text-xs text-faint">
          Remover um alerta padrão não remove o aviso dos registros. Eles passam a exibir a cópia armazenada.
        </p>
      </div>
    </section>
  )
}
