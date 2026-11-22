import type { Dict } from "./en";

// Portuguese (Brazil) copy for /pt-br/. Product terms stay in English where Brazilian developers
// use them (SSH, SFTP, FTP, terminal, upload, changelog). macOS and Windows steps use the names
// those systems show in pt-BR.
export const ptBr: Dict = {
  meta: {
    title: "hoprs: cliente SSH, SFTP e FTP para Linux, macOS e Windows",
    description:
      "hoprs é um cliente SSH, SFTP, FTP e FTPS gratuito e de código aberto, com terminal integrado. Importa seus sites do FileZilla. Para Linux, macOS e Windows.",
    ogLocale: "pt_BR",
    ogImageAlt: "hoprs, SSH, SFTP e FTP numa janela só: a aba de arquivos com uma pasta local e a mesma pasta num servidor.",
  },
  nav: {
    label: "Principal",
    home: "hoprs, início",
    download: "Baixar",
    otherLanguage: "English",
  },
  hero: {
    headline: "SSH, SFTP e FTP numa janela só.",
    sentence: "Um cliente de arquivos rápido e nativo para Linux, macOS e Windows.",
    downloadFor: "Baixar para",
    shotAlt:
      "Aba de arquivos do hoprs: dois arquivos locais sendo arrastados para a pasta do servidor de build, que ganha um contorno tracejado e o rótulo Upload 2 items to /home/alice/web-app; embaixo, o painel de comparação de pastas listando o que difere entre as duas pastas e um upload em andamento na fila de transferências.",
  },
  fast: {
    title: "Leve na memória, rápido na rede.",
    label: "Desempenho medido",
    startup: "até o primeiro frame",
    idle: "de memória em repouso",
    rows: (rows: string) => `p95 de renderização e layout, ${rows} linhas`,
    sftp: "SFTP, 1 GiB com 50 ms de RTT",
    measuredOn: "Medido num runner do GitHub Actions em",
    how: "Como medimos",
  },
  connection: {
    title: "Uma conexão, duas abas.",
    lead: "Uma aba de terminal e uma de arquivos usam a mesma conexão SSH. Abra mais abas na mesma sessão.",
    shotAlt:
      "Aba de terminal do hoprs na mesma conexão SSH, mostrando ls -la da pasta do projeto no servidor enquanto o upload continua na fila embaixo.",
  },
  filezilla: {
    title: "Seus sites do FileZilla vêm junto.",
    label: "O que muda quando você troca o FileZilla pelo hoprs",
    kept: "sites salvos, um clique para abrir",
    added: [
      "importa o sitemanager.xml, com um relatório do que ficou de fora",
      "senhas no chaveiro do sistema",
      "um único arquivo de backup criptografado",
      "terminal e arquivos na mesma conexão",
    ],
  },
  openSource: {
    title: "Código aberto.",
    lead: "MIT OR Apache-2.0. Feito com iced, russh, suppaftp e alacritty_terminal.",
    link: "Leia o código no GitHub",
  },
  install: {
    title: "É só instalar.",
    systems: "Sistema operacional",
    method: "Forma de instalação",
    source: "compilar",
    copy: "Copiar",
    copied: "Copiado",
    pressCtrlC: "Pressione Ctrl+C para copiar",
    linuxLead: "Ubuntu 24.04 ou mais recente, amd64.",
    macLead: "macOS 11 ou mais recente, Apple silicon e Intel.",
    download: "Baixar",
    macSteps: [
      "Abra a imagem de disco e arraste o hoprs para Aplicativos.",
      "O app não é notarizado. Abra o hoprs uma vez e vá em Ajustes do Sistema → Privacidade e Segurança → Abrir Mesmo Assim.",
    ],
    macTerminal: "Ou, no Terminal:",
    checkWith: "Confira com o",
    windowsLead: "Windows, 64 bits.",
    portable: "Portátil",
    windowsSteps: [
      "Rode o instalador ou descompacte a versão portátil onde quiser.",
      "O app não é assinado. Se o SmartScreen mostrar “O Windows protegeu o computador”, clique em Mais informações → Executar assim mesmo.",
    ],
  },
  faq: {
    title: "Perguntas frequentes.",
    items: [
      { q: "Em quais sistemas ele roda?", a: "Linux (Ubuntu 24.04 ou mais recente, amd64), macOS 11 ou mais recente e Windows 64 bits." },
      { q: "O macOS ou o Windows diz que o app não é confiável. É normal?", a: "É. Os builds não são notarizados nem assinados com um certificado pago. No macOS, abra Ajustes do Sistema → Privacidade e Segurança e clique em Abrir Mesmo Assim; ou rode xattr -dr com.apple.quarantine /Applications/hoprs.app. No Windows, em “O Windows protegeu o computador”, clique em Mais informações → Executar assim mesmo. Antes, confira o download com o SHA256SUMS." },
      { q: "Onde ficam as senhas?", a: "No chaveiro do sistema: Secret Service no Linux, Keychain (Acesso às Chaves) no macOS, Gerenciador de Credenciais no Windows. Os arquivos de sites não guardam segredos." },
      { q: "Dá para usar FTP sem criptografia?", a: "Dá. Ele aparece como ftp (unencrypted). FTPS, explícito e implícito, também é suportado." },
      { q: "Ele importa do FileZilla?", a: "Importa. Ele lê o sitemanager.xml e mostra o que foi importado e o que ficou de fora." },
      { q: "Agentes de código podem usar meus sites?", a: "Podem, pelo MCP. Rode claude mcp add hoprs -- \"<caminho do hoprs>\" mcp (Configurações › Agentes (MCP) mostra o comando exato e um trecho JSON para outros clientes). Os agentes alcançam só os sites que você permitir, um a um, no nível que você escolher: leitura ou tudo. O hoprs precisa estar aberto, guarda as senhas e mostra o que o agente faz no log. O próprio hoprs não envia nada a lugar nenhum por causa disso, e o canal até o agente fica no seu computador. O que um agente lê pelo hoprs (conteúdo de arquivos, saída de comandos, listagens) vai para onde esse agente enviar, por exemplo para a empresa que opera o modelo dele. Os arquivos que um agente envia ou baixa circulam entre o seu computador e os seus próprios servidores." },
      { q: "Ele coleta dados?", a: "O app, não. Ele não tem conta e só se conecta aos servidores que você adicionar, além de uma requisição ao GitHub para ver se há versão nova (dá para desligar em Configurações). Este site conta as visitas com o Cloudflare Web Analytics, que não usa cookies nem guarda dados pessoais." },
      { q: "Como atualizo?", a: "O hoprs consulta o GitHub por uma versão nova ao abrir e uma vez por dia, e mostra um cartão com o botão Atualizar (Configurações › Geral desliga a consulta). Atualizar baixa o arquivo certo, confere com o SHA256SUMS da versão e abre o instalador do sistema. Você também pode baixar a versão nova e instalar por cima da antiga: no Linux, sudo apt install ./hoprs_<versão>-1_amd64.deb; no macOS, arraste o hoprs para Aplicativos e substitua; no Windows, rode o novo .msi (ele substitui a versão anterior) ou descompacte a versão portátil por cima da pasta antiga. Sites, workspaces, configurações e senhas salvas são mantidos." },
      { q: "Como desinstalo?", a: "Linux: sudo apt remove hoprs. macOS: arraste o hoprs para fora de Aplicativos. Windows: remova em Aplicativos instalados. Seus dados ficam; para apagá-los também, exclua ~/.config/hoprs e ~/.local/share/hoprs (Linux), ~/Library/Application Support/hoprs (macOS) ou %APPDATA%\\hoprs (Windows), e as entradas do chaveiro do serviço hoprs/<site-id> (no Gerenciador de Credenciais do Windows aparecem como password.hoprs/<site-id> e key-passphrase.hoprs/<site-id>)." },
      { q: "Como reporto um bug?", a: "Abra uma issue no GitHub. Para vulnerabilidades, use o reporte privado do GitHub (Security Advisories)." },
    ],
  },
  footer: {
    label: "Rodapé",
    releases: "Versões",
    changelog: "Changelog",
    security: "Segurança",
    license: "Licença",
    licensed: "Licenciado sob MIT OR Apache-2.0.",
    madeBy: "Feito por",
    updated: "Atualizado em",
    privacy: "Privacidade",
  },
  privacy: {
    title: "Política de privacidade",
    metaTitle: "Política de privacidade | hoprs",
    metaDescription:
      "O que o hoprs faz com os seus dados: o app não coleta nada, a checagem de atualização faz um pedido ao GitHub e o Google Drive só é usado se você escolher.",
    updated: "Última alteração: 22 de novembro de 2026.",
    back: "Voltar ao hoprs",
    sections: [
      {
        heading: "Quem é o responsável",
        paragraphs: [
          "O hoprs é um app de desktop de código aberto, publicado por {responsible}. Ele responde pelo que esta página descreve.",
          "Dúvidas sobre privacidade: {email}.",
        ],
      },
      {
        heading: "O app não coleta nada",
        paragraphs: [
          "O hoprs não tem conta, não envia telemetria e não tem servidor próprio. Seus sites, workspaces e configurações ficam em arquivos no seu computador, e as senhas ficam no chaveiro do sistema.",
          "O acesso de agentes (MCP) vem desligado. Quando você o liga, um programa que você inicia neste computador, como um agente de código, pode falar com o hoprs por um canal que fica no próprio computador e só aceita o seu usuário. O próprio hoprs não envia nada a lugar nenhum por causa disso. O que um agente lê pelo hoprs vai para onde esse agente enviar, por exemplo para a empresa que opera o modelo dele.",
          "Fora o que está descrito abaixo (a checagem de atualização e, nas versões que o oferecem, o Google Drive), o app só se conecta aos servidores que você adiciona.",
        ],
      },
      {
        heading: "Checagem de atualização",
        paragraphs: [
          "Poucos segundos depois de abrir, e depois uma vez por dia, o hoprs faz um pedido à API do GitHub para saber qual é a versão mais recente. O pedido não leva nenhum identificador: só a versão do app no cabeçalho User-Agent (hoprs/<versão>) e um cabeçalho Accept padrão.",
          "Em Configurações → Geral dá para desligar a checagem. Ao clicar em Atualizar, o instalador é baixado do GitHub.",
        ],
      },
      {
        heading: "Google Drive (nas versões que o oferecem)",
        paragraphs: [
          "O hoprs só fala com o Google quando você clica em Conectar ou Desconectar, ou exporta para o Google Drive. O que vai para o Google é a troca de autorização OAuth, o arquivo de exportação criptografado (enviado para uma pasta chamada hoprs) e, ao desconectar, o pedido para revogar a autorização.",
          "A única permissão que o hoprs pede é drive.file: acesso aos arquivos que o próprio app criou. O hoprs não consegue ver, listar nem alterar mais nada no seu Drive.",
          "O arquivo é criptografado no seu computador, com uma senha que nunca sai dele. A autorização fica guardada no chaveiro do sistema, e o e-mail da conta é lido só para mostrar qual conta está conectada.",
          "Desconectar no hoprs apaga a autorização do chaveiro deste computador e pede ao Google que a revogue; se esse pedido falhar (sem internet, por exemplo), a autorização some deste computador, mas pode continuar valendo do lado do Google. Para removê-la do lado do Google com certeza, use https://myaccount.google.com/permissions.",
          "O uso que o hoprs faz das informações recebidas das APIs do Google segue a Google API Services User Data Policy, incluindo os requisitos de Uso Limitado (Limited Use). Em inglês: “hoprs's use of information received from Google APIs adheres to the Google API Services User Data Policy, including the Limited Use requirements.”",
        ],
      },
      {
        heading: "Este site",
        paragraphs: [
          "As visitas a hoprs.online são contadas com o Cloudflare Web Analytics, que não define cookies e não guarda dados pessoais. O site fica atrás da Cloudflare.",
        ],
      },
      {
        heading: "Alterações",
        paragraphs: ["Se esta política mudar, o novo texto é publicado nesta página, e a data acima muda junto."],
      },
    ],
  },
};
