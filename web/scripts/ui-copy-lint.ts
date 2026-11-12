#!/usr/bin/env node
/**
 * UI copy lint, ported from the DBMV workspace (docs/reference/UI_WRITING.md there).
 *
 * Extracts the copy a person reads on screen (string literals and JSX text of src/) and checks the
 * deterministic rules of the writing guide. Rules that need
 * judgment (conversational register, Title Case) are left to review, otherwise the lint lies.
 *
 * The baseline is what keeps it alive: it fails only on what is new compared to
 * ui-copy-lint.baseline.json. Every correction shrinks the baseline, so the file is the scoreboard.
 *
 *   npm run lint:copy                 fail on violations not in the baseline
 *   npm run lint:copy -- --update     rewrite the baseline with today's violations
 *   npm run lint:copy -- --report     print every violation, baseline included
 *   npm run lint:copy -- --json       every violation as JSON, with the full text (for fixers)
 *   npm run lint:copy -- --dump <dir> every copy string under a folder, for the rules that need review
 */

import * as fs from 'node:fs'
import * as path from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const SRC = path.join(ROOT, 'src')
const BASELINE = path.join(ROOT, 'scripts', 'ui-copy-lint.baseline.json')

interface Piece {
  line: number
  text: string
  /** Attribute the string is the value of (label, placeholder…), when it is one. */
  attr?: string
  /** JSX element the text sits in (DialogTitle…), for JSX text. */
  tag?: string
  /** Argument of console.*: a log line, not copy. */
  log?: boolean
  /** A lone "—" glued to a JSX expression ("{code} — {name}"): a separator, not the empty-cell marker. */
  sep?: boolean
}

