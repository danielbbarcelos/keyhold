import { CreditCard, FileText, KeyRound, ScrollText, Terminal } from 'lucide-react'
import type { LucideIcon } from 'lucide-react'

import type { Field } from '../api'

/**
 * The kinds of thing a vault holds, and what each one starts with.
 *
 * These are **templates, not schemas.** The encrypted payload has the same shape for every kind, so
 * changing a record's kind never migrates anything and never risks a blob that no longer opens.
 * What a kind decides is which fields somebody almost certainly wants prefilled.
 */
export interface RecordKind {
  value: string
  label: string
  hint: string
  starter: Field[]
  icon: LucideIcon
  /** The tile behind the icon: tells a key from a note at a glance, in a long list. */
  tone: string
}

export const RECORD_KINDS: RecordKind[] = [
  {
    value: 'login',
    label: 'Login',
    hint: 'Usuário e senha de um site ou sistema.',
    starter: [{ label: 'Senha', kind: 'password', value: '' }],
    icon: KeyRound,
    tone: 'bg-sky-500/15 text-sky-500',
  },
  {
    value: 'note',
    label: 'Nota segura',
    hint: 'Um texto que não pode ficar em qualquer lugar.',
    starter: [],
    icon: FileText,
    tone: 'bg-amber-500/15 text-amber-500',
  },
  {
    value: 'ssh_key',
    label: 'Chave SSH',
    hint: 'Chave privada, frase secreta e onde ela é usada.',
    starter: [
      { label: 'Chave privada', kind: 'hidden', value: '' },
      { label: 'Frase secreta', kind: 'password', value: '' },
    ],
    icon: Terminal,
    tone: 'bg-zinc-500/20 text-zinc-400',
  },
  {
    value: 'certificate',
    label: 'Certificado',
    hint: 'Certificado, senha e validade.',
    starter: [
      { label: 'Senha', kind: 'password', value: '' },
      { label: 'Validade', kind: 'date', value: '' },
    ],
    icon: ScrollText,
    tone: 'bg-violet-500/15 text-violet-500',
  },
  {
    value: 'card',
    label: 'Cartão',
    hint: 'Número, validade e código de segurança.',
    starter: [
      { label: 'Número', kind: 'password', value: '' },
      { label: 'Validade', kind: 'text', value: '' },
      { label: 'Código', kind: 'password', value: '' },
    ],
    icon: CreditCard,
    tone: 'bg-emerald-500/15 text-emerald-500',
  },
]

export function kindOf(value: string): RecordKind {
  return RECORD_KINDS.find((kind) => kind.value === value) ?? RECORD_KINDS[0]
}

export function kindIcon(value: string): LucideIcon {
  return kindOf(value).icon
}

export function kindTone(value: string): string {
  return kindOf(value).tone
}

/** What goes under a record's title in the list: who it logs in as, or else what it is. */
export function subtitleOf(record: { kind: string; username: string }): string {
  return record.username || kindOf(record.kind).label
}
