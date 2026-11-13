import { useEffect, useState } from 'react'

/**
 * Whether the browser may fill in and save a record's password and hidden fields. Off unless
 * somebody switched it on in Ajustes.
 *
 * Kept here rather than passed down: the inputs it decides about sit several components deep in
 * the record editor, and every one of them has to agree.
 */
const CHANGED = 'keyhold:field-autocomplete'

let allowed = false

export function setFieldAutocomplete(on: boolean) {
  if (on === allowed) return
  allowed = on
  window.dispatchEvent(new Event(CHANGED))
}

export function useFieldAutocomplete(): boolean {
  const [, tick] = useState(0)

  useEffect(() => {
    const again = () => tick((n) => n + 1)
    window.addEventListener(CHANGED, again)

    return () => window.removeEventListener(CHANGED, again)
  }, [])

  return allowed
}

/**
 * What an input holding a secret tells the browser.
 *
 * Allowed: an ordinary password field, which the browser and its password manager treat as one.
 *
 * Not allowed: never `type="password"`. Browsers ignore `autocomplete="off"` on a password field
 * on purpose, so the only way to be left alone is not to be one. The value is hidden with
 * `-webkit-text-security` instead (the class below), and the attributes the usual password
 * managers look for say to skip the field.
 */
export function secretInputProps(showing: boolean, allow: boolean) {
  if (allow) {
    return { type: showing ? 'text' : 'password', autoComplete: 'on' } as const
  }

  return {
    type: 'text',
    autoComplete: 'off',
    'data-1p-ignore': true,
    'data-lpignore': 'true',
    'data-bwignore': true,
    'data-form-type': 'other',
    'data-concealed': showing ? undefined : true,
  } as const
}
