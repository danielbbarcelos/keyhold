import { useState } from 'react'
import { Loader2 } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'

import { PasswordInput } from '@/components/common/PasswordInput'

import { api, ApiError, type Status } from '../api'
import { LogoLockup } from '../workspace/Logo'

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
    <main className="grid min-h-screen place-items-center bg-background p-8">
      <form
        className="flex w-full max-w-[26rem] flex-col rounded-2xl border bg-card p-8 shadow-pop"
        onSubmit={(event) => void submit(event)}
      >
        <LogoLockup className="h-[52px] w-auto self-start" />

        <h1 className="mt-5 text-[22px] font-semibold tracking-tight">Destrancar o Keyhold</h1>
        <p className="mt-1 mb-6 text-sm text-muted-foreground">
          {usingRecovery
            ? 'Digite a chave de recuperação que você guardou ao criar o cofre.'
            : needsPassword
              ? 'Digite a senha-mestra para abrir os seus cofres.'
              : 'Este cofre não usa senha-mestra: ele abre com a chave guardada nesta máquina.'}
        </p>

        <div className="w-full space-y-3 text-left">
          {needsPassword && !usingRecovery && (
            <PasswordInput
              id="password"
              aria-label="Senha-mestra"
              placeholder="Senha-mestra"
              className="h-10"
              value={password}
              autoFocus
              onChange={(event) => setPassword(event.target.value)}
            />
          )}

          {usingRecovery && (
            <Input
              id="recovery"
              aria-label="Chave de recuperação"
              placeholder="Chave de recuperação"
              className="h-10"
              value={recovery}
              autoFocus
              onChange={(event) => setRecovery(event.target.value)}
            />
          )}

          {needsCode && (
            <div className="space-y-2">
              <Label htmlFor="code">Código do seu app autenticador</Label>
              <Input
                id="code"
                value={code}
                inputMode="numeric"
                maxLength={6}
                className="h-10 font-mono tracking-[0.3em]"
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

          <Button type="submit" size="lg" className="h-10 w-full" disabled={busy}>
            {busy && <Loader2 className="animate-spin" />}
            Destrancar
          </Button>
        </div>

        {needsPassword && (
          <button
            type="button"
            className="mt-5 self-center text-[13px] text-link hover:underline"
            onClick={() => {
              setUsingRecovery(!usingRecovery)
              setError('')
            }}
          >
            {usingRecovery ? 'Usar a senha-mestra' : 'Perdi a senha-mestra'}
          </button>
        )}
      </form>
    </main>
  )
}
