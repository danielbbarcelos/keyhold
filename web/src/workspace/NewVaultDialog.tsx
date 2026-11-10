import { useEffect, useState } from 'react'
import { Cloud, HardDrive } from 'lucide-react'
import type { LucideIcon } from 'lucide-react'

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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { cn } from '@/lib/utils'

import { api, type AwsConnection, type AwsVault } from '../api'

/**
 * A new vault: kept in the local file, or in an AWS account's Parameter Store.
 *
 * An AWS vault is a whole account from `/`; what IAM lets the chosen role read is what shows. The
 * bucket is where attachments go, and Keyhold never creates it: the suggestion follows the name
 * the infrastructure is expected to give it.
 */
export function NewVaultDialog({
  onCreate,
  onCancel,
  onAccounts,
}: {
  onCreate: (name: string, aws?: AwsVault) => void
  onCancel: () => void
  onAccounts: () => void
}) {
  const [name, setName] = useState('')
  const [kind, setKind] = useState<'local' | 'aws'>('local')
  const [connections, setConnections] = useState<AwsConnection[] | null>(null)
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
          <label className="grid gap-1.5">
            <span className="text-[13px] font-medium">Nome</span>
            <Input autoFocus value={name} placeholder="Ex.: Trabalho" onChange={(event) => setName(event.target.value)} />
          </label>

          <div className="grid grid-cols-2 gap-2">
            <Kind on={kind === 'local'} icon={HardDrive} title="Cofre local" detail="No arquivo desta máquina. Nada sai daqui." onClick={() => setKind('local')} />
            <Kind
              on={kind === 'aws'}
              icon={Cloud}
              title="AWS Parameter Store"
              detail="Numa conta da AWS. Quem o IAM deixar lê e grava."
              onClick={() => setKind('aws')}
            />
          </div>

          {kind === 'aws' &&
            (connections === null ? (
              <p className="text-sm text-faint">Lendo as contas AWS…</p>
            ) : connections.length === 0 ? (
              <div className="rounded-xl border bg-raised p-4 text-sm">
                <p className="text-muted-foreground">Nenhuma conta AWS conectada ainda. Primeiro, um login SSO e uma conta em Ajustes.</p>
                <Button variant="outline" size="sm" className="mt-3" onClick={onAccounts}>
                  Abrir Contas AWS
                </Button>
              </div>
            ) : (
              <div className="space-y-3">
                <label className="grid gap-1.5">
                  <span className="text-[13px] font-medium">Conta</span>
                  <Select value={connection} onValueChange={setConnection}>
                    <SelectTrigger className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {connections.map((one) => (
                        <SelectItem key={one.id} value={one.id}>
                          {one.title || `${one.account_id} / ${one.role}`}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </label>
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
            onClick={() => onCreate(name.trim(), kind === 'aws' ? { connection_id: connection, region, bucket: shownBucket } : undefined)}
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
  icon: Icon,
  title,
  detail,
  onClick,
}: {
  on: boolean
  icon: LucideIcon
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
      <Icon className={cn('size-[18px]', on ? 'text-primary-text' : 'text-muted-foreground')} />
      <b className="text-sm font-semibold">{title}</b>
      <span className="text-xs text-muted-foreground">{detail}</span>
    </button>
  )
}
