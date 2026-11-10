import { useCallback, useEffect, useRef, useState } from 'react'
import {
  ArrowLeftRight,
  Bell,
  BellOff,
  Bot,
  Check,
  Cloud,
  KeyRound,
  Loader2,
  Lock,
  Palette,
  RefreshCw,
  ScrollText,
  ShieldCheck,
  Timer,
  Trash2,
  Vault,
  Wand2,
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'

import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { cn } from '@/lib/utils'

import { PasswordInput } from '@/components/common/PasswordInput'

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

import {
  api,
  ApiError,
  type Accent,
  type AgentGrant,
  type Appearance,
  type LockPolicy,
  type Theme,
  type Status,
  type VaultSummary,
} from '../api'
import { askToNotify, notifyState, type NotifyState } from '../notify'
import { AwsAccounts } from './AwsAccounts'
import { TransferPanel } from './TransferPanel'
import { VaultsPanel } from './VaultsPanel'

/**
 * Everything the vault decides about itself: the two clocks, the second factor, the master
 * password, and which vaults a local agent may ever see.
 *
 * Each of these is a layer somebody chooses. keyhold marks the ones it recommends and states what
 * each costs — a vault that refuses to open on its owner's terms is a vault that ends up as a text
 * file on the desktop, which protects nothing at all.
 */
interface SettingsProps {
  status: Status
  vaults: VaultSummary[]
  onChanged: () => void
  onVaultsChanged: () => void
  /** Tells the top bar which section is being read. */
  onSection?: (label: string) => void
}

const SECTIONS: Array<{ id: string; label: string; icon: LucideIcon }> = [
  { id: 'aparencia', label: 'Aparência', icon: Palette },
  { id: 'tranca', label: 'Tranca automática', icon: Timer },
  { id: 'segundo-fator', label: 'Segundo fator', icon: ShieldCheck },
  { id: 'senha', label: 'Senha-mestra', icon: KeyRound },
  { id: 'cofres', label: 'Cofres', icon: Vault },
  { id: 'aws', label: 'Contas AWS', icon: Cloud },
  { id: 'chaves', label: 'Trocar a chave', icon: RefreshCw },
  { id: 'lixeira', label: 'Lixeira', icon: Trash2 },
  { id: 'trilha', label: 'Trilha', icon: ScrollText },
  { id: 'exportar', label: 'Exportar e importar', icon: ArrowLeftRight },
  { id: 'agentes', label: 'Agentes', icon: Bot },
]

/**
 * The sections are cards in their own files, and they are flattened here rather than one by one:
 * a page of settings reads as one document with dividers, not a stack of boxes.
 */
const FLAT =
  '[&_[data-slot=card]]:gap-4 [&_[data-slot=card]]:rounded-[14px] [&_[data-slot=card]]:border [&_[data-slot=card]]:bg-card [&_[data-slot=card]]:py-5 [&_[data-slot=card]]:shadow-none [&_[data-slot=card]]:ring-0 ' +
  '[&_[data-slot=card-header]]:px-5 [&_[data-slot=card-content]]:px-5 [&_[data-slot=card-footer]]:border-0 [&_[data-slot=card-footer]]:bg-transparent [&_[data-slot=card-footer]]:px-5 [&_[data-slot=card-footer]]:pt-1 ' +
  '[&_[data-slot=card-title]]:text-[15px] [&_[data-slot=card-title]]:font-semibold [&_[data-slot=card-title]>svg]:hidden [&_[data-slot=card-description]]:max-w-2xl [&_[data-slot=card-description]]:text-[13px]'

export function Settings({ status, vaults, onChanged, onVaultsChanged, onSection }: SettingsProps) {
  const [active, setActive] = useState(SECTIONS[0].id)

  useEffect(() => {
    onSection?.(SECTIONS.find((one) => one.id === active)?.label ?? '')
  }, [active, onSection])
  const page = useRef<HTMLDivElement>(null)

  // Light up the section being read. The one nearest the top of the page wins.
  useEffect(() => {
    const root = page.current

    if (!root) {
      return
    }

    const watcher = new IntersectionObserver(
      (entries) => {
        const top = entries
          .filter((entry) => entry.isIntersecting)
          .sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top)[0]

        if (top) {
          setActive(top.target.id.replace('ajuste-', ''))
        }
      },
      { root, rootMargin: '0px 0px -70% 0px' },
    )

    root.querySelectorAll('section[id^="ajuste-"]').forEach((section) => watcher.observe(section))

    return () => watcher.disconnect()
  }, [])

  const sections: Record<string, React.ReactNode> = {
    aparencia: <AppearanceSettings appearance={status.appearance} onChanged={onChanged} />,
    tranca: <AutoLock policy={status.lock.policy} onChanged={onChanged} />,
    'segundo-fator': <SecondFactor status={status} onChanged={onChanged} />,
    senha: <MasterPassword status={status} onChanged={onChanged} />,
    cofres: <VaultsPanel vaults={vaults} onChanged={onVaultsChanged} />,
    aws: <AwsAccounts vaults={vaults} />,
    chaves: <Rotation vaults={vaults} onChanged={onVaultsChanged} />,
    lixeira: <Retention days={status.retention_days} onChanged={onChanged} />,
    trilha: <TrailSettings on={status.log_reads} onChanged={onChanged} />,
    exportar: <TransferPanel vaults={vaults} onChanged={onVaultsChanged} />,
    agentes: <AgentAccess vaults={vaults} onChanged={onVaultsChanged} />,
  }

  return (
    <div className="grid min-h-0 flex-1 grid-cols-[232px_minmax(0,1fr)] grid-rows-[minmax(0,1fr)] gap-5 p-6 max-xl:grid-cols-[200px_minmax(0,1fr)]">
      <nav aria-label="Seções dos ajustes" className="h-fit space-y-0.5 rounded-[14px] border bg-card p-2.5">
        {SECTIONS.map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            type="button"
            aria-current={active === id}
            onClick={() => {
              setActive(id)
              document.getElementById(`ajuste-${id}`)?.scrollIntoView({ block: 'start', behavior: 'smooth' })
            }}
            className={cn(
              'relative flex h-9 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-sm whitespace-nowrap text-muted-foreground transition-colors hover:bg-hover hover:text-foreground',
              active === id &&
                'bg-raised text-foreground ring-1 ring-border before:absolute before:top-2 before:bottom-2 before:-left-2.5 before:w-[3px] before:rounded-r before:bg-primary',
            )}
          >
            <Icon className="size-4" />
            {label}
          </button>
        ))}
      </nav>

      <div ref={page} className="min-h-0 overflow-y-auto">
        <div className={cn('space-y-5 pb-24', FLAT)}>
          {SECTIONS.map(({ id }) => (
            <section key={id} id={`ajuste-${id}`} className="scroll-mt-0">
              {sections[id]}
            </section>
          ))}
        </div>
      </div>
    </div>
  )
}

