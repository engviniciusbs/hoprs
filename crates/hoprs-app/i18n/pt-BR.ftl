# Texto da interface do hoprs, português do Brasil. As chaves e as variáveis são as de
# en.ftl; só o texto muda. Veja o cabeçalho de en.ftl para o formato.

app-title = hoprs
already-running = O hoprs já está em execução
cannot-open-profiles = Não foi possível abrir os sites salvos

# Um número de sites salvos: "1 site", "3 sites".
site-count =
    { $count ->
        [0] { $count } sites
        [one] { $count } site
       *[other] { $count } sites
    }

# A página de configurações: o idioma da interface e as opções. Os nomes dos idiomas são
# escritos no próprio idioma, então são iguais em todos os catálogos.
settings-language = Idioma
language-system = Sistema
language-english = English
language-pt-br = Português (Brasil)
language-tr = Türkçe

## Top bar

brand = hoprs
no-sessions = Nenhuma sessão aberta
go-to-anything = Ir para qualquer coisa
go-to-anything-key = Ctrl K
sidebar-options = Opções da barra lateral
menu-show-workspaces = Mostrar workspaces
menu-show-sites = Mostrar todos os sites
menu-open = Abrir
menu-copy-path = Copiar caminho
menu-refresh = Atualizar
menu-show-in-panes = Mostrar nos painéis
menu-open-files = Abrir arquivos
menu-close-other-tabs = Fechar outras abas
key-refresh = F5

## The Ctrl+K palette

palette-placeholder = Ir para uma aba, site ou workspace, ou executar uma ação
palette-workspaces-placeholder = Abrir workspace
palette-no-results = Nenhum resultado
palette-group-action = Ação
palette-group-tab = Aba
palette-group-site = Site
palette-group-workspace = Workspace
palette-kind-terminal = terminal
palette-kind-files = arquivos
palette-toggle-queue = Mostrar ou ocultar a fila
palette-toggle-log = Mostrar ou ocultar o log
palette-close-tab = Fechar aba
palette-close-settings = Fechar configurações
palette-open-workspace = Abrir workspace…
palette-close-all = Fechar todos em { $name }
update-palette = Atualizar o hoprs para { $version }

# O card de atualização (canto inferior esquerdo), a linha do rodapé e Configurações → Geral.
# $version é uma versão (ex.: 1.4.0).
update-available = hoprs { $version } está disponível
# $version é a versão em uso.
update-you-have = Sua versão é a { $version }.
update-action = Atualizar
update-whats-new = Novidades
update-dismiss = Dispensar
# $done e $total são tamanhos (ex.: 12,4 MB).
update-downloading = Baixando… { $done } de { $total }
update-downloading-size = Baixando… { $done }
update-verifying = Verificando…
update-try-again = Tentar de novo
update-ready-windows = Pronto para instalar. O hoprs vai fechar.
update-transfers-stop =
    { $count ->
        [0] { $count } transferências vão parar; elas continuam na próxima vez que o hoprs abrir.
        [one] 1 transferência vai parar; ela continua na próxima vez que o hoprs abrir.
       *[other] { $count } transferências vão parar; elas continuam na próxima vez que o hoprs abrir.
    }
update-close-and-install = Fechar e instalar
update-later = Depois
update-installing = Abrindo o instalador…
update-linux-opened = Aberto no app padrão para arquivos .deb. Ou execute:
update-linux-run = Execute este comando para instalar:
update-open-failed = O instalador não abriu.
update-installer-open = O instalador está aberto. Feche o hoprs para concluir.
update-close-hoprs = Fechar o hoprs
update-copy = Copiar
update-macos-steps = Feche a janela do hoprs, depois arraste o hoprs para Aplicativos e escolha Substituir. Se o macOS bloquear, escolha Abrir Mesmo Assim em Ajustes do Sistema, Privacidade e Segurança.
update-footer-available = { $version } disponível
update-footer-install = Instalar { $version }
settings-update-check = Procurar atualizações automaticamente
settings-update-check-hint = Uma vez por dia o hoprs pergunta ao GitHub qual é a versão mais recente. Nada sobre você é enviado.
settings-check-now = Procurar agora
settings-update-checking = Procurando…
settings-up-to-date = O hoprs está atualizado
palette-edit = Abrir no editor
palette-compare-folders = Comparar pastas
palette-upload = Fazer upload da seleção
palette-download = Fazer download da seleção
palette-copy = Copiar
palette-paste = Colar
palette-find = Buscar no histórico
palette-key-copy = Ctrl Shift C
palette-key-paste = Ctrl Shift V
palette-key-find = Ctrl Shift F

## Sidebar

filter-sites = Filtrar sites
workspaces = Workspaces
all-sites = Todos os sites
sidebar-all-hidden = As duas listas estão ocultas. Mostre-as no menu ⋯.
sidebar-collapse = Recolher
sidebar-expand = Expandir
no-workspaces = Nenhum workspace ainda
no-sites = Nenhum site ainda
import-filezilla = Importar do FileZilla
settings = Configurações
key-settings = Ctrl ,
key-settings-macos = ⌘ ,
new-site = Novo site
no-matches = Nenhum site encontrado
menu-open-all = Abrir todos
menu-close-all = Fechar todos
workspace-open-tip = Workspace aberto
edit = Editar
site-missing = Site ausente
cannot-load-sites = Não foi possível carregar os sites salvos
retry = Tentar novamente
unreadable-entry = Não foi possível ler esta entrada (escrita por uma versão mais recente do hoprs?)
open-profile-folder = Abrir a pasta do sites.toml
cannot-open-folder = Não foi possível abrir a pasta: { $reason }
workspace-site-count-partial =
    { $total ->
        [one] { $present } de { $total } site
       *[other] { $present } de { $total } sites
    }
