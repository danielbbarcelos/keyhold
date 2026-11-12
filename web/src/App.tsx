import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { TooltipProvider } from '@/components/ui/tooltip'

import { Setup } from './screens/Setup'
import { Unlock } from './screens/Unlock'
import { Workspace } from './workspace/Workspace'
import { clearNow } from './clipboard'
import { hasToken } from './session'
import { applyAppearance } from './theme'
import { LogoLockup } from './workspace/Logo'
import { applyLayout } from './workspace/Splitter'
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

  const tree = status?.layout?.tree
  const label = status?.layout?.label
  useEffect(() => {
    applyLayout(tree && label ? { tree, label } : undefined)
  }, [tree, label])

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
      return <Offline onRetry={refresh} />
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

function Notice({ title, children, logo = true }: { title: string; children: React.ReactNode; logo?: boolean }) {
  return (
    <main className="grid min-h-screen place-items-center p-8">
      <Card className="w-full max-w-md gap-5 py-8">
        <CardHeader className="px-8">
          {logo && <LogoLockup className="mb-4 h-[44px] w-auto self-start" />}
          <CardTitle>{title}</CardTitle>
          {children && <CardDescription>{children}</CardDescription>}
        </CardHeader>
        <CardContent />
      </Card>
    </main>
  )
}

/**
 * The program stopped answering: closed, restarted, or the computer went to sleep and back.
 *
 * No logo here: it is served by the program that just went away, so it would be a broken image.
 * The button goes through `keyhold://`, which the installer registers to start Keyhold (or bring
 * the running one forward); the new window comes with a new session, and this tab is left behind.
 */
function Offline({ onRetry }: { onRetry: () => void }) {
  const [asked, setAsked] = useState(false)

  return (
    <Notice logo={false} title="O Keyhold não está respondendo">
      <span className="flex flex-col items-start gap-4">
        <span>
          Esta aba perdeu a conexão com o Keyhold, que foi fechado ou reiniciado. A chave do cofre
          foi removida da memória, e os dados continuam no arquivo. Abra o Keyhold novamente e
          destranque o cofre.
        </span>
        <span className="flex flex-wrap gap-2">
          <Button
            onClick={() => {
              setAsked(true)
              window.location.href = 'keyhold://open'
            }}
          >
            Abrir o Keyhold
          </Button>
          <Button variant="outline" onClick={onRetry}>
            Tentar novamente
          </Button>
        </span>
        {asked && (
          <span className="text-xs">
            O Keyhold é aberto em uma nova aba, e esta pode ser fechada. Caso nada aconteça, abra o
            Keyhold pelo menu de aplicativos.
          </span>
        )}
      </span>
    </Notice>
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
      setMessage('O Keyhold não está em execução. Abra pelo atalho ou pelo terminal.')
    }
  }

  return (
    <Notice title={state === 'opened' ? 'Keyhold aberto em uma nova aba' : 'Abrir o Keyhold'}>
      {state === 'opened' ? (
        'Esta aba pode ser fechada. A credencial da sessão é enviada apenas para a nova aba, nunca para um endereço salvo.'
      ) : (
        <span className="flex flex-col items-start gap-4">
          <span>
            Esta aba não possui a credencial da sessão, que muda sempre que o Keyhold é aberto. O
            Keyhold abre uma nova aba já com a credencial.
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
