import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { TooltipProvider } from '@/components/ui/tooltip'

import { Setup } from './screens/Setup'
import { Unlock } from './screens/Unlock'
import { Workspace } from './workspace/Workspace'
import { hasToken } from './session'
import { useStatus } from './useStatus'

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

  return <TooltipProvider delayDuration={300}>{screen()}</TooltipProvider>

  function screen() {
    if (!hasToken()) {
      return (
        <Notice title="Esta janela não tem a credencial da sessão">
          Abra o Keyhold pelo terminal ou pelo atalho. A credencial vive na URL, vale só enquanto o
          programa estiver aberto, e deixa de valer se outra instância assumir o cofre.
        </Notice>
      )
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
          <CardTitle>{title}</CardTitle>
          {children && <CardDescription>{children}</CardDescription>}
        </CardHeader>
        <CardContent />
      </Card>
    </main>
  )
}
