import { useCallback, useEffect, useRef, useState } from 'react'
import { Copy, ExternalLink, Loader2, LogOut, Pencil, Plus, RefreshCw, Trash2 } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { cn } from '@/lib/utils'

import { api, type AwsConnection, type AwsLogin, type DeviceLogin, type VaultSummary } from '../api'
import { AwsMark } from './AwsMark'
import { toast, toastError } from './toast'

/**
 * Contas AWS: the SSO logins Keyhold holds a session for, and the accounts reached through them.
 *
 * Grouped by login because the session belongs to the login: Renovar and Revogar act on every
 * account behind it. The session lives only in memory, so after Keyhold restarts or locks each
 * login shows as disconnected until it is renewed.
 */
export function AwsAccounts({ vaults }: { vaults: VaultSummary[] }) {
  const [logins, setLogins] = useState<AwsLogin[] | null>(null)
  const [connections, setConnections] = useState<AwsConnection[]>([])
  const [adding, setAdding] = useState(false)
  const [device, setDevice] = useState<{ login: AwsLogin; device: DeviceLogin } | null>(null)
  const [connecting, setConnecting] = useState<AwsLogin | null>(null)
  const [editing, setEditing] = useState<AwsConnection | null>(null)
  const [failed, setFailed] = useState('')

  const refresh = useCallback(async () => {
    try {
      const found = await api.aws()
      setLogins(found.logins)
      setConnections(found.connections)
      setFailed('')
      announceAwsChange()
    } catch (cause) {
      setLogins([])
      setFailed(cause instanceof Error ? cause.message : 'Não foi possível carregar as contas AWS.')
    }
  }, [])

  useEffect(() => void refresh(), [refresh])

  async function renew(login: AwsLogin) {
    try {
      const answer = await api.renewAwsLogin(login.id)
      if (answer.device) {
        setDevice({ login: answer.login, device: answer.device })
      } else {
        toast('Sessão renovada', login.label || login.start_url)
        void refresh()
      }
    } catch (cause) {
      toastError(cause, 'Não foi possível renovar a sessão')
    }
  }

  const vaultName = (id: string) => vaults.find((vault) => vault.id === id)?.name ?? 'um cofre'

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          Contas AWS
          <span className="rounded-full bg-sky-soft px-2 py-px text-[11px] font-medium text-sky">novo</span>
        </CardTitle>
        <CardDescription>
          Para cofres armazenados no AWS Parameter Store. O login é feito pelo SSO do IAM Identity Center,
          no próprio Keyhold. A sessão fica apenas na memória e é encerrada quando o Keyhold é fechado ou
          trancado. Apenas o conteúdo de um cofre AWS sai desta máquina, e somente para a conta dele.
        </CardDescription>
      </CardHeader>

      <CardContent className="space-y-4">
        {failed && <p className="text-sm text-rose">{failed}</p>}

        {logins?.map((login) => {
          const mine = connections.filter((one) => one.login_id === login.id)

          return (
            <div key={login.id} className="overflow-hidden rounded-xl border">
              <div className="flex flex-wrap items-center gap-3 bg-raised px-4 py-3">
                <AwsMark size="sm" />
                <span className="min-w-0 flex-1">
                  <b className="block truncate text-sm font-semibold">{login.label || login.start_url}</b>
                  <span className="block truncate text-xs text-faint">
                    {login.start_url}, {login.region}
                  </span>
                </span>
                <span
                  className={cn(
                    'rounded-full px-2.5 py-0.5 text-xs font-medium',
                    login.connected ? 'bg-mint-soft text-mint' : 'bg-stone-soft text-muted-foreground',
                  )}
                >
                  {login.connected
                    ? login.expires_at
                      ? `Conectado até ${new Date(login.expires_at * 1000).toLocaleTimeString('pt-BR', { hour: '2-digit', minute: '2-digit' })}`
                      : 'Conectado'
                    : 'Desconectado'}
                </span>
                <Button variant="outline" size="sm" onClick={() => void renew(login)}>
                  <RefreshCw />
                  Renovar
                </Button>
                {login.connected && (
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() =>
                      void api
                        .revokeAwsLogin(login.id)
                        .then(() => {
                          toast('Sessão revogada', 'O SSO foi desconectado. As contas continuam listadas.')
                          void refresh()
                        })
                        .catch((cause) => toastError(cause, 'Não foi possível revogar a sessão'))
                    }
                  >
                    <LogOut />
                    Revogar
                  </Button>
                )}
                {mine.length === 0 && (
                  <Button
                    variant="ghost"
                    size="icon"
                    aria-label="Remover login"
                    onClick={() =>
                      void api
                        .removeAwsLogin(login.id)
                        .then(refresh)
                        .catch((cause) => toastError(cause, 'Não foi possível remover o login'))
                    }
                  >
                    <Trash2 />
                  </Button>
                )}
              </div>

              {mine.map((connection) => (
                <div key={connection.id} className="flex items-center gap-3 border-t px-4 py-3 text-sm">
                  <span className="min-w-0 flex-1">
                    <b className="block truncate font-medium">{connection.title || `${connection.account_id} / ${connection.role}`}</b>
                    <span className="block truncate text-xs text-faint">
                      {connection.account_name ? `${connection.account_name}, ` : ''}
                      {connection.account_id}, papel {connection.role}
                      {connection.used_by.length > 0 &&
                        `, usado por ${connection.used_by.map(vaultName).join(', ')}`}
                    </span>
                    {connection.note && <span className="block truncate text-xs text-muted-foreground">{connection.note}</span>}
                  </span>
                  <Button variant="ghost" size="icon" aria-label="Editar conta" onClick={() => setEditing(connection)}>
                    <Pencil />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    aria-label="Remover conta"
                    disabled={connection.used_by.length > 0}
                    title={connection.used_by.length > 0 ? 'Conta em uso por um cofre' : 'Remover conta'}
                    onClick={() =>
                      void api
                        .removeAwsConnection(connection.id)
                        .then(refresh)
                        .catch((cause) => toastError(cause, 'Não foi possível remover a conta'))
                    }
                  >
                    <Trash2 />
                  </Button>
                </div>
              ))}

              <button
                type="button"
                disabled={!login.connected}
                onClick={() => setConnecting(login)}
                className="flex h-10 w-full items-center gap-2 border-t px-4 text-sm font-medium text-muted-foreground hover:bg-hover hover:text-foreground disabled:opacity-50"
              >
                <Plus className="size-4" />
                {login.connected ? 'Adicionar conta deste login' : 'Renove a sessão para adicionar contas'}
              </button>
            </div>
          )
        })}

        {logins?.length === 0 && !failed && (
          <p className="text-sm text-faint">Nenhum login SSO adicionado.</p>
        )}

        <Button variant="outline" onClick={() => setAdding(true)}>
          <Plus />
          Adicionar login SSO
        </Button>
      </CardContent>

      {adding && (
        <NewLoginDialog
          onStarted={(started) => {
            setAdding(false)
            setDevice(started)
          }}
          onCancel={() => setAdding(false)}
        />
      )}

      {device && (
        <DeviceDialog
          login={device.login}
          device={device.device}
          onDone={() => {
            setDevice(null)
            toast('Login SSO conectado', device.login.label || device.login.start_url)
            void refresh()
          }}
          onCancel={() => {
            setDevice(null)
            void refresh()
          }}
        />
      )}

      {connecting && (
        <ConnectionDialog
          login={connecting}
          taken={connections.map((one) => one.account_id)}
          onDone={() => {
            setConnecting(null)
            void refresh()
          }}
          onCancel={() => setConnecting(null)}
        />
      )}

      {editing && (
        <EditConnectionDialog
          connection={editing}
          onDone={() => {
            setEditing(null)
            void refresh()
          }}
          onCancel={() => setEditing(null)}
        />
      )}
    </Card>
  )
}