interface Violation {
  file: string
  line: number
  rule: string
  text: string
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
      out.push({ line: startLine, text: buf, attr, log })
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

const RULES: Array<[string, (p: Piece) => boolean]> = [
  // A lone "—" is the empty-cell marker the guide allows.
  ['travessao', (p) => p.sep === true || (p.text.includes('—') && p.text.trim() !== '—')],
  ['reticencias', (p) => /\.\.\.(?![\w.])/.test(p.text)],
  ['exclamacao', (p) => /[A-Za-zÀ-ú0-9)]!(\s|$)/.test(p.text)],
  ['ponto_e_virgula', (p) => /;\s+\S/.test(p.text.replace(/&#?\w+;/g, ''))],
  ['ex_sem_ponto', (p) => /\bEx:/.test(p.text)],
  ['email_sem_hifen', (p) => /(?<![\w@.-])[Ee]mails?\b(?![-.@\w])/.test(p.text)],
  ['erro_ao', (p) => /^\s*(Erro|Falha) ao\b/.test(p.text)],
  // UI_WRITING §6: say what did not happen, never a generic apology.
  ['erro_generico', (p) => /^\s*(Ocorreu um erro|Houve um erro|Algo deu errado)|\b(Desculpe|Ops)\b/.test(p.text)],
  // UI_WRITING §4: conversational register that has a fixed product equivalent.
  ['registro_conversa', (p) => /\b(sai|saem) d[aeo]s? (lista|listagem|tela|árvore)|\bde vez\b|\bna hora\b|\b(não )?dá para\b|\bpra (frente|trás)\b|\bo jeito é\b|\ba pessoa\b|\b(tem|têm) acesso\b|^Ver( |$)/i.test(p.text)],
  ['com_sucesso', (p) => /\bcom sucesso\b/i.test(p.text)],
  ['por_favor', (p) => /\bpor favor\b/i.test(p.text)],
  ['tem_certeza', (p) => /\btem certeza\b/i.test(p.text)],
  // Only a placeholder gives no orders: "Digite X para confirmar" as a label is a legitimate instruction.
  ['placeholder_instrucao', (p) => /placeholder/i.test(p.attr ?? '') && /^\s*(Digite|Insira|Informe|Preencha)\b/.test(p.text)],
  // UI_WRITING §7: "…" only in search, select and loading placeholders (an elided value like "https://…" is data).
  ['reticencias_placeholder', (p) => /placeholder/i.test(p.attr ?? '') && /…\s*$/.test(p.text) && !/^(Buscar|Pesquisar|Filtrar|Selecione|Selecionar|Escolha|Carregando)|https?:\/\/|^ex\.: \w+…$/i.test(p.text)],
  // A dialog title states the action, it does not ask (UI_WRITING §5).
  ['titulo_interrogativo', (p) => (/^(title|dialogTitle|confirmTitle)$/i.test(p.attr ?? '') || /DialogTitle$/.test(p.tag ?? '')) && /\?\s*$/.test(p.text)],
  ['label_dois_pontos', (p) => p.attr === 'label' && /:\s*$/.test(p.text)],
  ['alteracoes_nao_salvas', (p) => /alterações não salvas/i.test(p.text)],
]

const SKIP_DIRS = new Set(['node_modules', 'dist', 'build', 'mocks', '__tests__'])

/** Source files that carry copy: the TS/TSX of src/. */
function files(dir: string, acc: string[] = []): string[] {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name)
    if (entry.isDirectory()) {
      if (!SKIP_DIRS.has(entry.name)) files(full, acc)
      continue
    }
    if (/\.(test|spec)\.tsx?$|\.d\.ts$/.test(entry.name)) continue
    if (/\.tsx?$/.test(entry.name)) acc.push(full)
  }
  return acc
}

function collect(full_text = false): Violation[] {
  const out: Violation[] = []
  for (const full of files(SRC)) {
    const rel = path.relative(ROOT, full).split(path.sep).join('/')
    const src = fs.readFileSync(full, 'utf8')
    const pieces = lexJs(src)
    for (const p of pieces) {
      if (p.log || (!p.sep && !isCopy(p.text))) continue
      for (const [rule, test] of RULES) {
        if (test(p)) out.push({ file: rel, line: p.line, rule, text: full_text ? p.text : p.text.trim().slice(0, 200) })
      }
    }
  }
  return out
}

/** Line numbers move with every edit, so the baseline counts by file + rule + text. */
const keyOf = (v: Violation) => `${v.file}::${v.rule}::${v.text}`

function countByKey(list: Violation[]): Record<string, number> {
  const out: Record<string, number> = {}
  for (const v of list) out[keyOf(v)] = (out[keyOf(v)] ?? 0) + 1
  return out
}

function summary(list: Violation[]): string {
  const per: Record<string, number> = {}
  for (const v of list) per[v.rule] = (per[v.rule] ?? 0) + 1
  return Object.entries(per).sort((a, b) => b[1] - a[1]).map(([r, c]) => `${r} ${c}`).join(', ')
}

const args = process.argv.slice(2)
const violations = collect()

if (args.includes('--update')) {
  const sorted = Object.fromEntries(Object.entries(countByKey(violations)).sort(([a], [b]) => a.localeCompare(b)))
  fs.writeFileSync(BASELINE, JSON.stringify(sorted, null, 2) + '\n')
  console.log(`Baseline: ${violations.length} violações (${summary(violations)})`)
  process.exit(0)
}

if (args.includes('--dump')) {
  // Every copy string under the given path prefix (review aid for the rules that need judgment).
  const prefix = args[args.indexOf('--dump') + 1] ?? 'src/'
  for (const full of files(SRC)) {
    const rel = path.relative(ROOT, full).split(path.sep).join('/')
    if (!rel.startsWith(prefix)) continue
    const src = fs.readFileSync(full, 'utf8')
    for (const p of lexJs(src)) {
      if (!p.log && isCopy(p.text)) console.log(`${rel}:${p.line}\t${p.text.replace(/\s+/g, ' ').trim()}`)
    }
  }
  process.exit(0)
}

if (args.includes('--json')) {
  console.log(JSON.stringify(collect(true)))
  process.exit(0)
}

if (args.includes('--report')) {
  for (const v of violations) console.log(`${v.file}:${v.line}  [${v.rule}]  ${v.text}`)
  console.log(`\n${violations.length} violações (${summary(violations)})`)
  process.exit(0)
}

const baseline: Record<string, number> = fs.existsSync(BASELINE) ? JSON.parse(fs.readFileSync(BASELINE, 'utf8')) : {}
const seen: Record<string, number> = {}
const fresh: Violation[] = []
for (const v of violations) {
  const k = keyOf(v)
  seen[k] = (seen[k] ?? 0) + 1
  if (seen[k] > (baseline[k] ?? 0)) fresh.push(v)
}

const baselineTotal = Object.values(baseline).reduce((a, b) => a + b, 0)

if (fresh.length) {
  for (const v of fresh) console.log(`${v.file}:${v.line}  [${v.rule}]  ${v.text}`)
  console.log(`\n${fresh.length} violação(ões) nova(s) de copy. Guia: UI_WRITING.md do workspace DBMV`)
  process.exit(1)
}

const shrunk = baselineTotal - violations.length
console.log(`Copy OK: ${violations.length} na baseline${shrunk > 0 ? `, ${shrunk} a menos que ela (rode com --update para encolher)` : ''}.`)
