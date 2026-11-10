import { useRef, useState } from 'react'
import { Download, Loader2, Upload } from 'lucide-react'

import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'

import { api, ApiError, type VaultSummary } from '../api'
import { PasswordInput } from '@/components/common/PasswordInput'

/**
 * Taking a copy out, and bringing one in.
 *
 * The export asks for a passphrase of its own — not the master password. A file that outlives the
 * session should not be opened by the credential that opens everything else, and being asked a
 * second time is being asked, once, to think about where the file is going.
 *
 * The import creates; it never merges. Matching an incoming record against one already here would
 * mean the vault deciding which of two passwords is the current one, and it has no way to know.
 */
interface TransferPanelProps {
  vaults: VaultSummary[]
  onChanged: () => void
}

export function TransferPanel({ vaults, onChanged }: TransferPanelProps) {
  const live = vaults.filter((vault) => !vault.archived)

  const [vaultId, setVaultId] = useState(live[0]?.id ?? '')
  const [passphrase, setPassphrase] = useState('')
  const [busy, setBusy] = useState(false)
  const [note, setNote] = useState('')
  const [error, setError] = useState('')
  const filePicker = useRef<HTMLInputElement>(null)

  const chosen = live.find((vault) => vault.id === vaultId) ?? live[0]

  async function attempt(action: () => Promise<string>) {
    setBusy(true)

    try {
      setNote(await action())
      setError('')
    } catch (cause) {
      setError(cause instanceof ApiError ? cause.message : 'Não foi possível falar com o Keyhold.')
      setNote('')
    } finally {
      setBusy(false)
    }
  }

  /** Hand the archive to the browser, which asks where to put it. keyhold never picks the place. */
  function save(contents: string, filename: string) {
    const blob = new Blob([contents], { type: 'application/json' })
    const href = URL.createObjectURL(blob)
    const link = document.createElement('a')

    link.href = href
    link.download = filename
    link.click()

    URL.revokeObjectURL(href)
  }

  return (
    <Card className="py-6">
      <CardHeader className="px-6">
        <CardTitle className="flex items-center gap-2">
          <Download className="size-4" />
          Levar embora e trazer de volta
        </CardTitle>
        <CardDescription>
          O arquivo exportado é selado com uma senha só dele — nunca a senha-mestra. Ele guarda no
          próprio cabeçalho como foi fechado, para que ainda abra daqui a alguns anos.
        </CardDescription>
      </CardHeader>

      <CardContent className="space-y-4 px-6">
        {live.length > 1 && (
          <div className="space-y-2">
            <Label>Cofre</Label>
            <Select value={vaultId} onValueChange={setVaultId}>
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {live.map((vault) => (
                  <SelectItem key={vault.id} value={vault.id}>
                    {vault.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        )}

        <div className="space-y-2">
          <Label htmlFor="archive-passphrase">Senha do arquivo</Label>
          <div className="flex items-center gap-2">
            <PasswordInput
              id="archive-passphrase"
              value={passphrase}
              onChange={(event) => setPassphrase(event.target.value)}
            />
            <Button
              variant="outline"
              disabled={busy || !chosen || passphrase.length < 12}
              onClick={() =>
                void attempt(async () => {
                  const answer = await api.exportVault(chosen!.id, passphrase)
                  const today = new Date().toISOString().slice(0, 10)

                  save(
                    JSON.stringify(answer.archive, null, 2),
                    `keyhold-${chosen!.name.toLowerCase().replace(/\s+/g, '-')}-${today}.json`,
                  )

                  setPassphrase('')

                  return 'Arquivo salvo. Guarde a senha dele junto, ela não está em lugar nenhum.'
                })
              }
            >
              {busy ? <Loader2 className="animate-spin" /> : <Download />}
              Exportar
            </Button>
          </div>
          <p className="text-sm text-muted-foreground">
            Mínimo de 12 caracteres. Para importar de volta, é esta senha que abre o arquivo.
          </p>
        </div>

        <div className="space-y-2 border-t pt-4">
          <Label>Trazer registros para {chosen?.name ?? 'este cofre'}</Label>

          <input
            ref={filePicker}
            type="file"
            accept=".json,.csv,text/csv,application/json"
            className="hidden"
            onChange={(event) => {
              const file = event.target.files?.[0]
              event.target.value = ''

              if (!file || !chosen) {
                return
              }

              void attempt(async () => {
                const text = await file.text()

                // A keyhold archive is JSON that says what it is; anything else is treated as the
                // CSV every other password manager exports.
                const done = file.name.endsWith('.json')
                  ? await api.importArchive(chosen.id, JSON.parse(text), passphrase)
                  : await api.importCsv(chosen.id, text)

                onChanged()

                return `${done.imported.records} ${
                  done.imported.records === 1 ? 'registro trazido' : 'registros trazidos'
                }${done.imported.folders > 0 ? `, em ${done.imported.folders} pasta(s) nova(s)` : ''}.`
              })
            }}
          />

          <Button
            variant="outline"
            disabled={busy || !chosen}
            onClick={() => filePicker.current?.click()}
          >
            <Upload />
            Escolher um arquivo
          </Button>

          <p className="text-sm text-muted-foreground">
            Um arquivo do Keyhold (<span className="text-foreground">.json</span>, pede a senha
            acima) ou um CSV de outro gerenciador. Nada é sobrescrito: tudo entra como registro
            novo.
          </p>
        </div>

        {note && <p className="text-sm text-muted-foreground">{note}</p>}
        {error && <p className="text-sm text-destructive">{error}</p>}
      </CardContent>
    </Card>
  )
}
