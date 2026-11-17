import { PageFooter } from '@/i18n/Language'
import { useState } from 'react'
import { KeyRound, Loader2, ShieldCheck, Wand2 } from 'lucide-react'

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

import { LogoLockup } from '../workspace/Logo'
import { Separator } from '@/components/ui/separator'

import { PasswordInput } from '@/components/common/PasswordInput'

import { api, ApiError, type Protection } from '../api'
import { t } from '../i18n'

/**
 * Setting up a vault: three steps, in the order that makes the consequences land before the
 * decisions do.
 *
 * The middle step is the one that matters. keyhold recommends a master password and offers to
 * generate one, and it also lets somebody choose their own or go without — and it says, in the same
 * breath, exactly what each option costs. A vault that refuses to open on its owner's terms is a
 * vault that ends up as a text file on the desktop, which protects nothing at all.
 *
 * The last step is not a formality. The recovery key exists in exactly one response, is never
 * stored, and cannot be derived from anything: if it is not written down now, it is gone. So the
 * step asks for it back before it will move on.
 */

type Step = 'choose' | 'password' | 'recovery'

const steps = (): Array<{ id: Step; label: string }> => [
  { id: 'choose', label: t('Proteção') },
  { id: 'password', label: t('Senha-mestra') },
  { id: 'recovery', label: t('Chave de recuperação') },
]

