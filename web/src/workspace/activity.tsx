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

import { locale, t } from '@/i18n'

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

/** Built at render, so the words follow the language in use. */
const kinds = (): Record<string, Kind> => ({
  'field.revealed': {
    label: t('Revelou'),
    icon: Eye,
    sentence: (d) => (
      <>
        {t('revelou o campo')} <span className="font-medium text-foreground">{text(d.field)}</span>
      </>
    ),
  },
  'field.copied': {
    label: t('Copiou'),
    icon: Copy,
    sentence: (d) => (
      <>
        {t('copiou o campo')} <span className="font-medium text-foreground">{text(d.field)}</span>
      </>
    ),
  },
  'record.viewed': { label: t('Visualizou'), icon: ScanEye, sentence: () => t('visualizou o registro') },
  'record.created': { label: t('Criou'), icon: Plus, sentence: () => t('criou o registro') },
  'record.updated': {
    label: t('Alterou'),
    icon: Pencil,
    sentence: (d) => {
      const fields = Array.isArray(d.fields) ? d.fields.filter((f): f is string => typeof f === 'string') : []

      return fields.length ? (
        <>
          {fields.length > 1 ? t('alterou os campos') : t('alterou o campo')}{' '}
          <span className="font-medium text-foreground">{fields.join(', ')}</span>
        </>
      ) : (
        t('alterou o registro')
      )
    },
  },
  'record.moved': {
    label: t('Moveu'),
    icon: ArrowRightLeft,
    sentence: (d) =>
      d.from && d.to ? (
        <>
          {t('moveu de')} <span className="font-mono text-[13px] text-foreground">{text(d.from)}</span> {t('para')}{' '}
          <span className="font-mono text-[13px] text-foreground">{text(d.to)}</span>
        </>
      ) : (
        t('moveu o registro')
      ),
  },
  'record.deleted': { label: t('Excluiu'), icon: Trash2, sentence: () => t('excluiu o registro') },
  'record.copied': { label: t('Copiou registro'), icon: CopyPlus, sentence: () => t('copiou o registro para outro cofre') },
  'attachment.downloaded': {
    label: t('Baixou anexo'),
    icon: FileDown,
    sentence: (d) => (
      <>
        {t('baixou o anexo')} <span className="font-medium text-foreground">{text(d.attachment)}</span>
      </>
    ),
  },
  'record.shared': {
    label: t('Compartilhou'),
    icon: Share2,
    sentence: (d) =>
      d.recipient ? (
        <>
          {t('compartilhou com')} <span className="font-medium text-foreground">{text(d.recipient)}</span>
        </>
      ) : (
        t('compartilhou o registro')
      ),
  },
  'share.opened': { label: t('Link aberto'), icon: Share2, sentence: () => t('abriu o link compartilhado') },
  'share.revealed': { label: t('Link revelado'), icon: Share2, sentence: () => t('revelou o conteúdo do link compartilhado') },
  'session.opened': { label: t('Abriu sessão'), icon: LogIn, sentence: () => t('abriu sessão') },
})

export function kindOf(action: string): Kind {
  return kinds()[action] ?? { label: action, icon: Pencil, sentence: () => action }
}

/** The actions a person can filter the vault's Trilha by, in the order the menu shows them. */
export const filterable = (): Array<[string, string]> => [
  ['field.revealed', t('Revelou')],
  ['field.copied', t('Copiou')],
  ['record.viewed', t('Visualizou')],
  ['record.updated', t('Alterou')],
  ['record.moved', t('Moveu')],
  ['record.deleted', t('Excluiu')],
  ['attachment.downloaded', t('Baixou anexo')],
  ['record.shared', t('Compartilhou')],
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

  const time = date.toLocaleTimeString(locale(), { hour: '2-digit', minute: '2-digit' })
  const today = new Date()
  const yesterday = new Date(today)
  yesterday.setDate(today.getDate() - 1)

  if (date.toDateString() === today.toDateString()) return t('hoje, {time}', { time })
  if (date.toDateString() === yesterday.toDateString()) return t('ontem, {time}', { time })

  return `${date.toLocaleDateString(locale())}, ${time}`
}
