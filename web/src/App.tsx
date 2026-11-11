import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { TooltipProvider } from '@/components/ui/tooltip'

import { Setup } from './screens/Setup'
import { Unlock } from './screens/Unlock'
import { Workspace } from './workspace/Workspace'
import { clearNow } from './clipboard'
import { hasToken } from './session'
import { applyAppearance } from './theme'
import { LogoLockup } from './workspace/Logo'
import { useStatus } from './useStatus'
import { useEffect, useState } from 'react'
import { Loader2 } from 'lucide-react'

import { Button } from '@/components/ui/button'

/**
 * Which screen keyhold is on, decided by one question asked of the binary.
 *
 * The order of these branches is the order of certainty: no token, then no contact, then no answer
 * yet, then the three real states. Everything above the real states renders as "not available"
 * rather than as an empty vault — failing closed is the only sane direction for a screen that
 * stands in front of a lock.
 */
export function App() {
  const { status, offline, refresh } = useStatus()

  const theme = status?.appearance.theme
  const accent = status?.appearance.accent
  const width = status?.appearance.width ?? 'inteira'

  // Locking takes the clipboard back. Here rather than in the workspace: the workspace is already
  // gone by the time the status says the vault is locked.
  const unlocked = status?.lock.unlocked
  useEffect(() => {
    if (unlocked === false) clearNow()
  }, [unlocked])

  useEffect(() => {
    if (theme && accent) {
      applyAppearance({ theme, accent, width })
    }
  }, [theme, accent, width])

  return <TooltipProvider delayDuration={300}>{screen()}</TooltipProvider>

  function screen() {
    if (!hasToken()) {
      return <OpenFromBookmark />
    }

    if (offline) {
      return (
        <Notice title="Sem contato com o Keyhold">
          O programa ainda está rodando? Se você o fechou, a chave já foi esquecida — é o que fechar
          significa aqui.
        </Notice>
      )
    }

    if (!status) {
      return <Notice title="Conferindo o cofre…">{null}</Notice>
    }

    if (!status.identity.initialised) {
      return <Setup onDone={refresh} />
    }

    if (!status.lock.unlocked) {
      return <Unlock status={status} onUnlocked={refresh} />
    }

    return <Workspace status={status} onChanged={refresh} />
  }
}

function Notice({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <main className="grid min-h-screen place-items-center p-8">
      <Card className="w-full max-w-md gap-5 py-8">
        <CardHeader className="px-8">
          <LogoLockup className="mb-4 h-[44px] w-auto self-start" />
          <CardTitle>{title}</CardTitle>
          {children && <CardDescription>{children}</CardDescription>}
        </CardHeader>
        <CardContent />
      </Card>
    </main>
  )
}

/**
 * A tab without the session token: a bookmark, or the address typed by hand.
 *
 * The token is never handed to this page. The button asks the running Keyhold to open its own
 * address, token included, in the browser — the same thing "Abrir e continuar" does from a second
 * start — and this tab is left behind, to be closed.
 */
function OpenFromBookmark() {
  const [state, setState] = useState<'idle' | 'busy' | 'opened' | 'failed'>('idle')
  const [message, setMessage] = useState('')

  async function open() {
    setState('busy')

    try {
      const response = await fetch('/open-window', { method: 'POST', credentials: 'omit' })
      const answer = await response.json().catch(() => ({}))

      if (response.ok) {
        setState('opened')
      } else {
        setState('failed')
        setMessage(answer.error ?? '')
      }
    } catch {
      setState('failed')
      setMessage('O Keyhold não está rodando. Abra pelo atalho ou pelo terminal.')
    }
  }

  return (
    <Notice title={state === 'opened' ? 'Abri o Keyhold numa aba nova' : 'Abrir o Keyhold'}>
      {state === 'opened' ? (
        'Pode fechar esta aba. A credencial da sessão vai só para a aba nova, nunca para um endereço salvo.'
      ) : (
        <span className="flex flex-col items-start gap-4">
          <span>
            Esta aba não tem a credencial da sessão, que muda toda vez que o Keyhold abre. O botão
            pede ao Keyhold para abrir a janela dele com ela.
          </span>
          <Button onClick={() => void open()} disabled={state === 'busy'}>
            {state === 'busy' && <Loader2 className="animate-spin" />}
            Abrir o Keyhold
          </Button>
          {state === 'failed' && message && <span className="text-destructive">{message}</span>}
        </span>
      )}
    </Notice>
  )
}
