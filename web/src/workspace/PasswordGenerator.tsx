import { useCallback, useEffect, useState } from 'react'
import { Copy, Dices, Eye, EyeOff, ShieldAlert, ShieldCheck, ShieldHalf } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { cn } from '@/lib/utils'

import { api, DEFAULT_RECIPE, type Recipe } from '../api'
import { copyForSeconds } from '../clipboard'

const CLIPBOARD_SECONDS = 30
import { toast, toastError } from './toast'

/** Every symbol that can be picked, the same list the binary accepts. */
export const ALL_SYMBOLS = [...'!@#$%^&*()_+-=[]{}|;:,.<>?/\\~`\'"']
/** What "Símbolos" means until somebody picks: the binary's usual set. */
const USUAL_SYMBOLS = '!@#$%^&*()_+-=[]{}|;:,.<>?'

type Kind = 'senha' | 'pin'

const kindOf = (recipe: Recipe): Kind =>
  recipe.digits && !recipe.lowercase && !recipe.uppercase && !recipe.symbols ? 'pin' : 'senha'

/**
 * How hard a value is to guess, judged by what it is made of: the pool its characters come from,
 * to the power of its length. Honest for a generated value; generous for one a person typed,
 * which is why it is only ever a label and a bar, never a reason to refuse.
 */
export function strengthOf(value: string): { bits: number; label: string; tone: 'rose' | 'amber' | 'mint' } {
  let pool = 0
  if (/[a-z]/.test(value)) pool += 26
  if (/[A-Z]/.test(value)) pool += 26
  if (/[0-9]/.test(value)) pool += 10
  if (/[^a-zA-Z0-9]/.test(value)) pool += 32
  const bits = value.length * Math.log2(Math.max(pool, 1))

  if (bits >= 80) return { bits, label: 'Forte', tone: 'mint' }
  if (bits >= 50) return { bits, label: 'Razoável', tone: 'amber' }
  return { bits, label: 'Fraca', tone: 'rose' }
}

/** Type, length, the four kinds of character and, under "Mostrar mais", which symbols. */
export function GeneratorControls({ recipe, onChange }: { recipe: Recipe; onChange: (recipe: Recipe) => void }) {
  const [more, setMore] = useState(recipe.symbol_set !== null || recipe.avoid_ambiguous)
  const kind = kindOf(recipe)
  const chosen = recipe.symbol_set ?? USUAL_SYMBOLS
  const [min, max] = kind === 'pin' ? [4, 16] : [8, 64]

  function toggleSymbol(symbol: string) {
    const next = chosen.includes(symbol) ? chosen.replace(symbol, '') : chosen + symbol
    onChange({ ...recipe, symbol_set: next, symbols: next.length > 0 ? recipe.symbols : false })
  }

  return (
    <div className="grid gap-4">
      <label className="grid gap-1.5">
        <span className="text-[13px] font-medium">Tipo</span>
        <Select
          value={kind}
          onValueChange={(value) =>
            onChange(
              value === 'pin'
                ? { ...recipe, lowercase: false, uppercase: false, symbols: false, digits: true, length: 6 }
                : { ...DEFAULT_RECIPE, symbol_set: recipe.symbol_set },
            )
          }
        >
          <SelectTrigger className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="senha">Senha</SelectItem>
            <SelectItem value="pin">PIN (só números)</SelectItem>
          </SelectContent>
        </Select>
      </label>

      <label className="grid gap-2">
        <span className="text-[13px] font-medium">{recipe.length} caracteres</span>
        <input
          type="range"
          min={min}
          max={max}
          value={Math.min(Math.max(recipe.length, min), max)}
          onChange={(event) => onChange({ ...recipe, length: Number(event.target.value) })}
          className="w-full accent-[var(--primary)]"
        />
      </label>

      {kind === 'senha' && (
        <div className="flex flex-wrap gap-x-5 gap-y-2">
          {(
            [
              ['lowercase', 'Letras minúsculas'],
              ['uppercase', 'Maiúsculas'],
              ['digits', 'Números'],
              ['symbols', 'Símbolos'],
            ] as const
          ).map(([key, label]) => (
            <label key={key} className="flex items-center gap-2 text-[13px]">
              <Checkbox
                checked={recipe[key]}
                onCheckedChange={(on) => onChange({ ...recipe, [key]: on === true })}
              />
              {label}
            </label>
          ))}
        </div>
      )}

      {kind === 'senha' && more && (
        <div className="grid gap-3">
          <div className="flex flex-wrap gap-1.5">
            {ALL_SYMBOLS.map((symbol) => (
              <button
                key={symbol}
                type="button"
                aria-pressed={chosen.includes(symbol)}
                disabled={!recipe.symbols}
                onClick={() => toggleSymbol(symbol)}
                className={cn(
                  'grid size-8 place-items-center rounded-lg border font-mono text-[13px] disabled:opacity-40',
                  chosen.includes(symbol) ? 'border-transparent bg-primary text-primary-foreground' : 'text-muted-foreground hover:bg-hover',
                )}
              >
                {symbol}
              </button>
            ))}
          </div>
          <label className="flex items-center gap-2 text-[13px]">
            <Checkbox
              checked={recipe.avoid_ambiguous}
              onCheckedChange={(on) => onChange({ ...recipe, avoid_ambiguous: on === true })}
            />
            Evitar caracteres parecidos (I, l, 1, O, 0)
          </label>
        </div>
      )}

      {kind === 'senha' && (
        <button type="button" className="justify-self-end text-[13px] text-primary hover:underline" onClick={() => setMore(!more)}>
          {more ? 'Mostrar menos' : 'Mostrar mais'}
        </button>
      )}
    </div>
  )
}

