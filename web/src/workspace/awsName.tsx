import { useState } from 'react'

import { Input } from '@/components/ui/input'

/**
 * Parameter Store names, said before anything is sent.
 *
 * A name in Parameter Store takes letters without accents, digits, `.`, `-` and `_`, with `/`
 * between folders. Everything people actually call things ("Banco Inter (PJ)", "Cartões") lives in
 * the description instead, and the name is made from it. These are the same rules the binary
 * checks (`mapping::validate_segment`), here so the field turns red while typing rather than after
 * a round trip to AWS.
 */

const FOLD: Record<string, string> = { ß: 'ss', æ: 'ae', Æ: 'AE', ø: 'o', Ø: 'O', œ: 'oe', Œ: 'OE' }

/** "Senha do Banco (produção)" → `senha-do-banco-producao`. */
export function suggestName(text: string, fallback = 'registro'): string {
  const plain = [...text.normalize('NFD').replace(/[̀-ͯ]/g, '')].map((c) => FOLD[c] ?? c).join('')
  let out = ''
  let gap = false

  for (const c of plain) {
    if (/[A-Za-z0-9_.]/.test(c)) {
      if (gap && out) out += '-'
      gap = false
      out += c.toLowerCase()
    } else {
      gap = true
    }
  }

  return out.replace(/^\.+|\.+$/g, '') || fallback
}

/** Why AWS would refuse this one segment, in words, or `null` when it would take it. */
export function segmentProblem(segment: string): string | null {
  if (!segment) return 'O nome não pode ficar vazio.'

  const bad = [...segment].find((c) => !/[A-Za-z0-9._-]/.test(c))
  if (bad === ' ') return 'Espaço não vale na AWS. Use - ou _.'
  if (bad === '/') return 'A barra separa pastas: aqui vai só um nome.'
  if (bad) return `“${bad}” não vale na AWS: só letras sem acento, números, ponto, hífen e sublinhado.`
  if (segment === '.keep') return 'Esse nome é reservado para as pastas.'

  return null
}

/** Why a full name would be refused: a segment, the reserved prefixes, the size. */
export function nameProblem(parent: string | null, leaf: string): string | null {
  const own = segmentProblem(leaf)
  if (own) return own

  const full = join(parent, leaf)
  const first = full.split('/')[1]?.toLowerCase() ?? ''
  if (first.startsWith('aws') || first.startsWith('ssm')) return 'A AWS reserva nomes que começam com aws ou ssm.'
  if (full.split('/').length - 1 > 15) return 'A AWS só aceita 15 níveis de pasta.'
  if (full.length > 1011) return 'O nome passa de 1011 caracteres.'

  return null
}

/** A title or a folder name: anything at all, except the `/` that would read as one more folder. */
export function titleProblem(title: string, what = 'O título'): string | null {
  return title.includes('/') ? `${what} não pode ter “/”: na AWS isso vira mais uma pasta.` : null
}

export function join(parent: string | null, leaf: string): string {
  return `${(parent ?? '').replace(/\/+$/, '')}/${leaf}`
}

/** The path an AWS folder id stands for (`aws~vault~f~base64url(path)`), or `null`. */
export function awsPathOf(id: string | null | undefined): string | null {
  if (!id?.startsWith('aws~')) return null

  const encoded = id.split('~')[3]
  if (!encoded) return null

  try {
    const base64 = encoded.replace(/-/g, '+').replace(/_/g, '/')
    const bytes = Uint8Array.from(atob(base64 + '='.repeat((4 - (base64.length % 4)) % 4)), (c) => c.charCodeAt(0))

    return new TextDecoder().decode(bytes)
  } catch {
    return null
  }
}

/** The last segment of a name. */
export function leafOf(name: string): string {
  return name.split('/').filter(Boolean).at(-1) ?? ''
}

/**
 * "Na AWS: /app/bancos-pj/banco-inter-pj", under a title or a folder name, with "Editar nome" for
 * when the made-up name will not do. `value` is the name typed by hand, or `null` while it follows
 * `from`.
 */
export function AwsNameField({
  parent,
  from,
  value,
  onChange,
  fallback,
}: {
  parent: string | null
  from: string
  value: string | null
  onChange: (value: string | null) => void
  fallback?: string
}) {
  const [editing, setEditing] = useState(value !== null)
  const leaf = value ?? suggestName(from, fallback)
  const problem = nameProblem(parent, leaf)

  if (!editing) {
    return (
      // The path breaks wherever it has to, and "Editar nome" keeps its place on the right: a path
      // five folders deep is still one readable line or two, never a field pushed off the dialog.
      <span className="flex min-w-0 items-start gap-3 text-xs text-faint">
        <span className="min-w-0 flex-1 break-all">
          Na AWS: <code className="font-mono text-muted-foreground">{join(parent, leaf)}</code>
        </span>
        <button
          type="button"
          className="shrink-0 text-primary hover:underline"
          onClick={() => {
            setEditing(true)
            onChange(leaf)
          }}
        >
          Editar nome
        </button>
      </span>
    )
  }

  return (
    <span className="grid min-w-0 gap-1">
      <span className="break-all font-mono text-xs text-faint">{join(parent, '')}</span>
      <span className="flex items-center gap-2">
        <Input
          className="h-8 font-mono text-[13px]"
          value={leaf}
          aria-invalid={problem !== null}
          aria-label="Nome na AWS"
          spellCheck={false}
          onChange={(event) => onChange(event.target.value)}
        />
      </span>
      <span className="flex flex-wrap items-center gap-x-2 text-xs">
        {problem ? <span className="text-destructive">{problem}</span> : <span className="text-faint">Letras sem acento, números, . - _</span>}
        <button
          type="button"
          className="text-primary hover:underline"
          onClick={() => {
            setEditing(false)
            onChange(null)
          }}
        >
          Usar o sugerido
        </button>
      </span>
    </span>
  )
}
