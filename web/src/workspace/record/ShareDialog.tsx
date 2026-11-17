import { useMemo, useState } from 'react'
import { Download, Eye, EyeOff, Link2, Loader2, Mail, Share2, ShieldOff, TriangleAlert } from 'lucide-react'

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
import { locale, t } from '@/i18n'
import { cn } from '@/lib/utils'

import { api, ApiError, type OpenRecord } from '../../api'
import { fromBase64url } from '../../base64'
import { fieldType } from '../fields'
import { toast } from '../toast'

/**
 * Sharing one record with somebody outside: a link from an AWS vault, a ZIP from a local one.
 *
 * As validated in the KH-COMPARTILHAR mock: Para, O que vai, Validade, Senha. The 2FA seed and the
 * attachments never go, and the dialog says which fields are left out. The opening password is
 * generated, shown once, and never stored.
 */
type Validity = { kind: 'hours'; hours: number } | { kind: 'until' } | { kind: 'first' }

const at = (seconds: number) => new Date(seconds * 1000).toLocaleTimeString(locale(), { hour: '2-digit', minute: '2-digit' })

export function ShareDialog({
  record,
  aws,
  sessionEndsAt,
  onClose,
  onShared,
}: {
  record: OpenRecord
  aws: boolean
  /** AWS: when the session that signs the link ends, the longest a link can last. */
  sessionEndsAt?: number | null
  onClose: () => void
  onShared: () => void
}) {
  const fields = record.payload.fields
  const shareable = fields.map((field, index) => ({ field, index })).filter(({ field }) => field.kind !== 'totp')
  const leftOut = fields.filter((field) => field.kind === 'totp').map((field) => field.label || fieldType(field.kind).label)

  const [recipient, setRecipient] = useState('')
  const [ticked, setTicked] = useState<number[]>(shareable.map(({ index }) => index))
  const [notes, setNotes] = useState(false)
  const [validity, setValidity] = useState<Validity>({ kind: 'hours', hours: 1 })
  // Follows the e-mail until the person touches it: ticked when the link goes by e-mail.
  const [password, setPassword] = useState<boolean | null>(null)
  const [format, setFormat] = useState<'txt' | 'md' | 'html'>('txt')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [done, setDone] = useState<null | { link?: string; password: string | null; expiresAt?: number; shareId?: string; name?: string }>(null)
  const [showing, setShowing] = useState(false)

  const withPassword = validity.kind === 'first' ? true : (password ?? recipient.trim() !== '')
  const hasNotes = record.payload.notes.trim() !== ''
  const nothing = ticked.length === 0 && !(notes && hasNotes)
  const sessionLeft = sessionEndsAt ? Math.max(0, sessionEndsAt - Date.now() / 1000) : null

  const choices = useMemo(() => {
    const options: Array<{ id: string; title: string; hint: string; validity: Validity }> = [
      { id: 'hours', title: t('1 hora'), hint: t('expira às {time}', { time: at(Date.now() / 1000 + 3600) }), validity: { kind: 'hours', hours: 1 } },
    ]
    if (sessionEndsAt && sessionLeft && sessionLeft > 3600) {
      options.push({ id: 'until', title: t('Até {time}', { time: at(sessionEndsAt) }), hint: t('o fim da sua sessão AWS'), validity: { kind: 'until' } })
    }
    options.push({ id: 'first', title: t('Primeiro acesso'), hint: t('apagado depois de aberto pela página'), validity: { kind: 'first' } })
    return options
  }, [sessionEndsAt, sessionLeft])

  async function share(byMail: boolean) {
    setBusy(true)
    setError('')
    const choice = { fields: ticked, notes: notes && hasNotes, recipient: recipient.trim() || undefined }

    try {
      if (aws) {
        const hours =
          validity.kind === 'hours' ? validity.hours : validity.kind === 'until' && sessionLeft ? Math.max(1, Math.floor(sessionLeft / 3600)) : 1
        const made = await api.shareRecord(record.id, {
          ...choice,
          hours,
          first_access: validity.kind === 'first',
          password: withPassword,
        })
        setDone({ link: made.link, password: made.password, expiresAt: made.share.expires_at, shareId: made.share.id })
        if (byMail) mail(made.link, made.share.expires_at, validity.kind === 'first')
        else await navigator.clipboard.writeText(made.link).then(() => toast(t('Link copiado')), () => undefined)
      } else {
        const made = await api.shareFile(record.id, { ...choice, format })
        const blob = new Blob([fromBase64url(made.data)], { type: 'application/zip' })
        const href = URL.createObjectURL(blob)
        const link = document.createElement('a')
        link.href = href
        link.download = made.name
        link.click()
        URL.revokeObjectURL(href)
        setDone({ password: made.password, name: made.name })
        if (byMail) mail(null, undefined, undefined, made.name)
      }
      onShared()
    } catch (cause) {
      setError(cause instanceof ApiError ? cause.message : t('Não foi possível compartilhar.'))
    } finally {
      setBusy(false)
    }
  }

  function mail(link: string | null, expiresAt?: number, first?: boolean, name?: string) {
    const title = record.payload.title
    const subject = t('{title}, compartilhado pelo Keyhold', { title })
    const body = link
      ? [
          t('Olá,'),
          '',
          t('Compartilhei com você o registro {title}. Abra no navegador:', { title }),
          '',
          link,
          '',
          first ? t('O conteúdo é apagado depois de aberto.') : t('O link vale até {time}.', { time: expiresAt ? at(expiresAt) : '' }),
          withPassword ? t('A senha de abertura vai por outro canal.') : '',
        ]
      : [
          t('Olá,'),
          '',
          t('Segue em anexo o arquivo {name} com o registro {title}.', { name: name ?? 'compartilhamento.zip', title }),
          t('Ele é protegido por senha (AES-256), que vai por outro canal.'),
          t('Para abrir no macOS ou em algumas versões do Windows, use o 7-Zip, o Keka ou o The Unarchiver.'),
        ]
    window.location.href = `mailto:${encodeURIComponent(recipient.trim())}?subject=${encodeURIComponent(subject)}&body=${encodeURIComponent(body.filter((line, i, all) => line !== '' || all[i - 1] !== '').join('\n'))}`
  }

  async function revoke() {
    if (!done?.shareId) return
    setBusy(true)
    try {
      await api.revokeShare(done.shareId)
      toast(t('Link revogado'), t('O conteúdo foi apagado do bucket.'))
      onShared()
      onClose()
    } catch (cause) {
      setError(cause instanceof ApiError ? cause.message : t('Não foi possível revogar.'))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="grid max-h-[calc(100vh-48px)] grid-rows-[auto_minmax(0,1fr)_auto] gap-0 overflow-hidden p-0 sm:max-w-[620px]">
        <DialogHeader className="border-b px-6 pt-5 pb-4">
          <DialogTitle className="flex items-center gap-2 text-lg">
            <Share2 className="size-[18px] text-muted-foreground" />
            {t('Compartilhar {title}', { title: record.payload.title })}
          </DialogTitle>
          <DialogDescription>
            {aws
              ? t('Para alguém de fora, sem Keyhold e sem acesso à AWS. Quem recebe abre o link no navegador e vê uma cópia dos campos que você escolher.')
              : t('Para alguém de fora. O Keyhold gera um arquivo ZIP protegido por senha com os campos que você escolher, para anexar ao e-mail.')}
          </DialogDescription>
        </DialogHeader>

        <div className="min-h-0 space-y-5 overflow-y-auto px-6 py-5">
          {done ? (
            <Done aws={aws} done={done} showing={showing} onShow={() => setShowing(!showing)} />
          ) : (
            <>
              <div className="grid gap-1.5">
                <span className="text-[13px] font-medium">{t('Para')}</span>
                <Input
                  type="email"
                  aria-label={t('E-mail de quem recebe')}
                  placeholder={t('nome@empresa.com')}
                  value={recipient}
                  onChange={(event) => setRecipient(event.target.value)}
                />
                <span className="text-xs text-faint">
                  {aws
                    ? t('Fica registrado na trilha do cofre, junto com os campos e a validade, mesmo se o link for por outro canal.')
                    : t('Fica registrado na Trilha deste computador, junto com os campos, mesmo se o arquivo for por outro canal.')}
                </span>
              </div>

              <div className="grid gap-1.5">
                <span className="text-[13px] font-medium">{t('O que vai')}</span>
                <div className="overflow-hidden rounded-xl border">
                  {shareable.map(({ field, index }) => (
                    <label key={index} className="flex cursor-pointer items-center gap-3 border-b px-3 py-2.5 text-sm last:border-b-0">
                      <Checkbox
                        checked={ticked.includes(index)}
                        onCheckedChange={(on) =>
                          setTicked((current) => (on === true ? [...current, index] : current.filter((one) => one !== index)))
                        }
                      />
                      <span className="flex-1">{field.label || fieldType(field.kind).label}</span>
                      <span className="max-w-[45%] truncate text-xs text-faint">
                        {fieldType(field.kind).secret ? '••••••••••••' : field.value}
                      </span>
                    </label>
                  ))}
                  {hasNotes && (
                    <label className="flex cursor-pointer items-center gap-3 px-3 py-2.5 text-sm">
                      <Checkbox checked={notes} onCheckedChange={(on) => setNotes(on === true)} />
                      <span className="flex-1">{t('Observação')}</span>
                      <span className="max-w-[45%] truncate text-xs text-faint">{record.payload.notes}</span>
                    </label>
                  )}
                </div>
                <span className="text-xs text-faint">
                  {t('O título do registro e o seu nome vão sempre. É uma cópia de agora: editar o registro depois não muda o que foi compartilhado.')}
                </span>
                <p className="flex items-start gap-2 rounded-xl bg-raised p-3 text-[13px] text-muted-foreground">
                  <ShieldOff className="mt-0.5 size-4 shrink-0" />
                  <span>
                    {leftOut.length > 0 && (
                      <b className="font-medium text-foreground">{t('Fica de fora: {list}.', { list: leftOut.join(', ') })} </b>
                    )}
                    {t('A semente do 2FA e os arquivos anexados nunca vão num compartilhamento.')}
                  </span>
                </p>
              </div>

              {aws ? (
                <>
                  <div className="grid gap-1.5">
                    <span className="text-[13px] font-medium">{t('Validade')}</span>
                    <div className="grid grid-cols-3 gap-2">
                      {choices.map((option) => (
                        <button
                          key={option.id}
                          type="button"
                          aria-pressed={validity.kind === option.validity.kind}
                          onClick={() => setValidity(option.validity)}
                          className={cn(
                            'rounded-xl border px-3 py-2.5 text-left hover:bg-hover',
                            validity.kind === option.validity.kind && 'border-primary bg-primary-soft',
                          )}
                        >
                          <b className="block text-sm font-semibold">{option.title}</b>
                          <span className="block text-xs text-muted-foreground">{option.hint}</span>
                        </button>
                      ))}
                    </div>
                    {sessionEndsAt && (
                      <span className="text-xs text-faint">
                        {t('O link é assinado com a sua sessão e não dura mais que ela: sua sessão AWS termina às {time}.', { time: at(sessionEndsAt) })}
                      </span>
                    )}
                  </div>

                  <label className="flex cursor-pointer items-start gap-3 rounded-xl border px-4 py-3">
                    <Checkbox
                      checked={withPassword}
                      disabled={validity.kind === 'first'}
                      onCheckedChange={(on) => setPassword(on === true)}
                      className="mt-0.5"
                    />
                    <span>
                      <b className="block text-sm font-semibold">{t('Proteger com senha de abertura')}</b>
                      <span className="block text-xs text-muted-foreground">
                        {t('O Keyhold gera uma senha ao criar o link. Mande por outro canal, como WhatsApp ou uma ligação, nunca no mesmo e-mail. Obrigatória no primeiro acesso.')}
                      </span>
                    </span>
                  </label>
                </>
              ) : (
                <>
                  <div className="grid gap-1.5">
                    <span className="text-[13px] font-medium">{t('Formato do arquivo dentro do ZIP')}</span>
                    <div className="flex rounded-lg border bg-raised p-0.5">
                      {(['txt', 'md', 'html'] as const).map((one) => (
                        <button
                          key={one}
                          type="button"
                          aria-pressed={format === one}
                          onClick={() => setFormat(one)}
                          className={cn(
                            'h-7 flex-1 rounded-md px-2.5 text-[13px] font-medium text-muted-foreground hover:text-foreground',
                            format === one && 'bg-card text-foreground shadow-sm',
                          )}
                        >
                          .{one}
                        </button>
                      ))}
                    </div>
                  </div>
                  <p className="flex items-start gap-2 rounded-xl bg-amber-soft p-3 text-[13px] text-foreground">
                    <TriangleAlert className="mt-0.5 size-4 shrink-0 text-amber" />
                    <span>
                      {t('O ZIP é protegido com AES-256 e uma senha de 24 caracteres gerada pelo Keyhold. O utilitário do macOS e algumas versões do Windows não abrem ZIP com AES: quem recebe vai precisar do 7-Zip, do Keka ou do The Unarchiver.')}
                    </span>
                  </p>
                </>
              )}
            </>
          )}

          {error && <p className="text-sm text-rose">{error}</p>}
        </div>

        <DialogFooter className="m-0 border-t px-6 py-4">
          {done ? (
            <>
              {aws && done.shareId && (
                <Button variant="outline" disabled={busy} onClick={() => void revoke()}>
                  {t('Revogar link')}
                </Button>
              )}
              <Button onClick={onClose}>{t('Concluir')}</Button>
            </>
          ) : (
            <>
              <Button variant="outline" onClick={onClose}>
                {t('Cancelar')}
              </Button>
              <span className="flex-1" />
              <Button variant="outline" disabled={busy || nothing} onClick={() => void share(false)}>
                {busy ? <Loader2 className="animate-spin" /> : aws ? <Link2 /> : <Download />}
                {aws ? t('Copiar link') : t('Baixar ZIP')}
              </Button>
              <Button disabled={busy || nothing || recipient.trim() === ''} onClick={() => void share(true)}>
                <Mail />
                {t('Enviar por e-mail')}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function Done({
  aws,
  done,
  showing,
  onShow,
}: {
  aws: boolean
  done: { link?: string; password: string | null; expiresAt?: number; name?: string }
  showing: boolean
  onShow: () => void
}) {
  return (
    <div className="space-y-4">
      {aws && done.link && (
        <div className="grid gap-1.5">
          <span className="text-[13px] font-medium">{t('Link')}</span>
          <div className="flex items-center gap-2">
            <Input readOnly value={done.link} className="font-mono text-xs" onFocus={(event) => event.target.select()} />
            <Button variant="outline" onClick={() => void navigator.clipboard.writeText(done.link!).then(() => toast(t('Link copiado')))}>
              {t('Copiar')}
            </Button>
          </div>
          {done.expiresAt && <span className="text-xs text-faint">{t('Vale até {time}.', { time: at(done.expiresAt) })}</span>}
        </div>
      )}

      {done.password && (
        <div className="grid gap-1.5">
          <span className="text-[13px] font-medium">{aws ? t('Senha de abertura') : t('Senha do ZIP')}</span>
          <div className="flex items-center gap-2">
            <Input readOnly type={showing ? 'text' : 'password'} value={done.password} className="font-mono" />
            <Button variant="ghost" size="icon" aria-label={showing ? t('Ocultar') : t('Mostrar')} onClick={onShow}>
              {showing ? <EyeOff /> : <Eye />}
            </Button>
            <Button variant="outline" onClick={() => void navigator.clipboard.writeText(done.password!).then(() => toast(t('Senha copiada')))}>
              {t('Copiar')}
            </Button>
          </div>
          <span className="text-xs text-faint">
            {aws
              ? t('Mande por outro canal, nunca junto com o link. O Keyhold não guarda esta senha: ela não aparece de novo depois de fechar.')
              : t('Mande por outro canal, nunca junto com o arquivo. O Keyhold não guarda esta senha: ela não aparece de novo depois de fechar.')}
          </span>
        </div>
      )}

      {!aws && (
        <p className="text-[13px] text-muted-foreground">
          {t('O arquivo {name} foi baixado. Anexe ao e-mail: o cliente de e-mail não anexa sozinho.', { name: done.name ?? 'compartilhamento.zip' })}
        </p>
      )}
    </div>
  )
}