/** The strength of a value, as a bar and a word. */
export function StrengthBar({ value }: { value: string }) {
  const strength = strengthOf(value)
  const Icon = strength.tone === 'mint' ? ShieldCheck : strength.tone === 'amber' ? ShieldHalf : ShieldAlert
  const width = Math.min(100, Math.round((strength.bits / 100) * 100))
  // Whole class names, so Tailwind finds them.
  const tone = { rose: ['bg-rose', 'text-rose'], amber: ['bg-amber', 'text-amber'], mint: ['bg-mint', 'text-mint'] }[strength.tone]

  return (
    <div className="grid gap-1.5">
      <div className="h-1 overflow-hidden rounded-full bg-foreground/10">
        <div className={cn('h-full rounded-full transition-[width]', tone[0])} style={{ width: `${width}%` }} />
      </div>
      <span className={cn('flex items-center gap-1.5 text-xs', tone[1])}>
        <Icon className="size-3.5" />
        {strength.label}
      </span>
    </div>
  )
}

/**
 * "Gerar senha": a new value from a recipe, editable by hand before it is used (to change one
 * character a site refuses, say). The recipe starts from the person's default, set in Ajustes or
 * here with "Salvar como padrão".
 */
export function PasswordGenerator({ onUse, onCancel }: { onUse: (value: string) => void; onCancel: () => void }) {
  const [recipe, setRecipe] = useState<Recipe | null>(null)
  const [value, setValue] = useState('')
  const [showing, setShowing] = useState(true)

  const make = useCallback((from: Recipe) => {
    void api
      .generate(from)
      .then((made) => setValue(made.password))
      .catch((cause) => toastError(cause, 'Não gerei a senha'))
  }, [])

  useEffect(() => {
    void api
      .status()
      .then((status) => setRecipe({ ...DEFAULT_RECIPE, ...(status.generator ?? {}) }))
      .catch(() => setRecipe(DEFAULT_RECIPE))
  }, [])

  useEffect(() => {
    if (recipe) make(recipe)
  }, [recipe, make])

  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="sm:max-w-[520px]">
        <DialogHeader>
          <DialogTitle>Gerador de senhas</DialogTitle>
          <DialogDescription>Ajuste o que a senha leva. O campo aceita edição à mão antes de usar.</DialogDescription>
        </DialogHeader>

        <div className="grid gap-5 py-2">
          <div className="grid gap-2">
            <div className="relative">
              <Input
                value={value}
                type={showing ? 'text' : 'password'}
                spellCheck={false}
                autoComplete="off"
                aria-label="Senha gerada"
                className="pr-[104px] font-mono text-[14px]"
                onChange={(event) => setValue(event.target.value)}
              />
              <span className="absolute top-1/2 right-1 flex -translate-y-1/2">
                <Button variant="ghost" size="icon-sm" aria-label={showing ? 'Esconder' : 'Mostrar'} onClick={() => setShowing(!showing)}>
                  {showing ? <EyeOff /> : <Eye />}
                </Button>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label="Copiar"
                  onClick={() =>
                    void copyForSeconds(value, CLIPBOARD_SECONDS, () => undefined)
                      .then(() => toast('Senha copiada', 'A área de transferência é limpa em 30 segundos.'))
                      .catch(() => toast('Não consegui copiar', 'O navegador não liberou a área de transferência.', 'error'))
                  }
                >
                  <Copy />
                </Button>
                <Button variant="ghost" size="icon-sm" aria-label="Gerar outra" onClick={() => recipe && make(recipe)}>
                  <Dices />
                </Button>
              </span>
            </div>
            <StrengthBar value={value} />
          </div>

          {recipe && <GeneratorControls recipe={recipe} onChange={setRecipe} />}
        </div>

        <DialogFooter className="items-center sm:justify-between">
          <button
            type="button"
            className="text-[13px] text-muted-foreground hover:text-foreground hover:underline"
            disabled={!recipe}
            onClick={() =>
              recipe &&
              void api
                .saveGenerator(recipe)
                .then(() => toast('Padrão salvo', 'O gerador vai começar sempre por aqui.'))
                .catch((cause) => toastError(cause))
            }
          >
            Salvar como padrão
          </button>
          <span className="flex gap-2">
            <Button variant="outline" onClick={onCancel}>
              Cancelar
            </Button>
            <Button disabled={!value} onClick={() => onUse(value)}>
              Usar senha
            </Button>
          </span>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
