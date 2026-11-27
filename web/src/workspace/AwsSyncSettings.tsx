import { useEffect, useState } from 'react'
import { RefreshCw, Trash2 } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'

import { api, type Status, type VaultSummary } from '../api'
import { t } from '../i18n'
import { toast, toastError } from './toast'
import { when } from './vault/paths'

const CHOICES = [1, 5, 15, 30, 0]

/**
 * Ajustes → Contas AWS: how often an open AWS vault reads its account again, and the listing each
 * one keeps in this computer so it opens at once.
 */
export function AwsSyncSettings({
  status,
  vaults,
  onChanged,
}: {
  status: Status
  vaults: VaultSummary[]
  onChanged: () => void
}) {
  const aws = vaults.filter((vault) => vault.kind === 'aws' && !vault.archived)
  const minutes = status.aws_sync_minutes ?? 5
  const [kept, setKept] = useState<Record<string, { records: number; at: number } | null>>({})

  const look = () =>
    void Promise.all(
      aws.map((vault) =>
        api
          .cachedListing(vault.id)
          .then((one) => [vault.id, one.cached ? { records: one.records.length, at: one.synced_at } : null] as const)
          .catch(() => [vault.id, null] as const),
      ),
    ).then((found) => setKept(Object.fromEntries(found)))

  useEffect(look, [vaults]) // eslint-disable-line react-hooks/exhaustive-deps

  if (aws.length === 0) return null

  return (
    <Card className="py-6">
      <CardHeader className="px-6">
        <CardTitle className="flex items-center gap-2">
          <RefreshCw className="size-4" />
          {t('Sincronização dos cofres AWS')}
        </CardTitle>
        <CardDescription>
          {t(
            'A lista de cada cofre AWS (pastas e o que aparece na árvore, nunca o valor de uma senha) fica guardada neste computador, cifrada com a sua chave, para o cofre abrir sem esperar pela AWS. Com a janela visível, o Keyhold lê a AWS de novo neste intervalo e sempre que a janela volta ao foco.',
          )}
        </CardDescription>
      </CardHeader>

      <CardContent className="px-6">
        <div className="divide-y overflow-hidden rounded-xl border">
          <div className="flex items-center justify-between gap-4 px-4 py-3.5 transition-colors hover:bg-hover">
            <span className="text-sm">{t('Sincronizar a cada')}</span>
            <Select
              value={String(minutes)}
              onValueChange={(value) =>
                void api
                  .setAwsSync(Number(value))
                  .then(onChanged)
                  .catch((cause) => toastError(cause, t('Não foi possível salvar o intervalo')))
              }
            >
              <SelectTrigger size="sm" className="w-[210px]" aria-label={t('Sincronizar a cada')}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {CHOICES.map((choice) => (
                  <SelectItem key={choice} value={String(choice)}>
                    {choice === 0 ? t('Só ao abrir') : choice === 1 ? t('1 minuto') : t('{n} minutos', { n: choice })}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          {aws.map((vault) => {
            const one = kept[vault.id]

            return (
              <div key={vault.id} className="flex items-center justify-between gap-4 px-4 py-3.5 transition-colors hover:bg-hover">
                <span className="min-w-0">
                  <span className="block truncate text-sm">{vault.name}</span>
                  <span className="block text-xs text-muted-foreground">
                    {one
                      ? t('lista salva de {n} registros, sincronizada {when}', {
                          n: one.records,
                          when: when(new Date(one.at * 1000).toISOString()),
                        })
                      : t('nenhuma lista salva')}
                  </span>
                </span>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={!one}
                  onClick={() =>
                    void api
                      .forgetListing(vault.id)
                      .then(() => {
                        toast(t('Lista salva apagada'), vault.name)
                        look()
                      })
                      .catch((cause) => toastError(cause))
                  }
                >
                  <Trash2 />
                  {t('Apagar a lista salva')}
                </Button>
              </div>
            )
          })}
        </div>
      </CardContent>
    </Card>
  )
}