unreadable-name-fallback = Entrada { $number } em { $file }

## Site form

site-form-new = Novo site
site-form-edit = Editar site
field-name = Nome
field-name-hint = Usa o host por padrão
field-host = Host
field-port = Porta
field-user = Usuário
field-protocol = Protocolo
field-password = Senha
password-optional = Opcional
password-replace = Digite para substituir
password-saved = Salva no keyring do sistema
forget-password = Esquecer a senha salva
ssh-options = SSH
use-agent = Usar ssh-agent
key-file = Arquivo de chave
key-file-hint = Caminho de uma chave privada (opcional)
open-as = Abrir como
open-as-terminal = Terminal
open-as-files = Arquivos
open-as-both = Terminal e arquivos
ftp-options = FTP
ftp-mode = Modo de transferência
ftp-passive = Passivo
ftp-active = Ativo
nat-workaround = Ignorar o endereço passivo enviado pelo servidor (NAT)
start-folders = Pastas iniciais
remote-dir = Pasta remota
local-dir = Pasta local
folder-hint = Opcional
host-required = O host é obrigatório
port-invalid = A porta deve ser um número de 1 a 65535
save = Salvar
saving = Salvando…
cancel = Cancelar
delete = Excluir
keep = Manter
delete-site-confirm = Excluir este site? Ele será removido de todos os workspaces e a senha salva será excluída.

## Workspace form

workspace-form-new = Novo workspace
workspace-form-edit = Editar workspace
workspace-sites = Sites
workspace-no-sites = Nenhum site ainda. Crie um site primeiro.
name-required = O nome é obrigatório
delete-workspace-confirm = Excluir este workspace? Os sites dele são mantidos.

## Center

no-session-title = Nenhuma sessão aberta
no-session-hint = Escolha um site na barra lateral para conectar.
keyring-unavailable = O keyring do sistema estava indisponível, então a senha salva não foi usada.
edit-site = Editar site

## Tabs

close-tab = Fechar aba
new-terminal-tab = Nova aba de terminal
new-files-tab = Nova aba de arquivos
terminal-overflowed = A saída chegou rápido demais e o shell foi encerrado
terminal-reopen = Reabrir
terminal-restart = Reiniciar o shell
terminal-shell-closed = Shell fechado
terminal-open-failed = Não foi possível abrir um shell
terminal-retry = Tentar novamente
terminal-waiting-for-connection = Aguardando a conexão
terminal-input-dropped = Parte da entrada foi descartada: o shell não a processou rápido o bastante
terminal-reconnected = — reconectado —
terminal-search-placeholder = Buscar no histórico
terminal-search-next = Próximo
terminal-search-previous = Anterior
terminal-search-close = Fechar
terminal-search-no-match = Nenhum resultado
shell-exited-code = Shell encerrado (código { $code })
shell-exited = Shell encerrado

## File panes
local = Local
col-name = Nome
col-size = Tamanho
col-modified = Modificado
col-permissions = Permissões
sort-ascending = ↑
sort-descending = ↓
parent-dir = ..
path-separator = /
home-dir = ~
no-size = -
empty-folder = Esta pasta está vazia
# A drag between panes, at the bottom of the pane it would drop on: "Upload 2 items to
# /srv", "Download 1 item to ~/Downloads" (the folder in mono, cut with "…").
drop-upload =
    { $count ->
        [0] Fazer upload de { $count } itens para { $folder }
        [one] Fazer upload de { $count } item para { $folder }
       *[other] Fazer upload de { $count } itens para { $folder }
    }
drop-download =
    { $count ->
        [0] Fazer download de { $count } itens para { $folder }
        [one] Fazer download de { $count } item para { $folder }
       *[other] Fazer download de { $count } itens para { $folder }
    }
# Files from the system file manager over the remote pane; "-unknown" while the system
# has not said how many.
drop-upload-files =
    { $count ->
        [0] Fazer upload de { $count } arquivos para { $folder }
        [one] Fazer upload de { $count } arquivo para { $folder }
       *[other] Fazer upload de { $count } arquivos para { $folder }
    }
drop-upload-files-unknown = Fazer upload de arquivos para { $folder }
# The chip that follows the pointer in a drag of several items: "+1" after the first
# name, then the badge with how many go ("2").
drag-more = +{ $more }
drag-count = { $count }
reconnect = Reconectar
size-unit-b = B
size-unit-kb = KB
size-unit-mb = MB
size-unit-gb = GB
size-unit-tb = TB
size-unit-pb = PB
size-unit-eb = EB
month-1 = jan
month-2 = fev
month-3 = mar
month-4 = abr
month-5 = mai
month-6 = jun
month-7 = jul
month-8 = ago
month-9 = set
month-10 = out
month-11 = nov
month-12 = dez

## Queue
transfers = Transferências
queue-active = Ativas
queue-queued = Na fila
queue-failed = Com falha
queue-done = Concluídas
queue-hide = Ocultar
queue-show = Mostrar
no-transfers = Nenhuma transferência
no-transfers-hint = Selecione arquivos e pressione Upload ou Download, arraste-os para o outro painel ou solte-os aqui.
queue-loading = Carregando transferências…
nothing-active = Nada em andamento
nothing-queued = Nada aguardando
nothing-failed = Nenhuma falha
nothing-done = Nada concluído ainda
clear-finished = Limpar concluídas e com falha