export function Setup({ version, onDone }: { version?: string; onDone: () => void }) {
  const [step, setStep] = useState<Step>('choose')
  const [protection, setProtection] = useState<Protection>('password')
  const [password, setPassword] = useState('')
  const [entropy, setEntropy] = useState<number | null>(null)
  const [recoveryKey, setRecoveryKey] = useState('')
  const [typedBack, setTypedBack] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)

  // The strength stays highlighted in either language: the sentence is split where it goes.
  const generated = t('Senha gerada com {bits} de entropia.').split('{bits}')

  async function attempt(action: () => Promise<void>) {
    setBusy(true)

    try {
      await action()
      setError('')
    } catch (cause) {
      setError(cause instanceof ApiError ? cause.message : t('O Keyhold não respondeu. Tente novamente.'))
    } finally {
      setBusy(false)
    }
  }

  return (
    <main className="flex min-h-screen flex-col items-center p-8">
      <div className="flex w-full flex-1 items-center justify-center">
      <Card className="w-full max-w-xl gap-5 py-8">
        <CardHeader className="px-8">
          <LogoLockup className="mb-4 h-[44px] w-auto self-start" />
          <nav className="mb-3 flex items-center gap-2 text-xs text-muted-foreground">
            {steps().map((entry, index) => (
              <span key={entry.id} className="flex items-center gap-2">
                {index > 0 && <span aria-hidden="true">·</span>}
                <span className={entry.id === step ? 'font-medium text-foreground' : undefined}>
                  {entry.label}
                </span>
              </span>
            ))}
          </nav>

          {step === 'choose' && (
            <>
              <CardTitle>{t('Escolha a proteção do cofre')}</CardTitle>
              <CardDescription>
                {t('As duas opções cifram o conteúdo do cofre. A diferença é onde fica a chave que o abre.')}
              </CardDescription>
            </>
          )}

          {step === 'password' && (
            <>
              <CardTitle>{t('Senha-mestra')}</CardTitle>
              <CardDescription>
                {t(
                  'A senha-mestra não é armazenada e não pode ser redefinida. Use uma senha longa e exclusiva, ou gere uma com o Keyhold.',
                )}
              </CardDescription>
            </>
          )}

          {step === 'recovery' && (
            <>
              <CardTitle>{t('Anote a chave de recuperação')}</CardTitle>
              <CardDescription>
                {t(
                  'É a única forma de abrir este cofre caso a senha-mestra seja perdida. Ela é exibida apenas uma vez, não é armazenada e não pode ser gerada novamente.',
                )}
              </CardDescription>
            </>
          )}
        </CardHeader>

        <CardContent className="space-y-4 px-8">
          {step === 'choose' && (
            <div className="space-y-2">
              <Option
                selected={protection === 'password'}
                onSelect={() => setProtection('password')}
                icon={<KeyRound className="size-4" />}
                title={t('Com uma senha-mestra')}
                badge={<Badge>{t('recomendado')}</Badge>}
              >
                {t(
                  'A chave que abre o cofre é derivada da sua senha e não existe em nenhum outro lugar. Uma cópia do arquivo do cofre não revela nada sem a senha.',
                )}
              </Option>

              <Option
                selected={protection === 'device'}
                onSelect={() => setProtection('device')}
                icon={<ShieldCheck className="size-4" />}
                title={t('Sem senha, protegido por esta máquina')}
              >
                {t(
                  'O cofre abre sem senha. A chave fica em um arquivo ao lado do cofre: quem conseguir ler seus arquivos (um backup, uma pasta sincronizada, outro programa) também abre o cofre.',
                )}
              </Option>
            </div>
          )}

          {step === 'password' && (
            <div className="space-y-2">
              <Label htmlFor="master">{t('Senha-mestra')}</Label>

              <PasswordInput
                id="master"
                // Remounts when a password is generated, so the field comes back already visible:
                // a generated password exists to be copied somewhere safe, and one nobody can see
                // is a vault nobody can reopen.
                key={entropy === null ? 'typed' : `generated-${entropy}`}
                initiallyVisible={entropy !== null}
                value={password}
                autoFocus
                onChange={(event) => {
                  setPassword(event.target.value)
                  setEntropy(null)
                }}
              />

              {entropy !== null && (
                <p className="text-sm text-muted-foreground">
                  {generated[0]}
                  <span className="text-foreground">{entropy} bits</span>
                  {generated[1]}{' '}
                  {t('Copie a senha para um lugar seguro antes de continuar. Ela não é exibida novamente.')}
                </p>
              )}
            </div>
          )}

          {step === 'recovery' && (
            <div className="space-y-4">
              <p className="rounded-lg border border-dashed bg-muted/40 p-4 text-sm leading-7 tracking-wider break-all">
                {recoveryKey}
              </p>

              <p className="text-sm text-muted-foreground">
                {t(
                  'Sem a senha-mestra e sem a chave de recuperação, o conteúdo do cofre não pode ser recuperado por ninguém.',
                )}
              </p>

              <Separator />

              <div className="space-y-2">
                <Label htmlFor="confirm">{t('Digite a chave novamente para confirmar')}</Label>
                <Input
                  id="confirm"
                  value={typedBack}
                  autoFocus
                  onChange={(event) => setTypedBack(event.target.value)}
                />
              </div>
            </div>
          )}

          {error && <p className="text-sm text-destructive">{error}</p>}
        </CardContent>

        <CardFooter className="gap-2 px-8">
          {step === 'choose' && (
            <Button
              disabled={busy}
              onClick={() =>
                protection === 'password'
                  ? setStep('password')
                  : void attempt(async () => {
                      const created = await api.setup(null)
                      setRecoveryKey(created.recoveryKey)
                      setStep('recovery')
                    })
              }
            >
              {t('Continuar')}
            </Button>
          )}

          {step === 'password' && (
            <>
              <Button
                variant="outline"
                disabled={busy}
                onClick={() =>
                  void attempt(async () => {
                    const made = await api.generatePassword()
                    setPassword(made.password)
                    setEntropy(made.entropyBits)
                  })
                }
              >
                <Wand2 />
                {t('Gerar uma senha forte')}
              </Button>

              <Button
                disabled={busy || password.trim().length === 0}
                onClick={() =>
                  void attempt(async () => {
                    const created = await api.setup(password)
                    setRecoveryKey(created.recoveryKey)
                    setStep('recovery')
                  })
                }
              >
                {busy && <Loader2 className="animate-spin" />}
                {t('Criar cofre')}
              </Button>
            </>
          )}

          {step === 'recovery' && (
            <Button
              disabled={busy || typedBack.trim().length === 0}
              onClick={() =>
                void attempt(async () => {
                  await api.confirmRecovery(typedBack)
                  onDone()
                })
              }
            >
              {busy && <Loader2 className="animate-spin" />}
              {t('Confirmar e abrir o cofre')}
            </Button>
          )}
        </CardFooter>
      </Card>
      </div>
      <PageFooter version={version} />
    </main>
  )
}

function Option({
  selected,
  onSelect,
  icon,
  title,
  badge,
  children,
}: {
  selected: boolean
  onSelect: () => void
  icon: React.ReactNode
  title: string
  badge?: React.ReactNode
  children: React.ReactNode
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-pressed={selected}
      className={`w-full rounded-lg border p-4 text-left transition-colors ${
        selected ? 'border-primary bg-muted/50' : 'border-border hover:bg-muted/30'
      }`}
    >
      <span className="flex items-center gap-2 font-medium">
        {icon}
        {title}
        {badge}
      </span>
      <span className="mt-1 block text-sm text-muted-foreground">{children}</span>
    </button>
  )
}
