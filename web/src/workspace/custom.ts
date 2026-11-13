import { useEffect, useState } from 'react'

import { api, type CustomIcon, type Field, type RecordType } from '../api'
import { newField } from './fields'

/**
 * What the person made their own: the icon library and the kinds of record.
 *
 * Read from the binary once and kept here, so the icon picker and the "Começar com" row open with
 * them already in hand. Saving goes through here too, and everything showing them hears about it.
 */
const CHANGED = 'keyhold:custom'

let icons: CustomIcon[] | null = null
let types: RecordType[] | null = null
let loading: Promise<void> | null = null

function announce() {
  window.dispatchEvent(new Event(CHANGED))
}

function load(): Promise<void> {
  loading ??= Promise.all([api.customIcons(), api.recordTypes()])
    .then(([foundIcons, foundTypes]) => {
      icons = foundIcons.icons
      types = foundTypes.types
      announce()
    })
    .catch(() => {
      // Tried again by whoever asks next: a locked vault answers nothing, and that is not an error.
      loading = null
    })

  return loading
}

export function useCustom(): { icons: CustomIcon[]; types: RecordType[]; loaded: boolean } {
  const [, tick] = useState(0)

  useEffect(() => {
    const again = () => tick((n) => n + 1)
    window.addEventListener(CHANGED, again)
    if (icons === null || types === null) void load()

    return () => window.removeEventListener(CHANGED, again)
  }, [])

  return { icons: icons ?? [], types: types ?? [], loaded: icons !== null && types !== null }
}

export async function saveIcons(next: CustomIcon[]): Promise<void> {
  icons = (await api.saveCustomIcons(next)).icons
  announce()
}

/** Keep a picture in the library. `false` when the very same picture was already there. */
export async function addIcon(name: string, image: string): Promise<boolean> {
  if (icons === null) await load()
  if ((icons ?? []).some((one) => one.image === image)) return false

  await saveIcons([...(icons ?? []), { id: crypto.randomUUID(), name: name.trim() || 'Sem nome', image }])

  return true
}

export async function saveTypes(next: RecordType[]): Promise<void> {
  types = (await api.saveRecordTypes(next)).types
  announce()
}

export async function resetTypes(): Promise<void> {
  types = (await api.resetRecordTypes()).types
  announce()
}

/** The fields a new record of this kind starts with, labelled the way the kind says. */
export function fieldsOfType(type: RecordType): Field[] {
  return type.fields.map(({ kind, label }) => ({ ...newField(kind), ...(label ? { label } : {}) }))
}
