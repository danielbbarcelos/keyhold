/** The shell around the workspace: opening a tab, setup, unlock, and the shared controls. */
export const APP: Record<string, string> = {
  // App.tsx
  'Conferindo o cofre…': 'Checking the vault…',
  'O Keyhold não está respondendo': "Keyhold isn't responding",
  'Esta aba perdeu a conexão com o Keyhold, que foi fechado ou reiniciado. A chave do cofre foi removida da memória, e os dados continuam no arquivo. Abra o Keyhold novamente e destranque o cofre.':
    'This tab lost its connection to Keyhold, which was closed or restarted. The vault key was removed from memory, and your data is still in the file. Open Keyhold again and unlock the vault.',
  'Abrir o Keyhold': 'Open Keyhold',
  'Tentar novamente': 'Try again',
  'O Keyhold é aberto em uma nova aba, e esta pode ser fechada. Caso nada aconteça, abra o Keyhold pelo menu de aplicativos.':
    'Keyhold opens in a new tab, and you can close this one. If nothing happens, open Keyhold from the applications menu.',
  'O Keyhold não está em execução. Abra pelo atalho ou pelo terminal.':
    "Keyhold isn't running. Open it from the shortcut or the terminal.",
  'Keyhold aberto em uma nova aba': 'Keyhold opened in a new tab',
  'Esta aba pode ser fechada. A sessão vai apenas para a nova aba, nunca para um endereço salvo.':
    'You can close this tab. The session goes only to the new tab, never to a saved address.',
  'O endereço que abriu esta aba já tinha sido usado antes. Outro programa pode ter lido o endereço, e a sessão criada com ele foi encerrada por segurança.':
    'The address that opened this tab had already been used. Another program may have read it, so the session it created was ended for safety.',
  'Esta aba não possui uma sessão do Keyhold. O Keyhold abre uma nova aba já com a sessão.':
    'This tab has no Keyhold session. Keyhold opens a new tab that already has one.',

  // screens/Setup.tsx
  Proteção: 'Protection',
  'Senha-mestra': 'Master password',
  'Chave de recuperação': 'Recovery key',
  'O Keyhold não respondeu. Tente novamente.': "Keyhold didn't respond. Try again.",
  'Escolha a proteção do cofre': 'Choose how to protect the vault',
  'As duas opções cifram o conteúdo do cofre. A diferença é onde fica a chave que o abre.':
    'Both options encrypt the vault. The difference is where the key that opens it is kept.',
  'A senha-mestra não é armazenada e não pode ser redefinida. Use uma senha longa e exclusiva, ou gere uma com o Keyhold.':
    "The master password isn't stored and can't be reset. Use a long, unique password, or let Keyhold generate one.",
  'Anote a chave de recuperação': 'Write down the recovery key',
  'É a única forma de abrir este cofre caso a senha-mestra seja perdida. Ela é exibida apenas uma vez, não é armazenada e não pode ser gerada novamente.':
    "It's the only way to open this vault if the master password is lost. It's shown only once, isn't stored, and can't be generated again.",
  'Com uma senha-mestra': 'With a master password',
  recomendado: 'recommended',
  'A chave que abre o cofre é derivada da sua senha e não existe em nenhum outro lugar. Uma cópia do arquivo do cofre não revela nada sem a senha.':
    'The key that opens the vault is derived from your password and exists nowhere else. A copy of the vault file reveals nothing without the password.',
  'Sem senha, protegido por esta máquina': 'No password, protected by this computer',
  'O cofre abre sem senha. A chave fica em um arquivo ao lado do cofre: quem conseguir ler seus arquivos (um backup, uma pasta sincronizada, outro programa) também abre o cofre.':
    'The vault opens without a password. The key is kept in a file next to the vault: anyone who can read your files (a backup, a synced folder, another program) can also open the vault.',
  'Senha gerada com {bits} de entropia.': 'Generated password with {bits} of entropy.',
  'Copie a senha para um lugar seguro antes de continuar. Ela não é exibida novamente.':
    "Copy the password somewhere safe before you continue. It won't be shown again.",
  'Sem a senha-mestra e sem a chave de recuperação, o conteúdo do cofre não pode ser recuperado por ninguém.':
    'Without the master password and the recovery key, nobody can recover what is in the vault.',
  'Digite a chave novamente para confirmar': 'Type the key again to confirm',
  Continuar: 'Continue',
  'Gerar uma senha forte': 'Generate a strong password',
  'Criar cofre': 'Create vault',
  'Confirmar e abrir o cofre': 'Confirm and open the vault',

  // screens/Unlock.tsx
  'Destrancar o Keyhold': 'Unlock Keyhold',
  'Digite a chave de recuperação anotada ao criar o cofre.':
    'Enter the recovery key you wrote down when you created the vault.',
  'Digite a senha-mestra para abrir os cofres.': 'Enter the master password to open your vaults.',
  'Este cofre não usa senha-mestra. Ele abre com a chave armazenada nesta máquina.':
    'This vault has no master password. It opens with the key stored on this computer.',
  'Código do app autenticador': 'Authenticator app code',
  'Resta 1 tentativa antes de o Keyhold parar de responder.':
    '1 attempt left before Keyhold stops responding.',
  'Restam {n} tentativas antes de o Keyhold parar de responder.':
    '{n} attempts left before Keyhold stops responding.',
  'Feche e abra o Keyhold novamente para tentar outra vez.': 'Close and reopen Keyhold to try again.',
  Destrancar: 'Unlock',
  'Usar a senha-mestra': 'Use the master password',
  'Perdi a senha-mestra': 'I lost the master password',

  // components/common
  'Sem data': 'No date',
  Data: 'Date',
  'Remover a data': 'Remove the date',
  'Escolher no calendário': 'Pick from the calendar',
  'Mês anterior': 'Previous month',
  'Próximo mês': 'Next month',
  Hoje: 'Today',
  'Em 3 meses': 'In 3 months',
  'Em 1 ano': 'In 1 year',
  'Selecione…': 'Select…',
  Buscar: 'Search',
  'Nada encontrado.': 'Nothing found.',
  Esconder: 'Hide',
  Mostrar: 'Show',

  // components/ui
  'Paleta de comandos': 'Command palette',
  'Buscar um comando': 'Search for a command',
}