function NewLoginDialog({
  onStarted,
  onCancel,
}: {
  onStarted: (started: { login: AwsLogin; device: DeviceLogin }) => void
  onCancel: () => void
}) {
  const [startUrl, setStartUrl] = useState('')
  const [region, setRegion] = useState('us-east-1')
  const [label, setLabel] = useState('')
  const [busy, setBusy] = useState(false)

  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Adicionar login SSO</DialogTitle>
          <DialogDescription>
            O endereço de acesso do IAM Identity Center (o “AWS access portal”) e a região onde ele foi criado.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <label className="grid gap-1.5">
            <span className="text-[13px] font-medium">Endereço do portal</span>
            <Input autoFocus value={startUrl} placeholder="minha-empresa.awsapps.com/start" onChange={(event) => setStartUrl(event.target.value)} />
          </label>
          <div className="grid grid-cols-2 gap-3">
            <label className="grid gap-1.5">
              <span className="text-[13px] font-medium">Região do SSO</span>
              <Input value={region} onChange={(event) => setRegion(event.target.value.trim())} />
            </label>
            <label className="grid gap-1.5">
              <span className="text-[13px] font-medium">Nome (opcional)</span>
              <Input value={label} placeholder="Ex.: Empresa" onChange={(event) => setLabel(event.target.value)} />
            </label>
          </div>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onCancel}>
            Cancelar
          </Button>
          <Button
            disabled={busy || !startUrl.trim() || !region}
            onClick={() => {
              setBusy(true)
              const url = /^[a-z]+:/i.test(startUrl.trim()) ? startUrl.trim() : `${['https', '//'].join(':')}${startUrl.trim()}`
              void api
                .startAwsLogin(url, region, label.trim())
                .then((answer) => {
                  if (answer.device) onStarted({ login: answer.login, device: answer.device })
                  else {
                    toast('Login SSO conectado', answer.login.label || answer.login.start_url)
                    onCancel()
                  }
                })
                .catch((cause) => toastError(cause, 'O SSO não respondeu'))
                .finally(() => setBusy(false))
            }}
          >
            {busy && <Loader2 className="animate-spin" />}
            Continuar
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

