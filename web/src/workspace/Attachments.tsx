import { useEffect, useRef, useState } from 'react'
import { Download, Loader2, Paperclip, Trash2 } from 'lucide-react'

import { Button } from '@/components/ui/button'

import { api, ApiError, type AttachmentInfo } from '../api'
import { fromBase64url, toBase64url } from '../base64'

/**
 * The files kept inside a record.
 *
 * Saving one hands it to the browser, which asks where it goes — keyhold does not choose a place on
 * disk for a decrypted copy of anything. The bytes travel as base64 in the response rather than as
 * a download link, because a link cannot carry the session token and a route authenticated by query
 * string would put a credential into browser history.
 */
export function Attachments({ recordId }: { recordId: string }) {
  const [files, setFiles] = useState<AttachmentInfo[] | null>(null)
  const [maxBytes, setMaxBytes] = useState(0)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const picker = useRef<HTMLInputElement>(null)

  const refresh = () =>
    api
      .attachments(recordId)
      .then((answer) => {
        setFiles(answer.attachments)
        setMaxBytes(answer.max_bytes)
      })
      .catch(() => setFiles([]))

  useEffect(() => {
    void refresh()
  }, [recordId])

  async function attempt(action: () => Promise<void>) {
    setBusy(true)

    try {
      await action()
      await refresh()
      setError('')
    } catch (cause) {
      setError(cause instanceof ApiError ? cause.message : 'Não foi possível falar com o Keyhold.')
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="space-y-2">
      <h3 className="flex items-center gap-1.5 text-xs font-medium tracking-wide text-muted-foreground uppercase">
        <Paperclip className="size-3" />
        Arquivos
      </h3>

      {files && files.length > 0 && (
        <ul className="space-y-1 text-sm">
          {files.map((file) => (
            <li key={file.id} className="flex items-center gap-1">
              <span className="min-w-0 flex-1">
                <span className="block truncate">{file.name}</span>
                <span className="block text-xs text-muted-foreground">{size(file.size)}</span>
              </span>

              <Button
                variant="ghost"
                size="icon-xs"
                aria-label={`Salvar ${file.name}`}
                onClick={() =>
                  void attempt(async () => {
                    const opened = await api.openAttachment(file.id)
                    const bytes = fromBase64url(opened.data)
                    const blob = new Blob([bytes], { type: opened.mime })
                    const href = URL.createObjectURL(blob)
                    const link = document.createElement('a')

                    link.href = href
                    link.download = opened.name
                    link.click()

                    URL.revokeObjectURL(href)
                  })
                }
              >
                <Download />
              </Button>

              <Button
                variant="ghost"
                size="icon-xs"
                aria-label={`Remover ${file.name}`}
                onClick={() => void attempt(() => api.deleteAttachment(file.id).then(() => {}))}
              >
                <Trash2 />
              </Button>
            </li>
          ))}
        </ul>
      )}

      <input
        ref={picker}
        type="file"
        className="hidden"
        onChange={(event) => {
          const file = event.target.files?.[0]
          event.target.value = ''

          if (!file) {
            return
          }

          void attempt(async () => {
            const bytes = new Uint8Array(await file.arrayBuffer())

            await api.addAttachment(
              recordId,
              file.name,
              file.type || 'application/octet-stream',
              toBase64url(bytes),
            )
          })
        }}
      />

      <Button
        variant="outline"
        size="sm"
        className="w-full"
        disabled={busy}
        onClick={() => picker.current?.click()}
      >
        {busy ? <Loader2 className="animate-spin" /> : <Paperclip />}
        Anexar um arquivo
      </Button>

      {maxBytes > 0 && (
        <p className="text-xs text-muted-foreground">
          Até {Math.round(maxBytes / (1024 * 1024))} MB. O nome e o tipo são selados junto com o
          conteúdo.
        </p>
      )}

      {error && <p className="text-sm text-destructive">{error}</p>}
    </div>
  )
}

function size(bytes: number): string {
  if (bytes < 1024) {
    return `${bytes} B`
  }

  if (bytes < 1024 * 1024) {
    return `${(bytes / 1024).toFixed(1)} kB`
  }

  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}
