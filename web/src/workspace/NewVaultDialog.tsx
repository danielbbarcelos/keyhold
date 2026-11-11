import { useEffect, useState } from 'react'
import { Check, HardDrive } from 'lucide-react'

import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'

import { api, type AwsConnection, type AwsLogin, type AwsVault, type RecordIcon } from '../api'
import { AwsMark } from './AwsMark'
import { IconPicker } from './IconPicker'
import { VaultInitial } from './shell'

/**
 * A new vault: kept in the local file, or in an AWS account's Parameter Store.
 *
 * An AWS vault is a whole account from `/`; what IAM lets the chosen role read is what shows. The
 * bucket is where attachments go, and Keyhold never creates it: the suggestion follows the name
 * the infrastructure is expected to give it.
 *
 * The icon works like a record's: a lucide icon in a soft colour, or a picture of one's own, sealed
 * with the vault's name. Without one the vault shows its initials, as it always did.
 */
export function NewVaultDialog({
  onCreate,
  onCancel,
  onAccounts,
}: {
  onCreate: (name: string, aws?: AwsVault, icon?: RecordIcon | null) => void
  onCancel: () => void
  onAccounts: () => void
}) {
  const [name, setName] = useState('')
  const [icon, setIcon] = useState<RecordIcon | null>(null)
  const [kind, setKind] = useState<'local' | 'aws'>('local')
  const [connections, setConnections] = useState<AwsConnection[] | null>(null)
  const [logins, setLogins] = useState<AwsLogin[]>([])
  const [connection, setConnection] = useState('')
  const [region, setRegion] = useState('us-east-2')
  const [bucket, setBucket] = useState('')
  const [bucketTouched, setBucketTouched] = useState(false)

  useEffect(() => {
    if (kind === 'aws' && connections === null) {
      void api
        .aws()
        .then((found) => {
          setConnections(found.connections)
          setLogins(found.logins)
          setConnection((current) => current || found.connections[0]?.id || '')
        })
        .catch(() => setConnections([]))
    }
  }, [kind, connections])

  const slug = name
    .normalize('NFD')
    .replace(/[̀-ͯ]/g, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '')
  const suggested = slug ? `keyhold-${slug}` : ''
  const shownBucket = bucketTouched ? bucket : suggested
  const ready = name.trim() !== '' && (kind === 'local' || (connection !== '' && region !== '' && shownBucket !== ''))

  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Novo cofre</DialogTitle>
          <DialogDescription>
            Um cofre é um agrupamento: pode ficar trancado enquanto outro está aberto, e é a unidade que um agente
            pode ou não enxergar.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="grid gap-1.5">
            <span className="text-[13px] font-medium">Ícone e nome</span>
            <div className="flex items-center gap-3">
              <IconPicker
                icon={icon}
                label="Escolher o ícone do cofre"
                onChange={setIcon}
                onClear={() => setIcon(null)}
                clearLabel="Voltar às iniciais"
                trigger={
                  icon ? undefined : (
                    <VaultInitial vault={{ name: name || '?', colour: 'default' }} size="lg" />
                  )
                }
              />
              <Input
                autoFocus
                aria-label="Nome"
                value={name}
                placeholder="Ex.: Trabalho"
                onChange={(event) => setName(event.target.value)}
              />
            </div>
          </div>

          <div className="grid grid-cols-2 gap-2">
            <Kind
              on={kind === 'local'}
              mark={<HardDrive className={cn('size-[18px]', kind === 'local' ? 'text-primary-text' : 'text-muted-foreground')} />}
              title="Cofre local"
              detail="No arquivo desta máquina. Nada sai daqui."
              onClick={() => setKind('local')}
            />
            <Kind
              on={kind === 'aws'}
              mark={<AwsMark className="size-[22px]" />}
              title="AWS Parameter Store"
              detail="Numa conta da AWS. Quem o IAM deixar lê e grava."
              onClick={() => setKind('aws')}
            />
          </div>

          {kind === 'aws' &&
            (connections === null ? (
              <p className="text-sm text-faint">Lendo as contas AWS…</p>
            ) : connections.length === 0 ? (
              <div className="flex flex-col items-center gap-2 rounded-xl border border-dashed border-line-2 px-6 py-6 text-center">
                <AwsMark size="md" />
                <b className="text-sm font-semibold">Nenhuma conta AWS conectada</b>
                <p className="max-w-xs text-xs text-muted-foreground">
                  Conecte um login SSO e escolha as contas dele em Ajustes. Depois é só voltar aqui e escolher a conta.
                </p>
                <Button size="sm" className="mt-1" onClick={onAccounts}>
                  Conectar conta AWS
                </Button>
              </div>
            ) : (
              <div className="space-y-3">
                <div className="grid gap-1.5">
                  <span className="text-[13px] font-medium">Conta</span>
                  <div role="radiogroup" aria-label="Conta" className="grid max-h-[208px] gap-1.5 overflow-y-auto">
                    {connections.map((one) => {
                      const on = one.id === connection
                      const live = logins.find((login) => login.id === one.login_id)?.connected ?? false

                      return (
                        <button
                          key={one.id}
                          type="button"
                          role="radio"
                          aria-checked={on}
                          onClick={() => setConnection(one.id)}
                          className={cn(
                            'flex items-center gap-3 rounded-xl border px-3 py-2.5 text-left hover:bg-hover',
                            on && 'border-primary bg-primary-soft hover:bg-primary-soft',
                          )}
                        >
                          <AwsMark size="sm" />
                          <span className="min-w-0 flex-1">
                            <b className="block truncate text-sm font-semibold">
                              {one.title || one.account_name || one.account_id}
                            </b>
                            <span className="block truncate text-xs text-faint">
                              {one.account_id}, papel {one.role}
                            </span>
                          </span>
                          <span
                            className={cn(
                              'shrink-0 rounded-full px-2 py-px text-[11px] font-medium',
                              live ? 'bg-mint-soft text-mint' : 'bg-stone-soft text-muted-foreground',
                            )}
                          >
                            {live ? 'sessão ativa' : 'sem sessão'}
                          </span>
                          {on && <Check className="size-4 shrink-0 text-primary-text" />}
                        </button>
                      )
                    })}
                  </div>
                </div>
                <div className="grid grid-cols-2 gap-3">
                  <label className="grid gap-1.5">
                    <span className="text-[13px] font-medium">Região</span>
                    <Input value={region} onChange={(event) => setRegion(event.target.value.trim())} />
                  </label>
                  <label className="grid gap-1.5">
                    <span className="text-[13px] font-medium">Bucket dos anexos</span>
                    <Input
                      value={shownBucket}
                      placeholder="keyhold-projeto"
                      onChange={(event) => {
                        setBucketTouched(true)
                        setBucket(event.target.value.trim())
                      }}
                    />
                  </label>
                </div>
                <p className="text-xs text-faint">
                  O Keyhold não cria o bucket. Sem ele, os registros funcionam e só os anexos ficam indisponíveis.
                </p>
              </div>
            ))}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onCancel}>
            Cancelar
          </Button>
          <Button
            disabled={!ready}
            onClick={() =>
              onCreate(
                name.trim(),
                kind === 'aws' ? { connection_id: connection, region, bucket: shownBucket } : undefined,
                icon,
              )
            }
          >
            Criar cofre
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function Kind({
  on,
  mark,
  title,
  detail,
  onClick,
}: {
  on: boolean
  mark: React.ReactNode
  title: string
  detail: string
  onClick: () => void
}) {
  return (
    <button
      type="button"
      aria-pressed={on}
      onClick={onClick}
      className={cn(
        'flex flex-col gap-1.5 rounded-xl border p-3.5 text-left hover:bg-hover',
        on && 'border-primary bg-primary-soft hover:bg-primary-soft',
      )}
    >
      {mark}
      <b className="text-sm font-semibold">{title}</b>
      <span className="text-xs text-muted-foreground">{detail}</span>
    </button>
  )
}