/** Said whenever logins change here, so the rail's session clock reads them again. */
export const AWS_CHANGED = 'keyhold:aws-changed'

export function announceAwsChange() {
  window.dispatchEvent(new Event(AWS_CHANGED))
}

/**
 * Renew an SSO session from anywhere: the rail, a copy that stopped on an expired session. When
 * AWS wants the device code confirmed, the code dialog opens right there, over whatever asked, so
 * nothing in progress is thrown away to go and find it.
 */
export function useAwsRenew(onRenewed: () => void) {
  const [pending, setPending] = useState<{ login: AwsLogin; device: DeviceLogin } | null>(null)
  const [busy, setBusy] = useState(false)
  const renewed = useRef(onRenewed)
  renewed.current = onRenewed

  async function renew(login: AwsLogin) {
    setBusy(true)
    try {
      const answer = await api.renewAwsLogin(login.id)
      if (answer.device) {
        setPending({ login: answer.login, device: answer.device })
      } else {
        toast('Sessão renovada', login.label || login.start_url)
        announceAwsChange()
        renewed.current()
      }
    } catch (cause) {
      toastError(cause, 'Não foi possível renovar a sessão')
    } finally {
      setBusy(false)
    }
  }

  const dialog = pending && (
    <DeviceDialog
      login={pending.login}
      device={pending.device}
      onDone={() => {
        toast('Login SSO conectado', pending.login.label || pending.login.start_url)
        setPending(null)
        announceAwsChange()
        renewed.current()
      }}
      onCancel={() => setPending(null)}
    />
  )

  return { renew, busy, dialog }
}