## Technical log panel
log = Log
log-error = Erro
log-warn = Aviso
log-info = Info
log-debug = Debug
filter-log = Filtrar o log
copy-shown = Copiar exibidas
clear = Limpar
clear-hint = Esvazia só este painel; novas linhas continuam chegando.
log-level-hint = Mostra linhas com pelo menos esta gravidade. Só info e acima são registradas, a menos que o hoprs seja iniciado com RUST_LOG (por exemplo, RUST_LOG=debug).
no-log-lines = Nada registrado ainda
no-log-matches = Nenhuma linha corresponde
log-lines =
    { $count ->
        [0] { $count } linhas
        [one] { $count } linha
       *[other] { $count } linhas
    }
waiting-for-worker = Aguardando um worker
starting = Iniciando…
transferring = Transferindo…
dismissed = Dispensada
paused = Pausada
cancelled = Cancelada
transfer-done = Concluída
file-exists = O arquivo existe —
overwrite = Sobrescrever
resume = Retomar
skip = Pular
pause = Pausar
apply-to-all = Aplicar a todos (nesta sessão)
saved-at = salvo às { $time }
site-paused = Pausada — conexão perdida
total-workers =
    { $workers ->
        [0] no total, { $workers } workers
        [one] no total, { $workers } worker
       *[other] no total, { $workers } workers
    }
policy-ask = perguntar
policy-overwrite = sobrescrever
policy-skip = pular
policy-overwrite-if-newer = sobrescrever se mais recente
policy-resume = retomar
session-policy = Conflitos: { $name } (nesta sessão)
policy-if-newer = se mais recente
session-policy-tag = { $name }: sessão
queue-unavailable = Fila de transferências indisponível: { $reason }. As transferências desta sessão não sobreviverão a uma reinicialização.
skip-exists = já existe
skip-not-newer = não é mais recente
skip-already-complete = já está completo
skip-by-user = pulada por você
skip-symlink = links simbólicos não são seguidos
skipped = Ignorada: { $reason }

## Status bar
not-connected = Não conectado

## Status bar and connection states
connecting = Conectando…
connected = Conectado
waiting-for-you = Aguardando você
closing = Salvando as abas e fechando…
reconnecting-now = Reconectando…
saved-folder-gone = A pasta salva { $saved } não existe mais; mostrando a pasta mais próxima.
reconnecting = Reconectando ({ $attempt }/{ $of })
disconnected = Desconectado — { $reason }
auth-none = sem autenticação
auth-agent = { $key_type } via ssh-agent
auth-key-file = chave { $key_type }
auth-password = senha
auth-keyboard-interactive = keyboard-interactive
ftp-auth-ftp = FTP (sem criptografia)
ftp-auth-ftps-explicit = FTPS explícito
ftp-auth-ftps-implicit = FTPS implícito
workspace-status = { $connected } de { $sites } em { $name }

## File actions
new-folder = Nova pasta
rename = Renomear
permissions = Permissões…
key-new-folder = F7
key-rename = F2
key-delete = Del
key-edit = F4
upload = Upload
download = Download
key-transfer = Ctrl ↵
new-folder-name = Nova pasta
name-empty = O nome não pode ficar vazio
name-invalid = O nome não pode conter / nem ser . ou ..
name-invalid-windows = O nome não pode conter \ neste disco
name-spaces = O nome não pode começar nem terminar com espaço
mixed-permissions = Permissões mistas: mostrando as do primeiro item
name-taken = Já existe algo com este nome aqui
dismiss = Dispensar
creating-folder = Criando pasta…
renaming = Renomeando…
changing-permissions = Alterando permissões…
delete-folders-note = As pastas são excluídas com tudo que há dentro.
permissions-title = Permissões
perm-owner = Dono
perm-group = Grupo
perm-others = Outros
perm-read = Leitura
perm-write = Escrita
perm-execute = Execução
perm-octal = Octal
octal-invalid = Três ou quatro dígitos de 0 a 7
apply = Aplicar
delete-question-one = Excluir { $name }?
delete-question-many = Excluir { $count } itens?
and-more = e mais { $count }
deleting =
    { $done ->
        [0] Excluindo…
        [1] Excluindo… 1 item
       *[other] Excluindo… { $done } itens
    }
deleted-then =
    { $done ->
        [1] 1 item excluído; depois: { $error }
       *[other] { $done } itens excluídos; depois: { $error }
    }