const THEMES: Array<{ value: Theme; label: string; rail: string; side: string; page: string; ink: string }> = [
  { value: 'grafite', label: 'Grafite', rail: '#18191c', side: '#1e1f23', page: '#24252a', ink: '#e6e6e9' },
  { value: 'claro', label: 'Claro', rail: '#ebebee', side: '#f3f3f5', page: '#ffffff', ink: '#1f2024' },
]

const ACCENTS: Array<{ value: Accent; label: string; colour: string }> = [
  { value: 'ambar', label: 'Âmbar', colour: '#e8b04b' },
  { value: 'oceano', label: 'Oceano', colour: '#86aee0' },
  { value: 'floresta', label: 'Floresta', colour: '#62be97' },
  { value: 'coral', label: 'Coral', colour: '#e2788b' },
]

/**
 * Theme and accent. Saved by the binary, so the unlock screen opens in the same colours, and applied
 * the moment it is picked rather than after a round trip.
 */
function AppearanceSettings({
  appearance,
  onChanged,
}: {
  appearance: Appearance
  onChanged: () => void
}) {
  async function save(patch: Partial<Appearance>) {
    await api.setAppearance({ ...appearance, ...patch })
    onChanged()
  }

  return (
    <Card className="py-6">
      <CardHeader className="px-6">
        <CardTitle>Aparência</CardTitle>
        <CardDescription>Vale para este Keyhold. Nada disso fica no conteúdo dos cofres.</CardDescription>
      </CardHeader>

      <CardContent className="space-y-7 px-6">
        <div className="space-y-2.5">
          <h3 className="text-sm font-bold">Tema</h3>
          <div className="flex flex-wrap gap-3">
            {THEMES.map((theme) => (
              <ThemeCard
                key={theme.value}
                label={theme.label}
                chosen={appearance.theme === theme.value}
                onPick={() => void save({ theme: theme.value })}
              >
                <Preview {...theme} />
              </ThemeCard>
            ))}
            <ThemeCard
              label="Igual ao sistema"
              chosen={appearance.theme === 'sistema'}
              onPick={() => void save({ theme: 'sistema' })}
            >
              <span className="grid h-full grid-cols-2">
                <span className="bg-[#1b1d21]" />
                <span className="bg-white" />
              </span>
            </ThemeCard>
          </div>
        </div>

        <div className="space-y-2.5">
          <h3 className="text-sm font-bold">Cor de destaque</h3>
          <p className="text-sm text-muted-foreground">
            A cor do botão principal, do item selecionado e do foco.
          </p>
          <div className="flex flex-wrap gap-3">
            {ACCENTS.map((accent) => {
              const chosen = appearance.accent === accent.value

              return (
                <button
                  key={accent.value}
                  type="button"
                  aria-pressed={chosen}
                  onClick={() => void save({ accent: accent.value })}
                  className={cn(
                    'flex h-11 w-36 items-center gap-3 rounded-xl border px-3 text-sm font-semibold transition-colors hover:bg-hover',
                    chosen && 'border-ring ring-1 ring-ring',
                  )}
                >
                  <span
                    className="grid size-5 place-items-center rounded-full text-white"
                    style={{ background: accent.colour }}
                  >
                    {chosen && <Check className="size-3" strokeWidth={3} />}
                  </span>
                  {accent.label}
                </button>
              )
            })}
          </div>
        </div>
      </CardContent>
    </Card>
  )
}

