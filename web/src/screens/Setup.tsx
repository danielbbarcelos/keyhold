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
import { Separator } from '@/components/ui/separator'

import { PasswordInput } from '@/components/common/PasswordInput'

import { api, ApiError, type Protection } from '../api'

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

const STEPS: Array<{ id: Step; label: string }> = [
  { id: 'choose', label: 'Proteção' },
  { id: 'password', label: 'Senha-mestra' },
  { id: 'recovery', label: 'Chave de recuperação' },
]

export function Setup({ onDone }: { onDone: () => void }) {
  const [step, setStep] = useState<Step>('choose')
  const [protection, setProtection] = useState<Protection>('password')
  const [password, setPassword] = useState('')
  const [entropy, setEntropy] = useState<number | null>(null)
  const [recoveryKey, setRecoveryKey] = useState('')
  const [typedBack, setTypedBack] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)

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
    <main className="grid min-h-screen place-items-center p-8">
      <Card className="w-full max-w-xl gap-5 py-8">
        <CardHeader className="px-8">
          <nav className="mb-3 flex items-center gap-2 text-xs text-muted-foreground">
            {STEPS.map((entry, index) => (
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
              <CardTitle>Como este cofre vai ser protegido?</CardTitle>
              <CardDescription>
                As duas opções cifram o que você guarda. A diferença é onde mora a chave que abre.
              </CardDescription>
            </>
          )}

          {step === 'password' && (
            <>
              <CardTitle>Sua senha-mestra</CardTitle>
              <CardDescription>
                Ela não é guardada em lugar nenhum e não pode ser redefinida. Use uma senha longa que
                você não use em mais nada — ou deixe o Keyhold gerar uma.
              </CardDescription>
            </>
          )}

          {step === 'recovery' && (
            <>
              <CardTitle>Guarde a chave de recuperação</CardTitle>
              <CardDescription>
                É a única forma de abrir este cofre se a senha-mestra se perder. Ela aparece uma vez
                só, não fica guardada em lugar nenhum e não pode ser gerada de novo.
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
                title="Com uma senha-mestra"
                badge={<Badge>recomendado</Badge>}
              >
                A chave que abre o cofre nasce da sua senha e não existe em nenhum outro lugar. Quem
                copiar o arquivo do cofre não leva nada aproveitável.
              </Option>

              <Option
                selected={protection === 'device'}
                onSelect={() => setProtection('device')}
                icon={<ShieldCheck className="size-4" />}
                title="Sem senha, protegido por esta máquina"
              >
                Abre sozinho, sem digitar nada. Em troca, a chave fica num arquivo ao lado do cofre:
                quem conseguir ler seus arquivos — um backup, uma pasta sincronizada, outro programa
                seu — abre o cofre também.
              </Option>
            </div>
          )}

          {step === 'password' && (
            <div className="space-y-2">
              <Label htmlFor="master">Senha-mestra</Label>

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
                  Gerada aqui: <span className="text-foreground">{entropy} bits</span> de entropia.
                  Copie-a para onde você guarda senhas antes de continuar — ela não aparece de novo.
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
                Perder as duas — senha e chave — significa perder o conteúdo, inclusive para nós,
                porque não existe nós.
              </p>

              <Separator />

              <div className="space-y-2">
                <Label htmlFor="confirm">Digite a chave de volta para confirmar</Label>
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
              Continuar
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
                Gerar uma senha forte
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
                Criar o cofre
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
              Confirmar e abrir o cofre
            </Button>
          )}
        </CardFooter>
      </Card>
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
