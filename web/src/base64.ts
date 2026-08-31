/**
 * Base64url, the alphabet keyhold uses for every opaque value on the wire.
 *
 * Written out rather than pulled in, and deliberately not via `btoa`: `btoa` takes a string, so a
 * file's bytes would have to be turned into one first — which is where a binary attachment quietly
 * becomes corrupt, because a byte above 0x7f is not a character.
 */
const ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_'

export function toBase64url(bytes: Uint8Array): string {
  let out = ''

  for (let at = 0; at < bytes.length; at += 3) {
    const one = bytes[at]
    const two = bytes[at + 1]
    const three = bytes[at + 2]
    const packed = (one << 16) | ((two ?? 0) << 8) | (three ?? 0)

    out += ALPHABET[(packed >> 18) & 63]
    out += ALPHABET[(packed >> 12) & 63]

    if (two !== undefined) {
      out += ALPHABET[(packed >> 6) & 63]
    }

    if (three !== undefined) {
      out += ALPHABET[packed & 63]
    }
  }

  return out
}

export function fromBase64url(text: string): Uint8Array<ArrayBuffer> {
  const out: number[] = []
  let bits = 0
  let held = 0

  for (const character of text) {
    const value = ALPHABET.indexOf(character)

    if (value < 0) {
      continue
    }

    bits = (bits << 6) | value
    held += 6

    if (held >= 8) {
      held -= 8
      out.push((bits >> held) & 0xff)
    }
  }

  // Backed by a plain ArrayBuffer rather than whatever the runtime feels like: a `Blob` refuses a
  // view over a SharedArrayBuffer, and that is a type error worth keeping rather than casting away.
  const bytes = new Uint8Array(new ArrayBuffer(out.length))
  bytes.set(out)

  return bytes
}
