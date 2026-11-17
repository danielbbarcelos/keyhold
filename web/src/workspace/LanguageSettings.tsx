import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { t, useLang } from '@/i18n'
import { chooseLanguage, Flag, LANGUAGES } from '@/i18n/Language'

import { ThemeCard } from './Settings'

/** Personalizar, Idioma: the same choice as the toggle, as cards like the theme's. */
export function LanguageSettings({ onChanged }: { onChanged: () => void }) {
  const current = useLang()

  return (
    <Card className="py-6">
      <CardHeader className="px-6">
        <CardTitle>{t('Idioma')}</CardTitle>
        <CardDescription>
          {t('Aplica-se a este Keyhold, a todos os cofres e às mensagens do Keyhold. A página de um link compartilhado segue o idioma do navegador de quem abre.')}
        </CardDescription>
      </CardHeader>

      <CardContent className="space-y-2.5 px-6">
        <div className="flex flex-wrap gap-3">
          {LANGUAGES.map(({ lang, name }) => (
            <ThemeCard
              key={lang}
              label={name}
              chosen={current === lang}
              onPick={() => void chooseLanguage(lang).then(onChanged)}
            >
              <span lang={lang} className="grid h-full place-items-center bg-raised">
                <Flag lang={lang} className="h-[34px] w-12" />
              </span>
            </ThemeCard>
          ))}
        </div>
        <p className="text-sm text-muted-foreground">
          {t('Vale imediatamente. Nomes de registros, pastas e campos continuam como estão.')}
        </p>
      </CardContent>
    </Card>
  )
}
