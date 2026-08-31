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
}

export const RECORD_KINDS: RecordKind[] = [
  {
    value: 'login',
    label: 'Login',
    hint: 'Usuário e senha de um site ou sistema.',
    starter: [{ label: 'Senha', kind: 'password', value: '' }],
    icon: KeyRound,
  },
  {
    value: 'note',
    label: 'Nota segura',
    hint: 'Um texto que não pode ficar em qualquer lugar.',
    starter: [],
    icon: FileText,
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
  },
]

export function kindOf(value: string): RecordKind {
  return RECORD_KINDS.find((kind) => kind.value === value) ?? RECORD_KINDS[0]
}

export function kindIcon(value: string): LucideIcon {
  return kindOf(value).icon
}