## Security and credential dialogs
host-key-title = Chave de host desconhecida
host-key-intro = O hoprs nunca se conectou a este servidor. Confira a impressão digital com o administrador antes de confiar nele.
trust-and-connect = Confiar e conectar
cert-untrusted-title = Certificado não confiável
cert-untrusted-intro = Não foi possível verificar o certificado do servidor. Confira a impressão digital com o administrador antes de confiar nele.
cert-changed-title = Certificado alterado
cert-changed-intro = O certificado do servidor não é o mesmo em que você confiou antes. Alguém pode estar interceptando a conexão.
trust-certificate = Confiar neste certificado
label-host = Host
label-account = Conta
label-key-type = Tipo de chave
label-fingerprint = Impressão digital (SHA-256)
label-reason = Motivo
label-trusted-before = Confiado anteriormente
label-presented-now = Apresentado agora
label-key-file = Chave
password-title = Senha
passphrase-title = Passphrase da chave
field-passphrase = Passphrase
kbd-title = Autenticação
save-in-keyring = Salvar no keyring do sistema
save-in-keyring-hint = Salvo somente depois que a conexão for bem-sucedida
connect = Conectar
unlock = Desbloquear
continue = Continuar
# Under a dialog, when others wait behind it: "1 more waiting".
dialogs-waiting = Mais { $count } na fila
# The warning of a host known with other key types.
other-key-types = Este host é conhecido com outro tipo de chave ({ $types }). Alguém pode estar se passando por ele.
# Why a certificate needs the user's decision.
cert-reason-expired = Expirado
cert-reason-not-yet-valid = Ainda não é válido
cert-reason-wrong-name = Emitido para outro nome
cert-reason-unknown-issuer = Autoridade desconhecida (autoassinado ou emissor não confiável)

## Changed host key, in the tab
host-key-changed-title = A chave do host mudou
host-key-changed-body = O servidor apresentou uma chave diferente da registrada para ele. Isso acontece quando o servidor foi reinstalado ou quando alguém intercepta a conexão. O hoprs não vai se conectar enquanto a entrada antiga estiver no known_hosts.
host-key-changed-session-body = O servidor apresentou uma chave diferente da que esta sessão verificou ao se conectar. Isso acontece quando o servidor foi reinstalado ou quando alguém intercepta a conexão. Uma nova sessão confere a chave com o known_hosts novamente.
remove-old-entry = Remover a entrada antiga…
close-and-reconnect = Fechar e reconectar
reading-known-hosts = Lendo o known_hosts…
removing-known-hosts = Removendo…
remove = Remover
connect-again = Conectar de novo
no-known-hosts-entry = O known_hosts não tem mais nenhuma entrada para este host. Conecte de novo para conferir a nova chave.
# "known_hosts line 3".
known-hosts-line = Linha { $line } do known_hosts
# The removal question: "Remove 2 lines for [h]:2222 from known_hosts? …".
remove-known-hosts-question =
    { $lines ->
    [1] Remover 1 linha de { $host } do known_hosts? Na próxima conexão, o hoprs pede que você confie na nova chave.
   *[other] Remover { $lines_text } linhas de { $host } do known_hosts? Na próxima conexão, o hoprs pede que você confie na nova chave.
    }
# The warning for lines that also name other hosts.
also-covers-one-host = A linha { $line } também cobre outros hosts (um padrão ou uma lista de nomes): removê-la esquece esta chave para eles também.
also-covers-other-hosts = As linhas { $lines } também cobrem outros hosts (padrões ou listas de nomes): removê-las esquece estas chaves para eles também.
# A connected tab whose site's host or port changed (an edit, a restore).

## Import from FileZilla
import-title = Importar do FileZilla
import-step-file = 1  Arquivo
import-step-folders = 2  Pastas
import-step-report = 3  Relatório
import-file-label = Arquivo do gerenciador de sites do FileZilla
import-file-hint = Normalmente o sitemanager.xml, na pasta de configuração do FileZilla. As senhas vão para o keyring do sistema.
import-path-required = Escolha o arquivo sitemanager.xml
choose-file = Escolher…
next = Avançar
back = Voltar
import-folders-question = Transformar as pastas do FileZilla em workspaces?
import-folders-hint = Sim: um workspace por pasta, com o caminho dela como nome e os sites da pasta. Não: o caminho da pasta vai antes do nome de cada site ("Work/Alpha").
import-folders-yes = Sim, criar workspaces
import-folders-no = Não, manter nos nomes
import-running = Importando…
import-imported = Importados
import-skipped = Ignorados
import-workspaces-created = Workspaces criados
import-workspaces-not-created = Workspaces não criados
import-failures = Falhas
import-none = Nenhum
import-failed-keep-source = Algumas entradas não puderam ser salvas: mantenha o sitemanager.xml e importe de novo depois de corrigir.
import-passwords-keep-source = O keyring recusou as senhas destes sites: mantenha o sitemanager.xml, ele ainda as tem.
delete-sitemanager = Excluir o sitemanager.xml
delete-sitemanager-confirm = Excluir o sitemanager.xml do FileZilla? O FileZilla não terá mais estes sites.
deleting-sitemanager = Excluindo…
sitemanager-deleted = sitemanager.xml excluído
done = Concluído
pick-sitemanager = Escolha o sitemanager.xml do FileZilla
pick-filter-xml = Arquivos XML

## Settings
settings-general = Geral
settings-general-description = Idioma, atualizações e versão.
settings-transfers-description = Quantos arquivos andam ao mesmo tempo, o que fazer com arquivos que já existem e como manter as conexões abertas.
settings-compare-description = Quando as pastas são comparadas e quais nomes ficam de fora.
settings-editing-description = O editor que abre os arquivos remotos.
settings-terminal-description = A aparência das abas de terminal e quanto elas lembram.
settings-backup-description = Salve seus sites, workspaces e senhas em um arquivo, ou restaure-os.
settings-transfers = Transferências
settings-workers = Workers por site
settings-workers-hint = Transferências paralelas por site, de 1 a 16. Vale para as próximas conexões.
settings-policy = Quando um arquivo já existe
settings-policy-hint = Vale para as próximas transferências. Perguntar mostra a escolha na fila.
settings-keepalive = Keepalive
settings-keepalive-hint = Com que frequência uma conexão ociosa é verificada, para que servidores e roteadores a mantenham aberta. Vale para as próximas conexões.
# A keepalive interval: "30 s".
seconds = { $secs } s
settings-unreadable = Não foi possível ler o settings.toml, então valem os padrões. As alterações duram até o hoprs fechar e não são salvas, para manter o arquivo como está: corrija ou exclua o arquivo para salvar as configurações.
settings-save-failed = Não foi possível salvar o settings.toml
settings-terminal = Terminal
settings-font-size = Tamanho da fonte
settings-scrollback = Linhas de histórico
settings-scrollback-hint = Vale para os terminais abertos daqui em diante.
settings-theme = Tema
theme-hoprs-dark = hoprs escuro
settings-policy-ask = Perguntar
settings-policy-overwrite = Sobrescrever
settings-policy-skip = Pular
settings-policy-overwrite-if-newer = Sobrescrever se mais recente
settings-policy-resume = Retomar
decrease = −
increase = +

