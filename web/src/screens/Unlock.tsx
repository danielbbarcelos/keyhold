import { useState } from 'react'
import { Loader2, Lock } from 'lucide-react'

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

import { Logo } from '@/components/common/Logo'
import { PasswordInput } from '@/components/common/PasswordInput'

import { api, ApiError, type Status } from '../api'

/**
 * The lock screen.
 *
 * It says which door it is asking about, because a vault with a second factor that simply reports
 * "wrong" when the code is missing teaches people that the app is broken. And it offers the
 * recovery key as a visible way in rather than a link somebody has to know exists — the day it is
 * needed is a bad day already.
 */
export function Unlock({ status, onUnlocked }: { status: Status; onUnlocked: () => void }) {
  const [password, setPassword] = useState('')
  const [code, setCode] = useState('')
  const [recovery, setRecovery] = useState('')
  const [usingRecovery, setUsingRecovery] = useState(false)
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)

  const needsPassword = status.identity.protection === 'password'
  const needsCode = status.identity.second_factor

  async function submit(event: React.FormEvent) {
    event.preventDefault()
    setBusy(true)

    try {
      await api.unlock(
        usingRecovery
          ? { recovery_key: recovery, ...(needsCode ? { code } : {}) }
          : { ...(needsPassword ? { password } : {}), ...(needsCode ? { code } : {}) },
      )
      onUnlocked()
    } catch (cause) {
      setError(cause instanceof ApiError ? cause.message : 'Não foi possível falar com o Keyhold.')
      setPassword('')
      setCode('')
    } finally {
      setBusy(false)
    }
  }

  return (
    <main className="grid min-h-screen place-items-center p-8">
      <div className="flex w-full max-w-sm flex-col items-center gap-6">
        <div className="flex flex-col items-center gap-2">
          <Logo className="size-14" />
          <span className="text-sm font-medium text-muted-foreground">Keyhold</span>
        </div>

        <Card className="w-full gap-5 py-8">
          {/*
            * The form carries the layout, not the card.
            *
            * `Card` spaces its own children, and wrapping the header, body and footer in a form makes
            * all three a single child — so the card's gap applied to nothing and the label sat flush
            * against the title. The spacing belongs wherever the children actually are.
            */}
          <form className="flex flex-col gap-5" onSubmit={(event) => void submit(event)}>
            <CardHeader className="px-8">
              <CardTitle className="flex items-center gap-2 text-base">
                <Lock className="size-4" />
                Cofre trancado
              </CardTitle>
              {!needsPassword && !usingRecovery && (
                <CardDescription>
                  Este cofre não usa senha-mestra: ele abre com a chave guardada nesta máquina.
                </CardDescription>
              )}
            </CardHeader>

            <CardContent className="space-y-5 px-8">
              {needsPassword && !usingRecovery && (
                <div className="space-y-2">
                  <Label htmlFor="password">Senha-mestra</Label>
                  <PasswordInput
                    id="password"
                    value={password}
                    autoFocus
                    onChange={(event) => setPassword(event.target.value)}
                  />
                </div>
              )}

              {usingRecovery && (
                <div className="space-y-2">
                  <Label htmlFor="recovery">Chave de recuperação</Label>
                  <Input
                    id="recovery"
                    value={recovery}
                    autoFocus
                    onChange={(event) => setRecovery(event.target.value)}
                  />
                </div>
              )}

              {needsCode && (
                <div className="space-y-2">
                  <Label htmlFor="code">Código do seu app autenticador</Label>
                  <Input
                    id="code"
                    value={code}
                    inputMode="numeric"
                    maxLength={6}
                    className="tracking-[0.3em]"
                    onChange={(event) => setCode(event.target.value)}
                  />
                </div>
              )}

              {error && <p className="text-sm text-destructive">{error}</p>}

              {status.attemptsLeft <= 2 && (
                <p className="text-sm text-muted-foreground">
                  {status.attemptsLeft > 0
                    ? `${status.attemptsLeft} tentativas antes de o Keyhold parar de responder.`
                    : 'Feche o Keyhold e abra de novo para tentar outra vez.'}
                </p>
              )}
            </CardContent>

            <CardFooter className="gap-2 px-8">
              <Button type="submit" disabled={busy}>
                {busy && <Loader2 className="animate-spin" />}
                Destrancar
              </Button>

              {needsPassword && (
                <Button
                  type="button"
                  variant="ghost"
                  onClick={() => {
                    setUsingRecovery(!usingRecovery)
                    setError('')
                  }}
                >
                  {usingRecovery ? 'Usar a senha-mestra' : 'Perdi a senha-mestra'}
                </Button>
              )}
            </CardFooter>
          </form>
        </Card>
      </div>
    </main>
  )
}
