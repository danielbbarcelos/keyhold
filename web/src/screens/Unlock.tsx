import { PageFooter } from '@/i18n/Language'
import { useState } from 'react'
import { Loader2 } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'

import { PasswordInput } from '@/components/common/PasswordInput'

import { api, ApiError, type Status } from '../api'
import { t } from '../i18n'
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
      setError(cause instanceof ApiError ? cause.message : t('O Keyhold não respondeu. Tente novamente.'))
      setPassword('')
      setCode('')
    } finally {
      setBusy(false)
    }
  }

  return (
    <main className="flex min-h-screen flex-col items-center bg-background p-8">
      <div className="flex w-full flex-1 items-center justify-center">
      <form
        className="flex w-full max-w-[26rem] flex-col rounded-2xl border bg-card p-8 shadow-pop"
        onSubmit={(event) => void submit(event)}
      >
        <LogoLockup className="h-[52px] w-auto self-start" />

        <h1 className="mt-5 text-[22px] font-semibold tracking-tight">{t('Destrancar o Keyhold')}</h1>
        <p className="mt-1 mb-6 text-sm text-muted-foreground">
          {usingRecovery
            ? t('Digite a chave de recuperação anotada ao criar o cofre.')
            : needsPassword
              ? t('Digite a senha-mestra para abrir os cofres.')
              : t('Este cofre não usa senha-mestra. Ele abre com a chave armazenada nesta máquina.')}
        </p>

        <div className="w-full space-y-3 text-left">
          {needsPassword && !usingRecovery && (
            <PasswordInput
              id="password"
              aria-label={t('Senha-mestra')}
              placeholder={t('Senha-mestra')}
              className="h-10"
              value={password}
              autoFocus
              onChange={(event) => setPassword(event.target.value)}
            />
          )}

          {usingRecovery && (
            <Input
              id="recovery"
              aria-label={t('Chave de recuperação')}
              placeholder={t('Chave de recuperação')}
              className="h-10"
              value={recovery}
              autoFocus
              onChange={(event) => setRecovery(event.target.value)}
            />
          )}

          {needsCode && (
            <div className="space-y-2">
              <Label htmlFor="code">{t('Código do app autenticador')}</Label>
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
                ? status.attemptsLeft === 1
                  ? t('Resta 1 tentativa antes de o Keyhold parar de responder.')
                  : t('Restam {n} tentativas antes de o Keyhold parar de responder.', { n: status.attemptsLeft })
                : t('Feche e abra o Keyhold novamente para tentar outra vez.')}
            </p>
          )}

          <Button type="submit" size="lg" className="h-10 w-full" disabled={busy}>
            {busy && <Loader2 className="animate-spin" />}
            {t('Destrancar')}
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
            {usingRecovery ? t('Usar a senha-mestra') : t('Perdi a senha-mestra')}
          </button>
        )}
      </form>
      </div>
      <PageFooter version={status.version} />
    </main>
  )
}