## Backup
backup = Backup
backup-hint = Um arquivo criptografado com todos os sites, workspaces e senhas salvas. Guarde a senha: sem ela o backup não pode ser aberto.
export-backup = Exportar backup…
restore-backup = Restaurar backup…
backup-file = Arquivo
backup-password = Senha
backup-password-confirm = Senha de novo
backup-path-required = Escolha um arquivo
backup-password-required = Digite uma senha
backup-passwords-differ = As senhas não coincidem
export = Exportar
export-overwrite-confirm = Este arquivo já existe. Sobrescrevê-lo com o novo backup?
restore = Restaurar
exporting = Criptografando o backup… isso leva alguns segundos
restoring = Descriptografando o backup… isso leva alguns segundos
restore-mode = Modo
restore-merge = Mesclar
restore-merge-hint = Adiciona o que é novo; os sites e workspaces que você tem continuam.
restore-replace = Substituir
restore-replace-hint = Tudo passa a ser o do backup: sites, workspaces e senhas que não estão nele são removidos.
restore-replace-confirm = Substituir todos os sites, workspaces e senhas salvas pelos do backup?
replace = Substituir
export-failed = O backup não foi gravado
restore-failed = Nada foi restaurado
backup-secret-errors = Senhas não restauradas
backup-missing-secrets = Salvos sem a senha (ausente do keyring)
backup-skipped-unreadable = Não exportadas (entradas ilegíveis)
backup-skipped-sites = Sites que já existem (mantidos)
backup-skipped-workspaces = Workspaces que já existem (mantidos)
export-ellipsis = Exportar…
export-everything = Exportar tudo…
export-title-site = Exportar "{ $name }" (site)
export-title-workspace = Exportar "{ $name }" (workspace, { $sites })
export-title-all = Exportar tudo ({ $sites }, { $workspaces })
export-password-hint = Guarde esta senha: sem ela o arquivo não pode ser aberto.
export-partial-note = Este arquivo terá só uma parte dos seus sites. Restaure com Mesclar; o hoprs 1.5.0 e anteriores não conseguem abri-lo.
pick-export = Salvar o backup do hoprs
pick-restore = Escolher um backup do hoprs
# "Exported 12 sites, 3 workspaces and 9 passwords."
export-done = Exportados { $sites }, { $workspaces } e { $secrets }.
workspace-count =
    { $count ->
    [0] { $count } workspaces
    [one] { $count } workspace
   *[other] { $count } workspaces
    }
password-count =
    { $count ->
    [0] { $count } senhas
    [one] { $count } senha
   *[other] { $count } senhas
    }
# "Restored 2 sites, 1 workspace and 2 passwords."
restore-done = Restaurados { $sites }, { $workspaces } e { $secrets }.

