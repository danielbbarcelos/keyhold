import {
  AlignLeft,
  AtSign,
  Calendar,
  CreditCard,
  EyeOff,
  FileLock,
  Globe,
  Grid3x3,
  KeyRound,
  Mail,
  MapPin,
  Phone,
  ShieldCheck,
  ShieldQuestion,
  Type,
  User,
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'

import type { Field, FieldKind, Payload } from '../api'
import { t } from '../i18n'

/**
 * The fifteen kinds of field a record can hold.
 *
 * A record has no type of its own any more: it is a title, an icon, notes and whatever fields it
 * needs. A field's kind decides how it is drawn and whether it is a secret, never how it is stored,
 * so changing a field's kind is always safe.
 */
export interface FieldType {
  kind: FieldKind
  label: string
  icon: LucideIcon
  /** Kept under dots, never in a list, and what the main button copies. */
  secret: boolean
  /** More than one line. */
  multiline?: boolean
  /** The value is chosen by a generator rather than typed. */
  generated?: boolean
  placeholder?: string
}

export const FIELD_TYPES: FieldType[] = [
  { kind: 'text', get label() { return t('Texto') }, icon: Type, secret: false },
  { kind: 'multiline', get label() { return t('Texto de várias linhas') }, icon: AlignLeft, secret: false, multiline: true },
  { kind: 'multiline_hidden', get label() { return t('Várias linhas oculto') }, icon: FileLock, secret: true, multiline: true },
  { kind: 'hidden', get label() { return t('Campo oculto') }, icon: EyeOff, secret: true },
  { kind: 'password', get label() { return t('Senha') }, icon: KeyRound, secret: true, generated: true },
  { kind: 'pin', get label() { return t('Código PIN') }, icon: Grid3x3, secret: true, placeholder: '0000' },
  {
    kind: 'totp',
    get label() { return t('Código 2FA') },
    icon: ShieldCheck,
    secret: true,
    get placeholder() { return t('Segredo ou endereço otpauth://') },
  },
  { kind: 'email', get label() { return t('E-mail') }, icon: Mail, secret: false, get placeholder() { return t('nome@exemplo.com') } },
  { kind: 'name', get label() { return t('Nome') }, icon: User, secret: false },
  { kind: 'phone', get label() { return t('Telefone') }, icon: Phone, secret: false, placeholder: '+55 11 90000-0000' },
  { kind: 'url', get label() { return t('Endereço web') }, icon: Globe, secret: false, get placeholder() { return t('exemplo.com') } },
  { kind: 'address', get label() { return t('Endereço') }, icon: MapPin, secret: false, multiline: true },
  { kind: 'date', get label() { return t('Data') }, icon: Calendar, secret: false },
  { kind: 'security_question', get label() { return t('Pergunta e resposta') }, icon: ShieldQuestion, secret: true },
  { kind: 'card', get label() { return t('Cartão de pagamento') }, icon: CreditCard, secret: true },
]

const FALLBACK: FieldType = { kind: 'text', get label() { return t('Texto') }, icon: AtSign, secret: false }

export function fieldType(kind: string): FieldType {
  return FIELD_TYPES.find((type) => type.kind === kind) ?? { ...FALLBACK, kind: kind as FieldKind }
}

export function isSecret(field: Pick<Field, 'kind'>): boolean {
  return fieldType(field.kind).secret
}

/** A new, empty field of a kind, labelled the way people usually label it. */
export function newField(kind: FieldKind): Field {
  const type = fieldType(kind)
  const label =
    kind === 'security_question'
      ? t('Pergunta de segurança')
      : kind === 'card'
        ? t('Cartão')
        : kind === 'multiline_hidden'
          ? t('Texto oculto')
          : type.label

  return { label, kind, value: '', ...(hasParts(kind) ? { extra: {} } : {}) }
}

/** Composite fields keep their other parts in `extra`. */
export function hasParts(kind: string): boolean {
  return kind === 'security_question' || kind === 'card'
}

export const CARD_PARTS: Array<{ key: string; label: string; secret: boolean; placeholder?: string }> = [
  { key: 'holder', get label() { return t('Nome no cartão') }, secret: false },
  { key: 'expiry', get label() { return t('Validade') }, secret: false, get placeholder() { return t('MM/AA') } },
  { key: 'code', get label() { return t('Código de segurança') }, secret: true, placeholder: 'CVV' },
  { key: 'pin', get label() { return t('PIN') }, secret: true },
]

/**
 * What goes under the title in a list: the field marked for it, or the first e-mail or name.
 * The same rule as the binary's `Payload::subtitle`, so the editor can show it before saving.
 */
export function subtitleField(fields: Field[]): Field | undefined {
  const shown = fields.filter((f) => !isSecret(f) && f.value.trim() !== '')

  return shown.find((f) => f.list) ?? shown.find((f) => ['email', 'name'].includes(f.kind))
}

/** The field the main button copies: the one marked, or the first secret that is not a 2FA code. */
export function primaryField(fields: Field[]): Field | undefined {
  // Never a 2FA field: its value is the seed that makes codes, which is exactly what the
  // interface promises never to hand out.
  const secrets = fields.filter((f) => isSecret(f) && (f.value !== '' || f.sealed !== undefined) && f.kind !== 'totp')

  return secrets.find((f) => f.primary) ?? secrets[0]
}

export function emptyPayload(): Payload {
  return { title: '', notes: '', fields: [], icon: null }
}
