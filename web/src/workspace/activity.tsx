import {
  ArrowRightLeft,
  Copy,
  CopyPlus,
  Eye,
  FileDown,
  LogIn,
  Pencil,
  Plus,
  ScanEye,
  Share2,
  Trash2,
  type LucideIcon,
} from 'lucide-react'

import type { ActivityEvent } from '../api'

/**
 * How each audit action reads on screen. One table for the record's Atividade and the vault's
 * Trilha, and for both kinds of vault: the local trail's "secret.*" arrives already as "field.*".
 */
interface Kind {
  /** Short label for the Trilha's event column. */
  label: string
  icon: LucideIcon
  /** The rest of the sentence after the person's name. */
  sentence: (detail: Record<string, unknown>) => React.ReactNode
}

const text = (value: unknown) => (typeof value === 'string' ? value : '')

const KINDS: Record<string, Kind> = {
  'field.revealed': {
    label: 'Revelou',
    icon: Eye,
    sentence: (d) => (
      <>
        revelou o campo <span className="font-medium text-foreground">{text(d.field)}</span>
      </>
    ),
  },
  'field.copied': {
    label: 'Copiou',
    icon: Copy,
    sentence: (d) => (
      <>
        copiou o campo <span className="font-medium text-foreground">{text(d.field)}</span>
      </>
    ),
  },
  'record.viewed': { label: 'Visualizou', icon: ScanEye, sentence: () => 'visualizou o registro' },
  'record.created': { label: 'Criou', icon: Plus, sentence: () => 'criou o registro' },
  'record.updated': {
    label: 'Alterou',
    icon: Pencil,
    sentence: (d) => {
      const fields = Array.isArray(d.fields) ? d.fields.filter((f): f is string => typeof f === 'string') : []

      return fields.length ? (
        <>
          alterou {fields.length > 1 ? 'os campos' : 'o campo'}{' '}
          <span className="font-medium text-foreground">{fields.join(', ')}</span>
        </>
      ) : (
        'alterou o registro'
      )
    },
  },
  'record.moved': {
    label: 'Moveu',
    icon: ArrowRightLeft,
    sentence: (d) =>
      d.from && d.to ? (
        <>
          moveu de <span className="font-mono text-[13px] text-foreground">{text(d.from)}</span> para{' '}
          <span className="font-mono text-[13px] text-foreground">{text(d.to)}</span>
        </>
      ) : (
        'moveu o registro'
      ),
  },
  'record.deleted': { label: 'Excluiu', icon: Trash2, sentence: () => 'excluiu o registro' },
  'record.copied': { label: 'Copiou registro', icon: CopyPlus, sentence: () => 'copiou o registro para outro cofre' },
  'attachment.downloaded': {
    label: 'Baixou anexo',
    icon: FileDown,
    sentence: (d) => (
      <>
        baixou o anexo <span className="font-medium text-foreground">{text(d.attachment)}</span>
      </>
    ),
  },
  'record.shared': {
    label: 'Compartilhou',
    icon: Share2,
    sentence: (d) =>
      d.recipient ? (
        <>
          compartilhou com <span className="font-medium text-foreground">{text(d.recipient)}</span>
        </>
      ) : (
        'compartilhou o registro'
      ),
  },
  'share.opened': { label: 'Link aberto', icon: Share2, sentence: () => 'abriu o link compartilhado' },
  'share.revealed': { label: 'Link revelado', icon: Share2, sentence: () => 'revelou o conteúdo do link compartilhado' },
  'session.opened': { label: 'Abriu sessão', icon: LogIn, sentence: () => 'abriu sessão' },
}

export function kindOf(action: string): Kind {
  return KINDS[action] ?? { label: action, icon: Pencil, sentence: () => action }
}

/** The actions a person can filter the vault's Trilha by, in the order the menu shows them. */
export const FILTERABLE: Array<[string, string]> = [
  ['field.revealed', 'Revelou'],
  ['field.copied', 'Copiou'],
  ['record.viewed', 'Visualizou'],
  ['record.updated', 'Alterou'],
  ['record.moved', 'Moveu'],
  ['record.deleted', 'Excluiu'],
  ['attachment.downloaded', 'Baixou anexo'],
  ['record.shared', 'Compartilhou'],
]

/** When it happened: CloudWatch's time for AWS, the trail's for a local vault. */
export function momentOf(event: ActivityEvent): Date | null {
  if (typeof event.at_ms === 'number') return new Date(event.at_ms)
  if (event.at) return new Date(event.at)

  return null
}

/** "hoje, 14:32", "ontem, 18:20" or "12/11/2026, 16:02". */
export function whenShort(date: Date | null): string {
  if (!date || Number.isNaN(date.getTime())) return ''

  const time = date.toLocaleTimeString('pt-BR', { hour: '2-digit', minute: '2-digit' })
  const today = new Date()
  const yesterday = new Date(today)
  yesterday.setDate(today.getDate() - 1)

  if (date.toDateString() === today.toDateString()) return `hoje, ${time}`
  if (date.toDateString() === yesterday.toDateString()) return `ontem, ${time}`

  return `${date.toLocaleDateString('pt-BR')}, ${time}`
}