## Remote editing
# The Edit action (remote pane).
edit-file = Editar
edit-no-folders = Não é possível editar pastas
edit-no-links = Não é possível editar links
edit-too-large = Não é possível editar arquivos com mais de 50 MB
waiting-for-connection = Aguardando a conexão…
remote-may-be-truncated = A cópia do servidor pode ficar incompleta até lá.
changed-on-server = Alterado no servidor desde que você o abriu
keep-remote = Manter a versão remota
save-a-copy = Salvar uma cópia
reload-in-editor = Agora é a versão do servidor: recarregue-a no editor.
upload-and-close = Fazer upload das alterações pendentes e fechar
close-keep-copies = Fechar e manter as cópias locais
upload-and-quit = Fazer upload e sair
quit-keep-copies = Sair e manter as cópias locais
checking-edits = Verificando as edições remotas…
stop-keep-copies = Parar e manter as cópias locais
# While "Upload" runs (an upload under way has no bound but its protocol's timeouts).
uploading-edits = Fazendo upload… (Parar para manter as cópias locais)
# The kept notice, before a remote path: "Server copy of".
server-copy-of = Cópia do servidor de
# …after it.
may-be-incomplete = pode estar incompleta
# The kept notice after a close that did not ask (the site moved, "Close and
# reconnect"): the editor's file is no longer linked to the server.
kept-saves-stopped = A sessão foi fechada: as próximas gravações no editor não serão enviadas. Suas alterações estão na cópia recuperada; abra o arquivo de novo para continuar editando.
edit-one-file = Selecione um arquivo para editar
uploaded = Enviado
editing = Editando
opening = Abrindo
close-workspace-edits-title = Algumas edições remotas deste workspace não foram enviadas
keeping-copies = Salvando as cópias locais…
close-edits-title = Algumas edições remotas não foram enviadas
quit-edits-title = Algumas edições remotas não foram enviadas
quit-edits-hint = A janela continua aberta até você escolher. Nada é excluído.
edit-unsent = não enviado
edit-conflict = alterado no servidor
editor-command = Editor
# Windows' example names `code.cmd`: the command runs without a shell, which does not
# find `.cmd` shims from a bare name.
editor-command-hint-windows = system abre os arquivos remotos no editor padrão do sistema. Ou digite um comando; o caminho do arquivo é adicionado no fim. Por exemplo: notepad++.exe, code.cmd --wait.
editor-command-hint-macos = system abre os arquivos remotos no editor padrão do sistema. Ou digite um comando; o caminho do arquivo é adicionado no fim. Por exemplo: open -W -a "Visual Studio Code" (um caminho completo também funciona).
editor-command-hint = system abre os arquivos remotos no editor padrão do sistema. Ou digite um comando; o caminho do arquivo é adicionado no fim. Por exemplo: gedit, code --wait.
# Next to the editor field when it holds `system`.
editor-system-default = Padrão do sistema
editor-command-empty = Digite um comando
settings-editing = Edição remota
settings-compare = Comparação de pastas
settings-compare-auto = Comparar pastas automaticamente
settings-compare-ignore = Ignorar ao comparar
settings-compare-ignore-hint = Nomes separados por vírgula. * corresponde a qualquer sequência de caracteres, por exemplo *.log. Um campo vazio não ignora nada.
# "Upload failed: Permission denied".
upload-failed = Falha no upload: { $reason }
# "Your version was saved to <path>".
your-version-at = Sua versão foi salva em { $path }
# Where "keep" puts the copies: "Unsent changes are copied to <dir>".
copies-go-to = As alterações não enviadas são copiadas para { $dir }
# "2 files could not be uploaded".
not-uploaded =
    { $count ->
    [0] Não foi possível fazer upload de { $count } arquivos
    [one] Não foi possível fazer upload de { $count } arquivo
   *[other] Não foi possível fazer upload de { $count } arquivos
    }
# The persistent notice after a close kept unsent edits: "Kept 2 unsent edits as
# recovered copies in" (the folder follows, in mono).
kept-notice =
    { $count ->
    [0] Mantidas { $count } edições não enviadas como cópias recuperadas em
    [one] Mantida { $count } edição não enviada como cópia recuperada em
   *[other] Mantidas { $count } edições não enviadas como cópias recuperadas em
    }
# The delete confirmation of a site with files open for editing.
delete-site-edits =
    { $count ->
    [0] { $count } arquivos deste site estão abertos para edição: as alterações não enviadas serão mantidas como cópias recuperadas, nunca excluídas.
    [one] { $count } arquivo deste site está aberto para edição: as alterações não enviadas serão mantidas como cópia recuperada, nunca excluídas.
   *[other] { $count } arquivos deste site estão abertos para edição: as alterações não enviadas serão mantidas como cópias recuperadas, nunca excluídas.
    }

## Errors — SSH and SFTP

error-auth-cancelled = Cancelado pelo usuário
error-sftp-subsystem-refused = O servidor recusou o subsistema SFTP
error-sftp-start-failed = Não foi possível iniciar o SFTP
error-sftp-connection-lost = Conexão SFTP perdida: { $path }
error-sftp-operation-failed = Falha na operação SFTP: { $path }
error-sftp-cannot-resume-size = Não foi possível retomar em { $offset }: { $path } tem apenas { $size } bytes
error-sftp-cannot-resume-missing = Não foi possível retomar um arquivo inexistente: { $path }
error-sftp-path-too-long = Caminho longo demais: { $path }
error-sftp-mtime-out-of-range = Data de modificação fora do intervalo aceito pelo SFTP: { $path }
error-auth-failed = Falha na autenticação
error-ssh-kbd-wrong-answer-count = Número incorreto de respostas keyboard-interactive
error-ssh-kbd-too-many-rounds = Etapas demais na autenticação keyboard-interactive
error-ssh-too-many-channels = Canais demais nesta conexão (limite: { $limit })
error-known-hosts-changed = O known_hosts mudou; revise as entradas novamente
error-known-hosts-invalid-host = Nome de host inválido para o known_hosts
error-ssh-protocol-error = Erro de protocolo SSH
error-host-key-changed = A chave do host { $host } mudou
error-host-key-revoked = A chave do host { $host } foi revogada
error-host-key-certificate-unsupported = Certificados de host não são suportados
error-ssh-shell-closed = Shell encerrado
error-ssh-pty-refused = O servidor recusou a requisição de PTY/shell
error-ssh-closed-during-request = Conexão encerrada durante a requisição de { $what }
error-ssh-shell-request-timed-out = Tempo esgotado na requisição do shell

## Errors — FTP and TLS

