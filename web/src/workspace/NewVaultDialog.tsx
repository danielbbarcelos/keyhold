import { useEffect, useState } from 'react'
import { HardDrive } from 'lucide-react'

import { Combobox } from '@/components/common/Combobox'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
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
import { t } from '../i18n'
import { AwsMark } from './AwsMark'
import { IconPicker } from './IconPicker'
import { VaultInitial } from './shell'

/**
 * A new vault: kept in the local file, or in an AWS account's Parameter Store.
 *
 * An AWS vault is an account from its path down: the whole account from `/` when the path is left
 * empty, or one branch of it (`/shared`) for a role allowed only that. What IAM lets the role read
 * is what shows. The bucket is where attachments go, and Keyhold never creates it: the suggestion
 * follows the name the infrastructure is expected to give it.
 *
 * Account, region, path and bucket are fixed once the vault exists, so the dialog asks for a tick
 * saying so before it creates one: a path typed wrong is a vault made again, not one edited.
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
  const [root, setRoot] = useState('')
  const [understood, setUnderstood] = useState(false)

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
  const ready =
    name.trim() !== '' &&
    (kind === 'local' || (connection !== '' && region !== '' && shownBucket !== '' && understood))

  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{t('Novo cofre')}</DialogTitle>
          <DialogDescription>
            {t(
              'Um cofre agrupa registros. Ele pode ficar trancado enquanto outro está aberto, e o acesso por agente é definido por cofre.',
            )}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="grid gap-1.5">
            <span className="text-[13px] font-medium">{t('Ícone e nome')}</span>
            <div className="flex items-center gap-3">
              <IconPicker
                icon={icon}
                label={t('Escolher ícone do cofre')}
                onChange={setIcon}
                onClear={() => setIcon(null)}
                clearLabel={t('Voltar às iniciais')}
                trigger={
                  icon ? undefined : (
                    <VaultInitial vault={{ name: name || '?', colour: 'default' }} size="lg" />
                  )
                }
              />
              <Input
                autoFocus
                aria-label={t('Nome')}
                value={name}
                placeholder={t('Ex.: Trabalho')}
                onChange={(event) => setName(event.target.value)}
              />
            </div>
          </div>

          <div className="grid grid-cols-2 gap-2">
            <Kind
              on={kind === 'local'}
              mark={<HardDrive className={cn('size-[18px]', kind === 'local' ? 'text-primary-text' : 'text-muted-foreground')} />}
              title={t('Cofre local')}
              detail={t('No arquivo desta máquina. O conteúdo não sai daqui.')}
              onClick={() => setKind('local')}
            />
            <Kind
              on={kind === 'aws'}
              mark={<AwsMark className="size-[22px]" />}
              title="AWS Parameter Store"
              detail={t('Em uma conta da AWS. Quem o IAM autorizar pode ler e gravar.')}
              onClick={() => setKind('aws')}
            />
          </div>

          {kind === 'aws' &&
            (connections === null ? (
              <p className="text-sm text-faint">{t('Carregando as contas AWS…')}</p>
            ) : connections.length === 0 ? (
              <div className="flex flex-col items-center gap-2 rounded-xl border border-dashed border-line-2 px-6 py-6 text-center">
                <AwsMark size="md" />
                <b className="text-sm font-semibold">{t('Nenhuma conta AWS conectada')}</b>
                <p className="max-w-xs text-xs text-muted-foreground">
                  {t('Conecte um login SSO e escolha as contas em Ajustes. Depois, volte aqui e selecione a conta.')}
                </p>
                <Button size="sm" className="mt-1" onClick={onAccounts}>
                  {t('Conectar conta AWS')}
                </Button>
              </div>
            ) : (
              <div className="space-y-3">
                <div className="grid gap-1.5">
                  <span className="text-[13px] font-medium">{t('Conta')}</span>
                  <Combobox
                    label={t('Conta')}
                    tall
                    value={connection}
                    searchPlaceholder={t('Buscar pelo título, número da conta ou papel')}
                    empty={t('Nenhuma conta com esse nome.')}
                    onChange={setConnection}
                    options={connections.map((one) => {
                      const live = logins.find((login) => login.id === one.login_id)?.connected ?? false

                      return {
                        value: one.id,
                        label: one.title || one.account_name || one.account_id,
                        detail: t('{account}, papel {role}', { account: one.account_id, role: one.role }),
                        leading: <AwsMark size="sm" />,
                        trailing: (
                          <span
                            className={cn(
                              'shrink-0 rounded-full px-2 py-px text-[11px] font-medium',
                              live ? 'bg-mint-soft text-mint' : 'bg-stone-soft text-muted-foreground',
                            )}
                          >
                            {live ? t('sessão ativa') : t('sem sessão')}
                          </span>
                        ),
                      }
                    })}
                  />
                </div>
                <div className="grid grid-cols-2 gap-3">
                  <label className="grid gap-1.5">
                    <span className="text-[13px] font-medium">{t('Região')}</span>
                    <Input value={region} onChange={(event) => setRegion(event.target.value.trim())} />
                  </label>
                  <label className="grid gap-1.5">
                    <span className="text-[13px] font-medium">{t('Bucket dos anexos')}</span>
                    <Input
                      value={shownBucket}
                      placeholder={t('keyhold-projeto')}
                      onChange={(event) => {
                        setBucketTouched(true)
                        setBucket(event.target.value.trim())
                      }}
                    />
                  </label>
                </div>
                <p className="text-xs text-faint">
                  {t('O Keyhold não cria o bucket. Sem ele, os registros funcionam e apenas os anexos ficam indisponíveis.')}
                </p>
                <label className="grid gap-1.5">
                  <span className="text-[13px] font-medium">{t('Caminho na AWS (opcional)')}</span>
                  <Input
                    value={root}
                    placeholder={t('/ (a conta inteira)')}
                    spellCheck={false}
                    onChange={(event) => setRoot(event.target.value.trim())}
                  />
                  <span className="text-xs text-faint">
                    {t(
                      'Deixe vazio para ver a conta inteira. Se o seu acesso é a uma pasta só, informe essa pasta, por exemplo /shared: o cofre começa nela.',
                    )}
                  </span>
                </label>
                <label className="flex items-start gap-2.5 rounded-[10px] border p-3 text-[13px]">
                  <Checkbox
                    className="mt-0.5"
                    checked={understood}
                    onCheckedChange={(on) => setUnderstood(on === true)}
                  />
                  <span className="grid gap-0.5">
                    <span className="font-medium">{t('Entendi que conta, região, caminho e bucket não mudam depois')}</span>
                    <span className="text-xs text-faint">
                      {t(
                        'Depois de criado, só o nome e o ícone do cofre podem ser alterados. Para mudar o resto, crie um cofre novo.',
                      )}
                    </span>
                  </span>
                </label>
              </div>
            ))}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onCancel}>
            {t('Cancelar')}
          </Button>
          <Button
            disabled={!ready}
            onClick={() =>
              onCreate(
                name.trim(),
                kind === 'aws' ? { connection_id: connection, region, bucket: shownBucket, root: root || null } : undefined,
                icon,
              )
            }
          >
            {t('Criar cofre')}
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
