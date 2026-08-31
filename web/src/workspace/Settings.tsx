import { useState } from 'react'
import { Bot, KeyRound, Loader2, Lock, RefreshCw, ShieldCheck, Trash2, Wand2 } from 'lucide-react'

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
 * Which vaults a local agent is allowed to see (F7).
 *
 * Off for every vault until it is switched on, and the switch is *visibility* rather than
 * permission: what is not marked is never listed to an agent, so there is nothing for it to ask
 * about. The warning is permanent because the trade is permanent — a value handed to an assistant
 * leaves this machine, which is the opposite of everything else keyhold promises.
 */
function AgentAccess({ vaults, onChanged }: { vaults: VaultSummary[]; onChanged: () => void }) {
  return (
    <Card className="py-6">
      <CardHeader className="px-6">
        <CardTitle className="flex items-center gap-2">
          <Bot className="size-4" />
          Acesso por agente
          <Badge variant="outline">ainda não conectável</Badge>
        </CardTitle>
        <CardDescription>
          Marcar um cofre aqui é dizer, de antemão, o que um assistente rodando nesta máquina poderá
          consultar. <strong className="text-foreground">A ligação ainda não existe</strong>: hoje
          isto só guarda a sua escolha, e nada consegue ler o cofre por fora desta janela.
        </CardDescription>
      </CardHeader>

      <CardContent className="space-y-4 px-6">
        <div className="space-y-2 rounded-lg border bg-muted/30 p-3 text-sm">
          <p className="font-medium">O que falta para funcionar</p>
          <p className="text-muted-foreground">
            Um canal local entre o keyhold e o assistente, e uma pergunta sua a cada leitura: qual
            processo pediu, qual registro, qual campo — com resposta válida uma vez, ou por alguns
            minutos. Enquanto isso não existe, esta marcação não abre nada.
          </p>
        </div>

        <p className="rounded-lg border border-destructive/40 bg-destructive/10 p-3 text-sm">
          Quando existir, vale saber desde já: todo valor entregue a um assistente entra no contexto
          do modelo e sai desta máquina. É o oposto do que o resto do keyhold promete, e por isso a
          via preferida será executar o comando com o segredo, em vez de mostrá-lo.
        </p>

        <div className="space-y-3">
          {vaults
            .filter((vault) => !vault.archived)
            .map((vault) => (
              <div key={vault.id} className="flex items-center justify-between gap-4">
                <Label htmlFor={`agent-${vault.id}`} className="font-normal">
                  {vault.name}
                </Label>
                <span className="flex items-center gap-2">
                  {vault.agent_access && (
                    <span className="text-xs text-muted-foreground">marcado, sem efeito ainda</span>
                  )}
                  <Switch
                    id={`agent-${vault.id}`}
                    checked={vault.agent_access}
                    onCheckedChange={(on) => void api.setAgentAccess(vault.id, on).then(onChanged)}
                  />
                </span>
              </div>
            ))}
        </div>
      </CardContent>
    </Card>
  )
}
