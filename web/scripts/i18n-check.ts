#!/usr/bin/env node
/**
 * i18n check: every Portuguese text on screen goes through `t()`, and every `t()` has English.
 *
 * Uses the same extraction as the copy lint (string literals and JSX text of src/, comments
 * skipped). A copy string outside `t()` is untranslated; a `t()` key missing from the English
 * catalog would show Portuguese to an English reader; a template literal in `t()` cannot be a key.
 *
 *   npm run i18n:check                 fail on any problem
 *   npm run i18n:check -- --summary    count per file, for tracking progress
 *   npm run i18n:check -- <dir>        only under src/<dir>
 */

import * as fs from 'node:fs'
import * as path from 'node:path'
import { fileURLToPath } from 'node:url'

import { EN } from '../src/i18n/en/index.ts'

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const SRC = path.join(ROOT, 'src')

interface Piece {
  line: number
  text: string
  attr?: string
  tag?: string
  log?: boolean
  sep?: boolean
  wrapped?: boolean
  quote?: string
}

const PT_WORDS = /\b(de|do|da|dos|das|para|com|em|não|os|as|um|uma|ao|no|na|nos|nas|se|que|é|está|são|foi|ser|ou|por|sem|seu|sua|este|esta|esse|essa|já|mais|como|quando|antes|depois|aqui|ainda|cada|todo|toda|nenhum|nenhuma)\b/i
const ACCENT = /[áàâãéêíóôõúçÁÀÂÃÉÊÍÓÔÕÚÇ]/
const CLASSY = /^[a-z0-9:\-[\]/.%#()_!,&>*=@ ]+$/

/** Whether a string reads as Portuguese copy (and not a class name, key, path or identifier). */
function isCopy(s: string): boolean {
  const t = s.trim()
  if (t.length < 2 || !/[A-Za-zÀ-ú]/.test(t)) return false
  if (/^(https?:|\/|\.\.?\/|@\/|data:|mailto:)/.test(t)) return false
  // A capitalised phrase closed by sentence punctuation is copy even without an accent ("Carregando...", "Copiado!").
  if (/^[A-ZÀ-Ú][a-zà-ú]+([ ,][\wÀ-ú-]+)*(\.\.\.|…|!|\.)$/.test(t)) return true
  if (/^[\w.\-/:]+$/.test(t) && !ACCENT.test(t)) return false
  if (CLASSY.test(t) && !ACCENT.test(t) && !PT_WORDS.test(t)) return false
  return ACCENT.test(t) || PT_WORDS.test(t)
}

/** String literals and JSX text of a TS/TSX source, comments skipped. */
function lexJs(src: string): Piece[] {
  const out: Piece[] = []
  const code: string[] = []
  const n = src.length
  let i = 0
  let line = 1

  while (i < n) {
    const c = src[i]
    const next = src[i + 1] ?? ''

    if (c === '\n') { line++; code.push(c); i++; continue }

    if (c === '/' && next === '/') {
      let j = src.indexOf('\n', i)
      if (j === -1) j = n
      code.push(' '.repeat(j - i)); i = j; continue
    }

    if (c === '/' && next === '*') {
      let j = src.indexOf('*/', i + 2)
      j = j === -1 ? n : j + 2
      const seg = src.slice(i, j)
      line += seg.split('\n').length - 1
      code.push(seg.replace(/[^\n]/g, ' ')); i = j; continue
    }

    if (c === "'" || c === '"' || c === '`') {
      const startLine = line
      const before = src.slice(Math.max(0, i - 40), i)
      const attr = before.match(/(\w+)\s*=\s*\{?\s*$/)?.[1] ?? before.match(/(\w+):\s*$/)?.[1]
      const log = /console\.\w+\(\s*$/.test(before)
      const wrapped = /(^|[^\w.])t\(\s*$/.test(before)
      let j = i + 1
      let buf = ''
      while (j < n) {
        const d = src[j]
        if (d === '\\') { buf += src.slice(j, j + 2); j += 2; continue }
        if (c === '`' && d === '$' && src[j + 1] === '{') {
          let depth = 1
          j += 2
          while (j < n && depth) {
            if (src[j] === '{') depth++
            else if (src[j] === '}') depth--
            else if (src[j] === '\n') line++
            j++
          }
          buf += '${}'
          continue
        }
        if (d === c) break
        if (d === '\n') {
          if (c !== '`') break
          line++
        }
        buf += d
        j++
      }
      out.push({ line: startLine, text: buf, attr, log, wrapped, quote: c })
      code.push(c + src.slice(i + 1, j).replace(/[^\n]/g, ' ') + c)
      i = j + 1
      continue
    }

    code.push(c); i++
  }

  const blanked = code.join('')
  const jsx = />([^<>{}]+)(?=[<{])/g
  let m: RegExpExecArray | null
  while ((m = jsx.exec(blanked))) {
    const text = m[1]
    if (/[A-Za-zÀ-ú]/.test(text) && !/=>|&&|\|\|/.test(text)) {
      const open = blanked.lastIndexOf('<', m.index)
      const tag = open >= 0 ? blanked.slice(open + 1).match(/^([A-Za-z][\w.]*)/)?.[1] : undefined
      out.push({ line: blanked.slice(0, m.index).split('\n').length, text: text.split(/\s+/).filter(Boolean).join(' '), tag })
    }
  }

  // A dash between JSX expressions is a separator written as text, which the letter-only filter
  // above never sees: "{code} — {description}", "> — {description}".
  const separator = /(\}|>)[ \t]+—[ \t]+(?=\{)/g
  while ((m = separator.exec(blanked))) {
    out.push({ line: blanked.slice(0, m.index).split('\n').length, text: ' — ', sep: true })
  }

  return out
}


/**
 * Files that are not screens: the catalogs themselves, the API's types. And the icon list, whose
 * Portuguese is search keywords (with English synonyms beside them), never shown; its colour names
 * do go through `t()`.
 */
const SKIP = [/^i18n\//, /\.test\.tsx?$/, /^components\/ui\//, /^workspace\/icons\.tsx$/]

function files(dir: string): string[] {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(dir, entry.name)
    if (entry.isDirectory()) return files(full)
    return /\.(tsx?|ts)$/.test(entry.name) ? [full] : []
  })
}

const args = process.argv.slice(2)
const summary = args.includes('--summary')
const only = args.find((a) => !a.startsWith('--'))
const problems: Array<{ file: string; line: number; kind: string; text: string }> = []

const start = only ? path.join(SRC, only) : SRC
for (const file of fs.statSync(start).isFile() ? [start] : files(start)) {
  const rel = path.relative(SRC, file).split(path.sep).join('/')
  if (SKIP.some((re) => re.test(rel))) continue
  for (const piece of lexJs(fs.readFileSync(file, 'utf8'))) {
    if (piece.log || piece.sep) continue
    if (piece.wrapped) {
      if (piece.quote === '`') problems.push({ file: rel, line: piece.line, kind: 'template', text: piece.text })
      else if (!(piece.text in EN)) problems.push({ file: rel, line: piece.line, kind: 'sem inglês', text: piece.text })
      continue
    }
    if (isCopy(piece.text)) problems.push({ file: rel, line: piece.line, kind: 'fora de t()', text: piece.text.trim() })
  }
}

if (summary) {
  const counts = new Map<string, number>()
  for (const p of problems) counts.set(p.file, (counts.get(p.file) ?? 0) + 1)
  for (const [file, count] of [...counts].sort((a, b) => b[1] - a[1])) console.log(`${String(count).padStart(5)}  ${file}`)
  console.log(`${problems.length} pendência(s)`)
} else {
  for (const p of problems) console.log(`${p.file}:${p.line}  [${p.kind}]  ${p.text}`)
  console.log(problems.length ? `\n${problems.length} pendência(s)` : 'i18n OK')
}

process.exit(problems.length ? 1 : 0)
