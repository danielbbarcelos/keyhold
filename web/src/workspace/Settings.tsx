import { useEffect, useState } from 'react'
import {
  Bot,
  KeyRound,
  Loader2,
  Lock,
  RefreshCw,
  ScrollText,
  ShieldCheck,
  Trash2,
  Wand2,
} from 'lucide-react'

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

import { api, ApiError, type LockPolicy, type Status, type VaultSummary } from '../api'
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
}

export function Settings({ status, vaults, onChanged, onVaultsChanged }: SettingsProps) {
  return (
    <div className="mx-auto grid max-w-2xl gap-4 p-6">
      <AutoLock policy={status.lock.policy} onChanged={onChanged} />
      <SecondFactor status={status} onChanged={onChanged} />
      <MasterPassword status={status} onChanged={onChanged} />
      <VaultsPanel vaults={vaults} onChanged={onVaultsChanged} />
      <Rotation vaults={vaults} onChanged={onVaultsChanged} />
      <Retention days={status.retention_days} onChanged={onChanged} />
      <TrailSettings on={status.log_reads} onChanged={onChanged} />
      <TransferPanel vaults={vaults} onChanged={onVaultsChanged} />
      <AgentAccess vaults={vaults} onChanged={onVaultsChanged} />
    </div>
  )
}

const IDLE_CHOICES: Array<[string, number | null]> = [
  ['1 min', 60],
  ['5 min', 300],
  ['15 min', 900],
  ['1 h', 3600],
  ['Nunca', null],
]

const HIDDEN_CHOICES: Array<[string, number | null]> = [
  ['15 s', 15],
  ['1 min', 60],
  ['2 min', 120],
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
          O relógio corre no keyhold, não nesta janela — uma aba fechada ou congelada não estica o
          prazo. Você pode desligar os dois, e o cofre então só tranca quando você mandar.
        </CardDescription>
      </CardHeader>

      <CardContent className="space-y-4 px-6">
        <Choices
          label="Parado por"
          options={IDLE_CHOICES}
          current={policy.idle_seconds}
          onPick={(value) => void save({ idle_seconds: value })}
        />
        <Choices
          label="Janela escondida por"
          options={HIDDEN_CHOICES}
          current={policy.hidden_seconds}
          onPick={(value) => void save({ hidden_seconds: value })}
        />
      </CardContent>
    </Card>
  )
}

function Choices({
  label,
  options,
  current,
  onPick,
}: {
  label: string
  options: Array<[string, number | null]>
  current: number | null
  onPick: (value: number | null) => void
}) {
  return (
    <div className="space-y-2">
      <Label>{label}</Label>
      <div className="flex flex-wrap gap-1.5">
        {options.map(([text, value]) => (
          <Button
            key={text}
            size="sm"
            variant={current === value ? 'default' : 'outline'}
            onClick={() => onPick(value)}
          >
            {text}
          </Button>
        ))}
      </div>
    </div>
  )
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
      setError(cause instanceof ApiError ? cause.message : 'Não foi possível falar com o keyhold.')
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
      setError(cause instanceof ApiError ? cause.message : 'Não foi possível falar com o keyhold.')
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
            ? 'Trocar a senha não mexe em nada que está guardado: o keyhold reembrulha uma chave só.'
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
            Ele executa o comando através do keyhold, e o segredo entra numa variável de ambiente do
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

        {surface && (
          <div className="space-y-2 border-t pt-4">
            <Label>Ligar o Claude Code a este cofre</Label>
            <p className="text-sm text-muted-foreground">
              Rode isto uma vez no terminal. O keyhold não mexe na configuração de outro programa
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

            {surface.active_grants > 0 && (
              <div className="flex items-center justify-between gap-4 pt-2">
                <span className="text-sm text-muted-foreground">
                  {surface.active_grants} permissão(ões) ainda valendo
                </span>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => void api.revokeAgentGrants().then(refresh)}
                >
                  Revogar todas
                </Button>
              </div>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  )
}