function ThemeCard({
  label,
  chosen,
  onPick,
  children,
}: {
  label: string
  chosen: boolean
  onPick: () => void
  children: React.ReactNode
}) {
  return (
    <button
      type="button"
      aria-pressed={chosen}
      onClick={onPick}
      className={cn(
        'w-44 overflow-hidden rounded-xl border text-left transition-colors hover:border-input',
        chosen && 'border-ring ring-1 ring-ring',
      )}
    >
      <span className="block h-20 border-b">{children}</span>
      <span className="flex items-center gap-2 px-3 py-2.5 text-sm font-semibold">
        <span
          className={cn(
            'size-4 shrink-0 rounded-full border-[1.5px] border-input',
            chosen && 'border-[5px] border-ring',
          )}
        />
        {label}
      </span>
    </button>
  )
}

function Preview({ rail, side, page, ink }: { rail: string; side: string; page: string; ink: string }) {
  return (
    <span className="grid h-full grid-cols-[1.25rem_3rem_1fr]">
      <span style={{ background: rail }} />
      <span style={{ background: side }} />
      <span className="flex flex-col gap-1.5 p-3" style={{ background: page }}>
        <span className="h-1.5 w-3/5 rounded-full bg-primary" />
        <span className="h-1.5 w-4/5 rounded-full opacity-25" style={{ background: ink }} />
        <span className="h-1.5 w-2/5 rounded-full opacity-25" style={{ background: ink }} />
      </span>
    </span>
  )
}

/**
 * The same presets for both clocks.
 *
 * They used to differ — the hidden one stopped at two minutes, on the theory that a window you
 * cannot see is a window you are not using. It is not: reading a password and typing it into
 * another tab is the ordinary way to use a vault, and a fuse that short turns the ordinary case
 * into a re-unlock. The two clocks answer the same question, so they offer the same answers.
 */
const CHOICES: Array<[string, number | null]> = [
  ['1 minuto', 60],
  ['5 minutos', 300],
  ['15 minutos', 900],
  ['1 hora', 3600],
  ['Nunca', null],
]

function AutoLock({ policy, onChanged }: { policy: LockPolicy; onChanged: () => void }) {
  async function save(patch: Partial<LockPolicy>) {
    await api.setLockPolicy({ ...policy, ...patch })
    onChanged()
  }

  return (
    <Card className="py-6">
      <CardHeader className="px-6">
        <CardTitle className="flex items-center gap-2">
          <Lock className="size-4" />
          Tranca automática
          <Badge>recomendado</Badge>
        </CardTitle>
        <CardDescription>
          O relógio corre no Keyhold, não nesta janela — uma aba fechada ou congelada não estica o
          prazo. Você pode desligar os dois, e o cofre então só tranca quando você mandar.
        </CardDescription>
      </CardHeader>

      <CardContent className="px-6">
        <Choices
          label="Trancar depois de parado por"
          hint="Sem nenhum clique ou tecla no Keyhold"
          current={policy.idle_seconds}
          onPick={(value) => void save({ idle_seconds: value })}
        />
        <Choices
          label="Trancar com a janela escondida por"
          hint="Aba em segundo plano ou janela minimizada"
          current={policy.hidden_seconds}
          onPick={(value) => void save({ hidden_seconds: value })}
        />
      </CardContent>
    </Card>
  )
}