/** The device-code step: show the code, open the AWS page, and wait for the yes. */
export function DeviceDialog({
  login,
  device,
  onDone,
  onCancel,
}: {
  login: AwsLogin
  device: DeviceLogin
  onDone: () => void
  onCancel: () => void
}) {
  const [state, setState] = useState<'waiting' | 'denied' | 'expired'>('waiting')
  const interval = useRef(Math.max(device.interval, 2))
  // The parent re-renders every second (the lock clock); keeping the callback in a ref stops each
  // render from cancelling the next poll before it runs.
  const done = useRef(onDone)
  done.current = onDone

  useEffect(() => {
    let alive = true
    let timer = 0

    const poll = async () => {
      try {
        const answer = await api.pollAwsLogin(login.id)
        if (!alive) return
        if (answer.state === 'done') return done.current()
        if (answer.state === 'denied' || answer.state === 'expired') return setState(answer.state)
        if (answer.state === 'slow_down') interval.current += 5
        timer = window.setTimeout(poll, interval.current * 1000)
      } catch {
        if (alive) timer = window.setTimeout(poll, interval.current * 1000)
      }
    }

    timer = window.setTimeout(poll, interval.current * 1000)

    return () => {
      alive = false
      window.clearTimeout(timer)
    }
  }, [login.id])

  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Confirme na página da AWS</DialogTitle>
          <DialogDescription>
            Abra a página, confira se o código abaixo é o mesmo e permita o acesso. Esta janela avança
            automaticamente após a confirmação.
          </DialogDescription>
        </DialogHeader>
        <div className="grid place-items-center gap-3 rounded-xl border bg-raised py-5">
          <span className="font-mono text-2xl tracking-[0.2em]">{device.user_code}</span>
          <div className="flex flex-wrap justify-center gap-2">
            <Button
              disabled={!device.verification_uri_complete.startsWith(['https', '//'].join(':'))}
              onClick={() => window.open(device.verification_uri_complete, '_blank', 'noopener,noreferrer')}
            >
              <ExternalLink />
              Abrir a página da AWS
            </Button>
            {/* For confirming in another browser or profile, the one already signed in to AWS. The
                address is not a secret: it only works together with a login on AWS's own page. */}
            <Button
              variant="outline"
              onClick={() =>
                void navigator.clipboard
                  .writeText(device.verification_uri_complete)
                  .then(() => toast('Link copiado', 'Cole no navegador em que você entra na AWS.'))
                  .catch((cause) => toastError(cause, 'Não foi possível copiar o link'))
              }
            >
              <Copy />
              Copiar link
            </Button>
          </div>
        </div>
        <p className="flex items-center gap-2 text-sm text-muted-foreground">
          {state === 'waiting' && (
            <>
              <Loader2 className="size-4 animate-spin" />
              Aguardando confirmação…
            </>
          )}
          {state === 'denied' && <span className="text-rose">O acesso foi negado na página da AWS.</span>}
          {state === 'expired' && <span className="text-rose">O código expirou. Feche e tente novamente.</span>}
        </p>
        <DialogFooter>
          <Button variant="outline" onClick={onCancel}>
            {state === 'waiting' ? 'Cancelar' : 'Fechar'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function ConnectionDialog({
  login,
  taken,
  onDone,
  onCancel,
}: {
  login: AwsLogin
  taken: string[]
  onDone: () => void
  onCancel: () => void
}) {
  const [accounts, setAccounts] = useState<Array<{ account_id: string; account_name: string; roles: string[] }> | null>(null)
  const [account, setAccount] = useState('')
  const [role, setRole] = useState('')
  const [title, setTitle] = useState('')
  const [note, setNote] = useState('')

  useEffect(() => {
    void api
      .awsAccounts(login.id)
      .then((answer) => setAccounts(answer.accounts))
      .catch((cause) => {
        toastError(cause, 'Não foi possível carregar as contas deste login')
        setAccounts([])
      })
  }, [login.id])

  const chosen = accounts?.find((one) => one.account_id === account)

  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Adicionar conta</DialogTitle>
          <DialogDescription>A conta da AWS e o papel que o Keyhold usa para acessá-la.</DialogDescription>
        </DialogHeader>
        {accounts === null ? (
          <p className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="size-4 animate-spin" />
            Carregando as contas…
          </p>
        ) : (
          <div className="space-y-3">
            <label className="grid gap-1.5">
              <span className="text-[13px] font-medium">Conta</span>
              <Select
                value={account}
                onValueChange={(value) => {
                  setAccount(value)
                  setRole(accounts.find((one) => one.account_id === value)?.roles[0] ?? '')
                }}
              >
                <SelectTrigger className="w-full">
                  <SelectValue placeholder="Selecione…" />
                </SelectTrigger>
                <SelectContent>
                  {accounts.map((one) => (
                    <SelectItem key={one.account_id} value={one.account_id} disabled={taken.includes(one.account_id)}>
                      {one.account_name || one.account_id} ({one.account_id})
                      {taken.includes(one.account_id) && ', já adicionada'}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
            {chosen && (
              <label className="grid gap-1.5">
                <span className="text-[13px] font-medium">Papel</span>
                <Select value={role} onValueChange={setRole}>
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {chosen.roles.map((one) => (
                      <SelectItem key={one} value={one}>
                        {one}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>
            )}
            <label className="grid gap-1.5">
              <span className="text-[13px] font-medium">Título (opcional)</span>
              <Input value={title} placeholder={chosen ? `${chosen.account_id} / ${role}` : ''} onChange={(event) => setTitle(event.target.value)} />
            </label>
            <label className="grid gap-1.5">
              <span className="text-[13px] font-medium">Observação interna</span>
              <Textarea className="min-h-16" value={note} onChange={(event) => setNote(event.target.value)} />
            </label>
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={onCancel}>
            Cancelar
          </Button>
          <Button
            disabled={!chosen || !role}
            onClick={() =>
              void api
                .addAwsConnection({
                  login_id: login.id,
                  account_id: account,
                  account_name: chosen?.account_name ?? '',
                  role,
                  title: title.trim(),
                  note: note.trim(),
                })
                .then(() => {
                  toast('Conta adicionada', title || `${account} / ${role}`)
                  onDone()
                })
                .catch((cause) => toastError(cause, 'Não foi possível adicionar a conta'))
            }
          >
            Adicionar
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function EditConnectionDialog({
  connection,
  onDone,
  onCancel,
}: {
  connection: AwsConnection
  onDone: () => void
  onCancel: () => void
}) {
  const [role, setRole] = useState(connection.role)
  const [title, setTitle] = useState(connection.title)
  const [note, setNote] = useState(connection.note)

  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Editar conta {connection.account_id}</DialogTitle>
          <DialogDescription>O papel define o que o Keyhold pode ler e gravar nos cofres desta conta.</DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <label className="grid gap-1.5">
            <span className="text-[13px] font-medium">Papel</span>
            <Input value={role} onChange={(event) => setRole(event.target.value.trim())} />
          </label>
          <label className="grid gap-1.5">
            <span className="text-[13px] font-medium">Título</span>
            <Input value={title} onChange={(event) => setTitle(event.target.value)} />
          </label>
          <label className="grid gap-1.5">
            <span className="text-[13px] font-medium">Observação interna</span>
            <Textarea className="min-h-16" value={note} onChange={(event) => setNote(event.target.value)} />
          </label>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onCancel}>
            Cancelar
          </Button>
          <Button
            disabled={!role}
            onClick={() =>
              void api
                .updateAwsConnection(connection.id, { role, title: title.trim(), note: note.trim() })
                .then(onDone)
                .catch((cause) => toastError(cause, 'Não foi possível salvar a conta'))
            }
          >
            Salvar
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
