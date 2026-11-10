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
  { kind: 'text', label: 'Texto', icon: Type, secret: false },
  { kind: 'multiline', label: 'Texto de várias linhas', icon: AlignLeft, secret: false, multiline: true },
  { kind: 'multiline_hidden', label: 'Várias linhas oculto', icon: FileLock, secret: true, multiline: true },
  { kind: 'hidden', label: 'Campo oculto', icon: EyeOff, secret: true },
  { kind: 'password', label: 'Senha', icon: KeyRound, secret: true, generated: true },
  { kind: 'pin', label: 'Código PIN', icon: Grid3x3, secret: true, placeholder: '0000' },
  {
    kind: 'totp',
    label: 'Código 2FA',
    icon: ShieldCheck,
    secret: true,
    placeholder: 'Segredo ou endereço otpauth://',
  },
  { kind: 'email', label: 'E-mail', icon: Mail, secret: false, placeholder: 'nome@exemplo.com' },
  { kind: 'name', label: 'Nome', icon: User, secret: false },
  { kind: 'phone', label: 'Telefone', icon: Phone, secret: false, placeholder: '+55 11 90000-0000' },
  { kind: 'url', label: 'Endereço web', icon: Globe, secret: false, placeholder: 'exemplo.com' },
  { kind: 'address', label: 'Endereço', icon: MapPin, secret: false, multiline: true },
  { kind: 'date', label: 'Data', icon: Calendar, secret: false },
  { kind: 'security_question', label: 'Pergunta e resposta', icon: ShieldQuestion, secret: true },
  { kind: 'card', label: 'Cartão de pagamento', icon: CreditCard, secret: true },
]

const FALLBACK: FieldType = { kind: 'text', label: 'Texto', icon: AtSign, secret: false }

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
      ? 'Pergunta de segurança'
      : kind === 'card'
        ? 'Cartão'
        : kind === 'multiline_hidden'
          ? 'Texto oculto'
          : type.label

  return { label, kind, value: '', ...(hasParts(kind) ? { extra: {} } : {}) }
}

/** Composite fields keep their other parts in `extra`. */
export function hasParts(kind: string): boolean {
  return kind === 'security_question' || kind === 'card'
}

export const CARD_PARTS: Array<{ key: string; label: string; secret: boolean; placeholder?: string }> = [
  { key: 'holder', label: 'Nome no cartão', secret: false },
  { key: 'expiry', label: 'Validade', secret: false, placeholder: 'MM/AA' },
  { key: 'code', label: 'Código de segurança', secret: true, placeholder: 'CVV' },
  { key: 'pin', label: 'PIN', secret: true },
]

/**
 * What goes under the title in a list: the field marked for it, or the first e-mail, name or text.
 * The same rule as the binary's `Payload::subtitle`, so the editor can show it before saving.
 */
export function subtitleField(fields: Field[]): Field | undefined {
  const shown = fields.filter((f) => !isSecret(f) && f.value.trim() !== '')

  return shown.find((f) => f.list) ?? shown.find((f) => ['email', 'name', 'text'].includes(f.kind))
}

/** The field the main button copies: the one marked, or the first secret that is not a 2FA code. */
export function primaryField(fields: Field[]): Field | undefined {
  const secrets = fields.filter((f) => isSecret(f) && f.value !== '')

  return secrets.find((f) => f.primary) ?? secrets.find((f) => f.kind !== 'totp')
}

/**
 * Starting points for a new record. A template is not a type: it only adds fields, and the record
 * is free to lose or gain any of them afterwards.
 */
export interface Template {
  id: string
  label: string
  icon: string
  colour: string
  fields: Array<[FieldKind, string?]>
}

export const TEMPLATES: Template[] = [
  {
    id: 'login',
    label: 'Login',
    icon: 'key-round',
    colour: 'sky',
    fields: [['email'], ['password'], ['url'], ['totp']],
  },
  {
    id: 'cartao',
    label: 'Cartão',
    icon: 'credit-card',
    colour: 'mint',
    fields: [['card'], ['phone', 'Central de atendimento']],
  },
  {
    id: 'ssh',
    label: 'Chave SSH',
    icon: 'terminal',
    colour: 'stone',
    fields: [['text', 'Servidor'], ['text', 'Usuário'], ['multiline_hidden', 'Chave privada'], ['password', 'Frase secreta']],
  },
  { id: 'nota', label: 'Nota', icon: 'file-text', colour: 'sand', fields: [['multiline_hidden', 'Nota']] },
]

export function fieldsOf(template: Template): Field[] {
  return template.fields.map(([kind, label]) => ({ ...newField(kind), ...(label ? { label } : {}) }))
}

export function emptyPayload(): Payload {
  return { title: '', notes: '', fields: [], icon: null }
}