/**
 * The presets, plus whatever is set now if it is not one of them.
 *
 * A vault written before these presets changed can hold a value no button offers, and a row of
 * buttons with none of them lit reads as "off" — which is the opposite of what it means.
 */
function Choices({
  label,
  hint,
  current,
  onPick,
}: {
  label: string
  hint: string
  current: number | null
  onPick: (value: number | null) => void
}) {
  const options: Array<[string, number | null]> = (() => {
    if (current === null || CHOICES.some(([, value]) => value === current)) {
      return CHOICES
    }

    const timed: Array<[string, number]> = [
      ...CHOICES.filter((choice): choice is [string, number] => choice[1] !== null),
      [minutes(current), current],
    ]

    timed.sort((a, b) => a[1] - b[1])

    return [...timed, ['Nunca', null] as [string, null]]
  })()

  return (
    <div className="flex items-center justify-between gap-6 border-t py-3.5 first:border-t-0">
      <div className="min-w-0">
        <div className="text-sm font-semibold">{label}</div>
        <div className="text-xs text-muted-foreground">{hint}</div>
      </div>
      <Select
        value={current === null ? 'never' : String(current)}
        onValueChange={(value) => onPick(value === 'never' ? null : Number(value))}
      >
        <SelectTrigger className="w-48 shrink-0" aria-label={label}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {options.map(([text, value]) => (
            <SelectItem key={text} value={value === null ? 'never' : String(value)}>
              {text}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  )
}

function minutes(seconds: number): string {
  if (seconds < 60) {
    return `${seconds} s`
  }

  if (seconds % 3600 === 0) {
    return seconds === 3600 ? '1 hora' : `${seconds / 3600} horas`
  }

  const whole = Math.round(seconds / 60)

  return whole === 1 ? '1 minuto' : `${whole} minutos`
}

function SecondFactor({ status, onChanged }: { status: Status; onChanged: () => void }) {
  const [secret, setSecret] = useState('')
  const [qr, setQr] = useState('')
  const [code, setCode] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)

  const on = status.identity.second_factor

  async function attempt(action: () => Promise<void>) {
    setBusy(true)

    try {
      await action()
      setError('')
    } catch (cause) {
      setError(cause instanceof ApiError ? cause.message : 'Não foi possível falar com o Keyhold.')
    } finally {
      setBusy(false)
    }
  }

  return (
    <Card className="py-6">
      <CardHeader className="px-6">
        <CardTitle className="flex items-center gap-2">
          <ShieldCheck className="size-4" />
          Segundo fator
          <Badge variant="secondary">opcional</Badge>
        </CardTitle>
        <CardDescription>
          Um código do seu app autenticador, pedido junto com a senha. Protege contra quem descobriu
          a sua senha mas não tem o seu telefone. Não protege o arquivo do cofre: quem levar o
          arquivo e quebrar a senha leva o segredo do código junto.
        </CardDescription>
      </CardHeader>

      <CardContent className="space-y-4 px-6">
        {on ? (
          <div className="space-y-2">
            <Label htmlFor="off-code">Para desligar, informe um código atual</Label>
            <div className="flex items-center gap-2">
              <Input
                id="off-code"
                className="w-32 tracking-[0.3em]"
                value={code}
                inputMode="numeric"
                maxLength={6}
                placeholder="000000"
                onChange={(event) => setCode(event.target.value)}
              />
              <Button
                variant="outline"
                disabled={busy}
                onClick={() =>
                  void attempt(async () => {
                    await api.disableSecondFactor(code)
                    setCode('')
                    onChanged()
                  })
                }
              >
                Desligar
              </Button>
            </div>
          </div>
        ) : qr ? (
          <div className="space-y-3">
            <img
              src={qr}
              alt="Código QR para o app autenticador"
              className="size-48 rounded-lg bg-white p-2"
            />

            <p className="text-sm text-muted-foreground">
              Se preferir digitar: <span className="tracking-wider text-foreground">{secret}</span>
            </p>

            <div className="flex items-center gap-2">
              <Input
                className="w-32 tracking-[0.3em]"
                value={code}
                inputMode="numeric"
                maxLength={6}
                placeholder="000000"
                autoFocus
                onChange={(event) => setCode(event.target.value)}
              />
              <Button
                disabled={busy}
                onClick={() =>
                  void attempt(async () => {
                    await api.enableSecondFactor(secret, code)
                    setSecret('')
                    setQr('')
                    setCode('')
                    onChanged()
                  })
                }
              >
                {busy && <Loader2 className="animate-spin" />}
                Ligar
              </Button>
            </div>
          </div>
        ) : (
          <Button
            variant="outline"
            disabled={busy}
            onClick={() =>
              void attempt(async () => {
                const started = await api.beginSecondFactor()
                setSecret(started.secret)
                setQr(started.qr)
              })
            }
          >
            Ligar o segundo fator
          </Button>
        )}

        {error && <p className="text-sm text-destructive">{error}</p>}
      </CardContent>
    </Card>
  )
}

function MasterPassword({ status, onChanged }: { status: Status; onChanged: () => void }) {
  const [password, setPassword] = useState('')
  const [error, setError] = useState('')
  const [done, setDone] = useState('')

  const hasPassword = status.identity.protection === 'password'

  async function apply(next: string | null) {
    try {
      await api.changeProtection(next)
      setPassword('')
      setDone(next === null ? 'Senha removida.' : 'Senha trocada.')
      setError('')
      onChanged()
    } catch (cause) {
      setError(cause instanceof ApiError ? cause.message : 'Não foi possível falar com o Keyhold.')
      setDone('')
    }
  }

  return (
    <Card className="py-6">
      <CardHeader className="px-6">
        <CardTitle className="flex items-center gap-2">
          <KeyRound className="size-4" />
          Senha-mestra
          {hasPassword ? <Badge variant="secondary">ativa</Badge> : <Badge>recomendada</Badge>}
        </CardTitle>
        <CardDescription>
          {hasPassword
            ? 'Trocar a senha não mexe em nada que está guardado: o Keyhold reembrulha uma chave só.'
            : 'Este cofre abre sem senha. Definir uma agora passa a proteger o arquivo por criptografia, e não só por permissão de arquivo.'}
        </CardDescription>
      </CardHeader>

      <CardContent className="space-y-2 px-6">
        <Label htmlFor="new-password">{hasPassword ? 'Nova senha' : 'Senha-mestra'}</Label>
        <PasswordInput
          id="new-password"
          value={password}
          onChange={(event) => setPassword(event.target.value)}
        />

        {done && <p className="text-sm text-muted-foreground">{done}</p>}
        {error && <p className="text-sm text-destructive">{error}</p>}
      </CardContent>

      <CardFooter className="gap-2 px-6 pt-2">
        <Button
          variant="outline"
          onClick={() => void api.generatePassword().then((made) => setPassword(made.password))}
        >
          <Wand2 />
          Gerar
        </Button>
        <Button disabled={password.trim().length === 0} onClick={() => void apply(password)}>
          {hasPassword ? 'Trocar' : 'Definir'}
        </Button>
        {hasPassword && (
          <Button variant="ghost" onClick={() => void apply(null)}>
            Remover a senha
          </Button>
        )}
      </CardFooter>
    </Card>
  )
}

/**
 * Rotating a vault's key (F5).
 *
 * The key that wraps every record's key is the one value whose exposure would matter beyond a
 * single secret. Rotating replaces it and re-wraps what hung off it — no payload is re-encrypted,
 * which is why this is cheap enough to do on a suspicion rather than only after a certainty.
 */
function Rotation({ vaults, onChanged }: { vaults: VaultSummary[]; onChanged: () => void }) {
  const [rotating, setRotating] = useState<VaultSummary | null>(null)
  const [busy, setBusy] = useState(false)

  return (
    <Card className="py-6">
      <CardHeader className="px-6">
        <CardTitle className="flex items-center gap-2">
          <RefreshCw className="size-4" />
          Trocar a chave de um cofre
        </CardTitle>
        <CardDescription>
          Gera uma chave nova para o cofre e reembrulha tudo que dependia da antiga. Nada guardado é
          re-cifrado, então é barato — dá para fazer por suspeita, sem esperar uma certeza.
        </CardDescription>
      </CardHeader>

      <CardContent className="space-y-3 px-6">
        {vaults
          .filter((vault) => !vault.archived)
          .map((vault) => (
            <div key={vault.id} className="flex items-center justify-between gap-4">
              <span className="min-w-0">
                <span className="block truncate text-sm">{vault.name}</span>
                <span className="block text-xs text-muted-foreground">
                  {vault.key_rotated_at
                    ? `geração ${vault.key_version}, trocada em ${vault.key_rotated_at.slice(0, 10)}`
                    : 'na chave original'}
                </span>
              </span>

              <Button variant="outline" size="sm" onClick={() => setRotating(vault)}>
                Trocar
              </Button>
            </div>
          ))}
      </CardContent>

      <AlertDialog open={rotating !== null} onOpenChange={(open) => !open && setRotating(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Trocar a chave de {rotating?.name}?</AlertDialogTitle>
            <AlertDialogDescription>
              Tudo que está no cofre continua legível — só os envelopes mudam. A troca acontece de
              uma vez só: ou todos os registros passam para a chave nova, ou nenhum passa.
            </AlertDialogDescription>
          </AlertDialogHeader>

          <AlertDialogFooter>
            <AlertDialogCancel>Cancelar</AlertDialogCancel>
            <AlertDialogAction
              disabled={busy}
              onClick={() => {
                if (!rotating) {
                  return
                }

                setBusy(true)
                void api
                  .rotateVault(rotating.id)
                  .then(() => {
                    setRotating(null)
                    onChanged()
                  })
                  .finally(() => setBusy(false))
              }}
            >
              Trocar a chave
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Card>
  )
}

const RETENTION_CHOICES: Array<[string, number]> = [
  ['7 dias', 7],
  ['30 dias', 30],
  ['90 dias', 90],
  ['Nunca', 0],
]

/** How long the trash keeps what was deleted (F5). */
function Retention({ days, onChanged }: { days: number; onChanged: () => void }) {
  return (
    <Card className="py-6">
      <CardHeader className="px-6">
        <CardTitle className="flex items-center gap-2">
          <Trash2 className="size-4" />
          Lixeira
        </CardTitle>
        <CardDescription>
          Excluir um registro não apaga nada de imediato: ele espera aqui, e some no primeiro
          destranque depois do prazo. Com "Nunca", nada é apagado sem você mandar.
        </CardDescription>
      </CardHeader>

      <CardContent className="space-y-2 px-6">
        <Label>Guardar o que foi excluído por</Label>
        <div className="flex flex-wrap gap-1.5">
          {RETENTION_CHOICES.map(([label, value]) => (
            <Button
              key={label}
              size="sm"
              variant={days === value ? 'default' : 'outline'}
              onClick={() => void api.setRetention(value).then(onChanged)}
            >
              {label}
            </Button>
          ))}
        </div>
      </CardContent>
    </Card>
  )
}

/**
 * Whether reads go into the trail.
 *
 * On by default, because "what did the agent read?" is the question the trail exists to answer.
 * Switchable off, because with it on the vault also keeps a record of *your* use of it — and on a
 * machine only you touch, that is a log of you. Writes and security events are recorded either way.
 */
function TrailSettings({ on, onChanged }: { on: boolean; onChanged: () => void }) {
  return (
    <Card className="py-6">
      <CardHeader className="px-6">
        <CardTitle className="flex items-center gap-2">
          <ScrollText className="size-4" />
          Trilha
        </CardTitle>
        <CardDescription>
          Destrancar, criar, alterar e excluir são sempre registrados — é o que permite conferir
          depois se alguma coisa aconteceu sem você. Revelar e copiar um segredo é opcional.
        </CardDescription>
      </CardHeader>

      <CardContent className="space-y-4 px-6">
        <div className="flex items-center justify-between gap-4">
          <Label htmlFor="log-reads" className="font-normal">
            Registrar quando um segredo é revelado ou copiado
          </Label>
          <Switch
            id="log-reads"
            checked={on}
            onCheckedChange={(next) => void api.setLogReads(next).then(onChanged)}
          />
        </div>

        {!on && (
          <p className="text-sm text-muted-foreground">
            Com isto desligado, não haverá como responder depois o que um assistente leu.
          </p>
        )}
      </CardContent>
    </Card>
  )
}

/**
 * Permission to knock on the desktop when an agent asks for something.
 *
 * It lives here, next to the switch that opens the door, because that is the moment the question
 * makes sense: a request expires in ninety seconds, and nobody watching a terminal sees a dialog
 * in a window behind it. Asked from a click and never on load — a page that asks cold is a page
 * whose prompts people learn to dismiss.
 */
function DesktopKnock() {
  const [state, setState] = useState<NotifyState>(() => notifyState())

  if (state === 'unsupported') {
    return null
  }

  return (
    <div className="flex items-start justify-between gap-4">
      <Label className="font-normal">
        Avisar na área de trabalho
        <span className="mt-1 block text-sm text-muted-foreground">
          {state === 'granted'
            ? 'Um pedido aparece como notificação do sistema, com o processo e o campo, nunca com o valor.'
            : state === 'denied'
              ? 'O navegador está bloqueando as notificações deste endereço. Libere nas permissões do site para voltar a receber.'
              : 'Um pedido expira em 90 segundos. Sem isto, ele só aparece se esta janela estiver à vista.'}
        </span>
      </Label>

      {state === 'granted' ? (
        <Badge variant="secondary" className="shrink-0 gap-1">
          <Bell className="size-3" />
          ligado
        </Badge>
      ) : state === 'denied' ? (
        <Badge variant="secondary" className="shrink-0 gap-1">
          <BellOff className="size-3" />
          bloqueado
        </Badge>
      ) : (
        <Button
          variant="outline"
          size="sm"
          className="shrink-0"
          onClick={() => void askToNotify().then(setState)}
        >
          Permitir
        </Button>
      )}
    </div>
  )
}

/**
 * What the agent may take without being asked again.
 *
 * A standing permission is a decision to stop being asked, so the screen that offers it also has to
 * be the screen that shows what was agreed and takes it back. Each row says what it covers — one
 * field of one record, never more — and when it ends.
 */
function StandingGrants({ onChanged }: { onChanged: () => void }) {
  const [grants, setGrants] = useState<AgentGrant[] | null>(null)

  const load = useCallback(async () => {
    try {
      setGrants((await api.agentGrants()).grants)
    } catch {
      setGrants([])
    }
  }, [])

  // Permissions are granted from a dialog that can open over this screen, so the list is refreshed
  // rather than fetched once: a "revoke" button next to a stale list revokes the wrong thing.
  useEffect(() => {
    void load()
    const timer = window.setInterval(() => void load(), 5000)

    return () => window.clearInterval(timer)
  }, [load])

  const revoke = async (id?: string) => {
    await (id ? api.revokeAgentGrant(id) : api.revokeAgentGrants())
    await load()
    onChanged()
  }

  if (!grants || grants.length === 0) {
    return null
  }

  return (
    <div className="space-y-3 border-t pt-4">
      <div className="flex items-center justify-between gap-4">
        <Label>Permissões valendo</Label>
        <Button variant="outline" size="sm" onClick={() => void revoke()}>
          Revogar todas
        </Button>
      </div>

      <p className="text-sm text-muted-foreground">
        Enquanto uma destas estiver de pé, o pedido correspondente não abre nenhuma pergunta. O uso
        continua indo para a trilha.
      </p>

      <ul className="space-y-1">
        {grants.map((grant) => (
          <li
            key={grant.id}
            className="flex items-center justify-between gap-3 rounded-lg border p-3"
          >
            <div className="min-w-0 space-y-0.5">
              <p className="truncate text-sm">
                <span className="text-foreground">{grant.field}</span>
                <span className="text-muted-foreground"> de </span>
                <span className="text-foreground">{grant.record_title || grant.record_id}</span>
              </p>
              <p className="text-xs text-muted-foreground">{untilWhen(grant)}</p>
            </div>

            <Button variant="ghost" size="sm" onClick={() => void revoke(grant.id)}>
              Revogar
            </Button>
          </li>
        ))}
      </ul>
    </div>
  )
}

function untilWhen(grant: AgentGrant): string {
  if (grant.scope.kind === 'always') {
    return 'Sem prazo, só acaba quando você revogar'
  }

  if (grant.scope.kind === 'while-running') {
    return `Enquanto ${grant.scope.process} (PID ${grant.scope.pid}) estiver aberto`
  }

  const at = new Date(grant.scope.at * 1000)
  const left = Math.max(0, grant.scope.at - Math.floor(Date.now() / 1000))
  const remaining =
    left >= 86400
      ? `${Math.floor(left / 86400)} d`
      : left >= 3600
        ? `${Math.floor(left / 3600)} h`
        : `${Math.max(1, Math.floor(left / 60))} min`

  return `Até ${at.toLocaleString('pt-BR', { dateStyle: 'short', timeStyle: 'short' })} — faltam ${remaining}`
}

/**
 * The agent surface: which vaults it may see, and whether the door is open at all.
 *
 * Two switches on purpose. Marking a vault says what *could* be reached; opening the socket says
 * whether anything can reach it right now. Closing the socket removes the file, so an agent finds
 * nothing to connect to rather than something that refuses — the difference between an integration
 * that is off and one that looks broken.
 */
function AgentAccess({ vaults, onChanged }: { vaults: VaultSummary[]; onChanged: () => void }) {
  const [surface, setSurface] = useState<{
    listening: boolean
    socket: string
    exposed_vaults: number
    may_reveal: boolean
    can_copy: boolean
    active_grants: number
    command: string
  } | null>(null)
  const [copied, setCopied] = useState(false)

  const refresh = () =>
    api
      .agentStatus()
      .then(setSurface)
      .catch(() => setSurface(null))

  useEffect(() => {
    void refresh()
  }, [])

  const marked = vaults.filter((vault) => !vault.archived && vault.agent_access).length

  return (
    <Card className="py-6">
      <CardHeader className="px-6">
        <CardTitle className="flex items-center gap-2">
          <Bot className="size-4" />
          Acesso por agente
          {surface?.listening ? <Badge>ligado</Badge> : <Badge variant="secondary">desligado</Badge>}
        </CardTitle>
        <CardDescription>
          Um assistente rodando nesta máquina pode procurar e pedir segredos dos cofres marcados
          aqui. Cada pedido de valor vira uma pergunta nesta janela, com o processo e o PID à vista.
        </CardDescription>
      </CardHeader>

      <CardContent className="space-y-4 px-6">
        <div className="space-y-2 rounded-lg border bg-muted/30 p-3 text-sm">
          <p className="font-medium">Como um assistente usa uma credencial sem lê-la</p>
          <p className="text-muted-foreground">
            Ele executa o comando através do Keyhold, e o segredo entra numa variável de ambiente do
            processo — nunca no texto da conversa:
          </p>
          <code className="block rounded-md border bg-background px-3 py-2 text-xs">
            keyhold run "Banco de dados" Senha -- psql "$KEYHOLD_SECRET"
          </code>
          <p className="text-muted-foreground">
            Para quando é <span className="text-foreground">você</span> quem precisa da senha, ele
            usa <span className="text-foreground">keyhold_copy</span>: o valor vai para a sua área de
            transferência e ele recebe só a confirmação.
          </p>
        </div>

        <div className="flex items-start justify-between gap-4">
          <Label htmlFor="agent-reveal" className="font-normal">
            Deixar o assistente ler valores
            <span className="mt-1 block text-sm text-muted-foreground">
              Desligado, ele nunca recebe uma senha — e por isso não tem como escrevê-la na conversa.
              Ligado, o valor entra no histórico da sessão dele assim que chega, e nada aqui decide o
              que ele faz depois.
            </span>
          </Label>
          <Switch
            id="agent-reveal"
            checked={surface?.may_reveal ?? false}
            onCheckedChange={(on) => void api.setAgentMayReveal(on).then(refresh)}
          />
        </div>

        <div className="flex items-center justify-between gap-4">
          <Label htmlFor="agent-surface" className="font-normal">
            Aceitar pedidos de um assistente local
          </Label>
          <Switch
            id="agent-surface"
            checked={surface?.listening ?? false}
            disabled={marked === 0}
            onCheckedChange={(on) => void api.setAgentSurface(on).then(refresh)}
          />
        </div>

        {marked === 0 && (
          <p className="text-sm text-muted-foreground">
            Marque ao menos um cofre abaixo antes de abrir a porta.
          </p>
        )}

        <DesktopKnock />

        <div className="space-y-3 border-t pt-4">
          <Label>Cofres ao alcance</Label>

          {vaults
            .filter((vault) => !vault.archived)
            .map((vault) => (
              <div key={vault.id} className="flex items-center justify-between gap-4">
                <Label htmlFor={`agent-${vault.id}`} className="font-normal">
                  {vault.name}
                </Label>
                <Switch
                  id={`agent-${vault.id}`}
                  checked={vault.agent_access}
                  onCheckedChange={(on) =>
                    void api.setAgentAccess(vault.id, on).then(() => {
                      onChanged()
                      void refresh()
                    })
                  }
                />
              </div>
            ))}
        </div>

        <StandingGrants onChanged={refresh} />

        {surface && (
          <div className="space-y-2 border-t pt-4">
            <Label>Ligar o Claude Code a este cofre</Label>
            <p className="text-sm text-muted-foreground">
              Rode isto uma vez no terminal. O Keyhold não mexe na configuração de outro programa
              sozinho.
            </p>

            <div className="flex items-center gap-2">
              <code className="min-w-0 flex-1 truncate rounded-md border bg-muted/40 px-3 py-2 text-xs">
                {surface.command}
              </code>
              <Button
                variant="outline"
                size="sm"
                onClick={() => {
                  void navigator.clipboard.writeText(surface.command)
                  setCopied(true)
                  window.setTimeout(() => setCopied(false), 2000)
                }}
              >
                {copied ? 'Copiado' : 'Copiar'}
              </Button>
            </div>


          </div>
        )}
      </CardContent>
    </Card>
  )
}
