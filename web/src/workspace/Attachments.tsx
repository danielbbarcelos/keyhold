import { useEffect, useRef, useState } from 'react'
import { Download, Loader2, Paperclip, Trash2 } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

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
  const [over, setOver] = useState(false)
  const picker = useRef<HTMLInputElement>(null)

  function upload(file: File) {
    void attempt(async () => {
      const bytes = new Uint8Array(await file.arrayBuffer())

      await api.addAttachment(
        recordId,
        file.name,
        file.type || 'application/octet-stream',
        toBase64url(bytes),
      )
    })
  }

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
      <h3 className="pt-2 text-sm font-bold">
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

          if (file) {
            upload(file)
          }
        }}
      />

      {/* A place to drop a file as well as a button to pick one: dragging from the file manager is
          how most people already hold the file they mean to keep. */}
      <button
        type="button"
        disabled={busy}
        onClick={() => picker.current?.click()}
        onDragOver={(event) => {
          event.preventDefault()
          setOver(true)
        }}
        onDragLeave={() => setOver(false)}
        onDrop={(event) => {
          event.preventDefault()
          setOver(false)

          const file = event.dataTransfer.files?.[0]

          if (file) {
            upload(file)
          }
        }}
        className={cn(
          'flex w-full flex-col items-center gap-1.5 rounded-xl border border-dashed border-input px-3 py-5 text-center text-xs text-muted-foreground transition-colors hover:bg-hover',
          over && 'border-ring bg-hover',
        )}
      >
        {busy ? <Loader2 className="size-5 animate-spin" /> : <Paperclip className="size-5" />}
        <span className="text-sm font-semibold text-foreground">Anexar um arquivo</span>
        {maxBytes > 0 && (
          <span>
            Até {Math.round(maxBytes / (1024 * 1024))} MB. O nome e o tipo são selados junto com o
            conteúdo.
          </span>
        )}
      </button>

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