error-ftp-login-failed = Falha no login: { $context }
error-ftp-server-closed = O servidor encerrou a conexão: { $context }
error-ftp-cannot-open-data-connection = Não foi possível abrir a conexão de dados: { $context }
error-ftp-transfer-aborted = Transferência interrompida: { $context }
error-ftp-reply-no-such-file = Arquivo ou pasta inexistente: { $context }
error-ftp-reply-permission-denied = Permissão negada: { $context }
error-ftp-file-unavailable = Arquivo indisponível: { $context }
error-ftp-server-failed-file = O servidor não conseguiu processar o arquivo: { $context }
error-ftp-storage-full = Armazenamento do servidor cheio ou cota excedida: { $context }
error-ftp-name-not-allowed = Nome de arquivo não permitido: { $context }
error-ftp-unexpected-reply = Resposta FTP inesperada: { $context }
error-ftp-connection-lost = Conexão perdida: { $context }
error-tls-failed = Falha de TLS: { $context }
error-ftp-unexpected-reply-path = Resposta FTP inesperada: { $path }
error-ftp-name-line-break = O FTP não aceita um nome com quebra de linha: { $name }
error-ftp-listing-format = Não foi possível ler o formato da listagem do servidor
error-ftp-connection-lost-path = Conexão perdida: { $path }
error-ftp-offset-out-of-range = Posição { $offset } fora do intervalo: { $path }
error-ftp-cannot-continue = Não foi possível continuar { $path } no byte { $offset }
error-ftp-cannot-continue-missing = Não foi possível continuar um arquivo inexistente: { $path }
error-ftp-mtime-out-of-range = Data de modificação fora do intervalo aceito pelo FTP: { $context }
error-tls-certificate-not-trusted = Certificado não confiável
error-ftp-ftps-refused = O servidor recusou FTPS: { $target }
error-ftp-tls-config-missing = O FTPS precisa de uma configuração de TLS
error-ftp-control-command-timed-out = Tempo esgotado no comando de controle
error-ftp-command-timed-out = Tempo esgotado no comando
error-ftp-no-data-timeout = Conexão perdida: sem dados há { $secs } s
error-ftp-upload-refused = Upload recusado pelo servidor: { $context }
error-ftp-write-after-end = Escrita após o fim do upload: { $context }
error-tls-trusted-cert-unusable = Não foi possível usar o certificado confiável { $path }
error-tls-verifier-build-failed = Não foi possível criar o verificador de certificados
error-tls-configure-failed = Não foi possível configurar o TLS

## Errors — network, files and local disk

error-fs-permission-denied = Permissão negada: { $path }
error-fs-no-such-file = Arquivo ou pasta inexistente: { $path }
error-fs-not-a-regular-file = Não é um arquivo regular: { $path }
error-fs-is-a-directory = É uma pasta: { $path }
error-fs-not-a-directory = Não é uma pasta: { $path }
error-net-connection-timed-out = Tempo de conexão esgotado
error-net-connection-failed = Falha na conexão
error-net-not-connected = Não conectado
error-net-disconnected-by-user = Desconectado pelo usuário
error-net-connection-lost = Conexão perdida
error-local-invalid-path = Caminho local inválido
error-local-not-a-file = Não é um arquivo: { $path }
error-local-offset-beyond-end = Posição { $offset } além do fim de { $path }
error-local-cannot-resume-missing = Não foi possível retomar um arquivo inexistente: { $path }
error-local-is-a-directory = É uma pasta: { $path }
error-local-not-a-directory = Não é uma pasta: { $path }
error-local-set-modified-failed = Falha ao definir a data de modificação: { $reason }


## Errors — vault, profiles and backups
error-backup-unsupported-parameters = Parâmetros de backup não suportados
error-backup-not-a-backup = Não é um backup do hoprs
error-backup-truncated = O arquivo de backup está truncado
error-backup-unsupported-version = Versão de backup não suportada
error-backup-invalid-key-length = Tamanho de chave do backup inválido
error-backup-random-failed = O gerador de números aleatórios do sistema falhou
error-backup-cannot-encrypt = Não foi possível criptografar o backup
error-backup-wrong-password = Senha incorreta ou arquivo danificado
error-backup-too-large = O arquivo de backup é grande demais
error-backup-cannot-build = Não foi possível montar o backup
error-backup-invalid-content = Conteúdo do backup inválido
error-backup-password-empty = A senha do backup está vazia
error-backup-cannot-read-secret = Não foi possível ler um segredo para o backup
error-backup-partial-no-replace = Este arquivo tem só uma parte de um backup. Restaure com Mesclar.
error-vault-keyring-error = Erro no keyring do sistema
error-vault-keyring-unavailable = Keyring do sistema indisponível
error-vault-unknown-site = Site desconhecido
error-vault-unknown-workspace = Workspace desconhecido
error-vault-workspace-unknown-site = Site desconhecido no workspace
error-vault-site-secrets-not-deleted = Site removido, mas não foi possível excluir seus segredos do keyring
error-profile-invalid-path = Caminho de perfil inválido
error-profile-symlink-loop = Links simbólicos demais encadeados
error-profile-newer-version = O perfil foi gravado por uma versão mais nova do hoprs
error-profile-cannot-read = Não foi possível ler { $file }
error-profile-duplicate-id = ID duplicado em { $file }
error-profile-cannot-save = Não foi possível salvar { $file }
error-profile-site-id-unreadable = O ID do site pertence a uma entrada ilegível
error-profile-workspace-id-unreadable = O ID do workspace pertence a uma entrada ilegível

## Errors — transfers and the queue
error-transfer-already-finished = A transferência já terminou
error-transfer-not-paused = A transferência não está pausada
error-transfer-duplicate-name-no-retry = Um nome duplicado em uma listagem de pasta não pode ser repetido
error-transfer-not-failed-or-cancelled = A transferência não falhou nem foi cancelada
error-transfer-not-awaiting-decision = A transferência não está aguardando uma decisão
error-transfer-size-mismatch = Tamanho diferente após a transferência
error-transfer-items-failed = { $failed } itens falharam
error-transfer-duplicate-name = Nome duplicado na listagem da pasta
error-transfer-worker-panicked = O worker encerrou com um erro inesperado
error-transfer-destination-not-file = O destino não é um arquivo comum
error-transfer-invalid-listing-name = Nome inválido na listagem da pasta
error-transfer-unknown = Transferência desconhecida
error-transfer-engine-stopped = O mecanismo de transferência parou
error-queue-database-error = Erro no banco de dados da fila
error-queue-newer-version = A fila de transferências foi salva por uma versão mais nova do hoprs

## Errors — import, settings, editing and sessions
error-import-cannot-read = Não foi possível ler o sitemanager.xml
error-import-file-too-large = O sitemanager.xml é grande demais
error-import-delete-symlink = Não excluído: o caminho é um link simbólico
error-import-delete-not-regular = Não excluído: não é um arquivo comum
error-import-delete-not-filezilla = Não excluído: não é um arquivo do Gerenciador de Sites do FileZilla
error-settings-cannot-read = Não foi possível ler o settings.toml
error-update-bad-response = O servidor de atualizações deu uma resposta inesperada
error-update-checksum-missing = { $name } não está no SHA256SUMS da versão
error-update-checksum-mismatch = O arquivo baixado não confere com o checksum
error-update-check-failed = Não foi possível verificar se há atualizações
error-update-download-failed = Não foi possível baixar a atualização
error-update-dev-build = A verificação de atualizações fica desligada em builds de desenvolvimento (defina HOPRS_UPDATE_URL)
error-update-download-busy = Esta atualização já está sendo baixada
error-app-opener-task-failed = A tarefa de abertura falhou
error-app-opener-failed = { $opener } falhou
error-app-vault-task-failed = A tarefa do cofre falhou
error-app-background-task-failed = A tarefa em segundo plano falhou
error-app-no-config-dir = Sem pasta de configuração
error-app-drop-needs-site = Abra um site conectado para soltar arquivos nele
error-app-drop-pane-busy = A pasta remota está ocupada ou ainda não foi listada; solte de novo em instantes
error-app-drop-not-connected = O site não está conectado; reconecte-o para soltar arquivos nele
error-app-drop-unsupported = Não é possível fazer upload deste caminho
error-app-site-endpoint-changed = O host ou a porta do site mudou: esta sessão foi fechada. Reconecte para usar as novas configurações.
error-session-closed = Sessão encerrada
error-session-shell-closed = Shell encerrado
error-session-terminal-needs-ssh = Terminais exigem um site SSH
error-fs-already-exists = Já existe
error-edit-unknown = Edição inexistente
error-edit-only-files = Só é possível editar arquivos
error-edit-remote-changed = O arquivo remoto mudou
error-edit-local-gone = O arquivo local não existe mais
error-edit-copy-exists = A cópia já existe
error-edit-cannot-watch = Não foi possível monitorar os arquivos editados
error-edit-no-parent = Sem pasta pai
error-edit-unmatched-quote = O comando do editor tem aspas sem fechar
error-edit-no-editor = Nenhum comando de editor definido
error-edit-cannot-start-editor = Não foi possível iniciar o editor
error-edit-path-refused = Este caminho remoto não pode ser editado localmente
error-drive-connect-cancelled = Conexão com o Google Drive cancelada
error-drive-connect-timed-out = O navegador não respondeu em 5 minutos
error-drive-connect-denied = O acesso ao Google Drive não foi concedido
error-drive-connect-failed = Não foi possível conectar ao Google Drive

# The folder compare panel (between the panes and the queue).
compare-button = Comparar
compare-title = Diferenças
compare-auto = Comparar automaticamente
compare-close = Fechar o painel de diferenças
compare-collapse = Recolher
compare-expand = Expandir
compare-group-only-local = Só neste computador ({ $count })
compare-group-only-remote = Só no servidor ({ $count })
compare-group-different = Diferentes ({ $count })
compare-group-unreadable = Não foi possível ler ({ $count })
compare-summary-only-here = { $count } só aqui
compare-summary-only-server = { $count } só no servidor
compare-summary-different =
    { $count ->
    [1] 1 diferente
   *[other] { $count } diferentes
    }
compare-local-newer = Local mais novo
compare-remote-newer = Servidor mais novo
compare-size-differs = Tamanho diferente
compare-kind-differs = Arquivo × pasta
compare-unreadable = Não foi possível ler: { $error }
compare-queued = Na fila
compare-tip-local = Este computador: { $when }
compare-tip-remote = Servidor: { $when }
compare-upload-selected = Fazer upload dos selecionados
compare-download-selected = Fazer download dos selecionados
compare-identical =
    { $count ->
    [0] { $count } idênticos
    [one] { $count } idêntico
   *[other] { $count } idênticos
    }
compare-all-identical =
    { $count ->
    [0] As duas pastas são idênticas (0 itens)
    [one] As duas pastas são idênticas (1 item)
   *[other] As duas pastas são idênticas ({ $count } itens)
    }
compare-running =
    { $count ->
    [0] Comparando… 0 itens
    [one] Comparando… 1 item
   *[other] Comparando… { $count } itens
    }
compare-paused = { $count } itens vistos até agora; a lista é parcial.
compare-continue = Comparar tudo
compare-stop = Parar
compare-stopped = Comparação interrompida: { $error }
compare-partial = Interrompida; a lista é parcial.
compare-again = Comparar de novo
compare-roots = { $local } ↔ { $remote }
compare-ellipsis = …
