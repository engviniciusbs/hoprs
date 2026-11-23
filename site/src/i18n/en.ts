// English copy: the default page at /. pt-br.ts has the same shape (typed as Dict), so a
// missing or extra key fails typecheck. Facts (version, URLs, numbers) live in facts.ts.
export const en = {
  meta: {
    title: "hoprs: SSH, SFTP and FTP client for Linux, macOS and Windows",
    description:
      "hoprs is a free, open source SSH, SFTP, FTP and FTPS client with a built-in terminal. It imports your FileZilla sites. For Linux, macOS and Windows.",
    ogLocale: "en_US",
    ogImageAlt: "hoprs, SSH, SFTP and FTP in one window: the files tab with a local folder and the same folder on a server.",
  },
  nav: {
    label: "Primary",
    home: "hoprs, home",
    download: "Download",
    // The link to the other language, written in that language.
    otherLanguage: "Português",
  },
  hero: {
    headline: "SSH, SFTP and FTP in one window.",
    sentence: "A fast, native file client for Linux, macOS and Windows.",
    downloadFor: "Download for",
    shotAlt:
      "hoprs files tab: two local files being dragged onto the build server's folder, which has a dashed outline and the label Upload 2 items to /home/alice/web-app; below, the folder comparison panel listing what differs between the two folders, and one upload running in the transfer queue.",
  },
  fast: {
    title: "Light on memory, quick on the wire.",
    label: "Measured performance",
    startup: "to the first frame",
    idle: "memory at idle",
    rows: (rows: string) => `p95 view and layout, ${rows} rows`,
    sftp: "SFTP, 1 GiB at 50 ms RTT",
    measuredOn: "Measured on a GitHub Actions runner,",
    how: "How it is measured",
  },
  connection: {
    title: "One connection, two tabs.",
    lead: "A terminal tab and a files tab share one SSH connection. Add more tabs over the same session.",
    shotAlt:
      "hoprs terminal tab on the same SSH connection, showing ls -la of the project folder on the server while the upload keeps running in the queue below.",
  },
  filezilla: {
    title: "Your FileZilla sites, kept.",
    label: "What changes when you move from FileZilla to hoprs",
    kept: "saved sites, one click to open",
    added: [
      "imports sitemanager.xml, with a report of what was skipped",
      "passwords in the system keyring",
      "one encrypted backup file",
      "terminal and files over one connection",
    ],
  },
  openSource: {
    title: "Open source.",
    lead: "MIT OR Apache-2.0. Built on iced, russh, suppaftp and alacritty_terminal.",
    link: "Read the code on GitHub",
  },
  install: {
    title: "Ready when you are.",
    systems: "Operating system",
    method: "Install method",
    source: "source",
    copy: "Copy",
    copied: "Copied",
    pressCtrlC: "Press Ctrl+C to copy",
    linuxLead: "Ubuntu 24.04 or newer, amd64.",
    macLead: "macOS 11 or newer, Apple silicon and Intel.",
    download: "Download",
    macSteps: [
      "Open the disk image and drag hoprs to Applications.",
      "The app is not notarized. Open hoprs once, then go to System Settings, Privacy & Security, and choose Open Anyway.",
    ],
    macTerminal: "Or, in Terminal:",
    checkWith: "Check it with",
    windowsLead: "Windows, 64-bit.",
    portable: "Portable",
    windowsSteps: [
      "Run the installer, or unzip the portable build anywhere.",
      "The app is not signed. If SmartScreen says Windows protected your PC, choose More info, then Run anyway.",
    ],
  },
  faq: {
    title: "Questions.",
    items: [
      { q: "Which systems does it run on?", a: "Linux (Ubuntu 24.04 or newer, amd64), macOS 11 or newer, and 64-bit Windows." },
      { q: "macOS or Windows says the app is not trusted. Is that expected?", a: "Yes. The builds are not notarized or signed with a paid certificate. On macOS, open System Settings, Privacy & Security, and choose Open Anyway; or run xattr -dr com.apple.quarantine /Applications/hoprs.app. On Windows, choose More info, then Run anyway. Check the download against SHA256SUMS first." },
      { q: "Where are passwords stored?", a: "In the system keyring: Secret Service on Linux, Keychain on macOS, Credential Manager on Windows. Site files hold no secrets." },
      { q: "Is plain FTP allowed?", a: "Yes. It is labelled ftp (unencrypted). FTPS, explicit and implicit, is supported too." },
      { q: "Does it import from FileZilla?", a: "Yes. It reads sitemanager.xml and reports what was imported and what was skipped." },
      { q: "Can coding agents use my sites?", a: "From version 1.6.0, yes, through MCP. Run claude mcp add hoprs -- \"<path to hoprs>\" mcp (Settings › Agents (MCP) shows the exact command, and a JSON snippet for other clients). Agents reach only the sites you allow, one by one, at the level you choose: reading, or everything. hoprs must be open, keeps the passwords, and shows what the agent does in its log. hoprs itself sends nothing anywhere because of this, and the channel to the agent stays on your computer. What an agent reads through hoprs (file contents, command output, listings) goes wherever that agent sends it, for example to the company that runs its model. Files an agent uploads or downloads move between your computer and your own servers." },
      { q: "Does it collect data?", a: "The app: no. It has no accounts and talks only to the servers you add, plus a request to GitHub to check for a new version (you can turn it off in Settings), and Google, only when you connect Google Drive, export there or disconnect it. This website counts visits with Cloudflare Web Analytics, which sets no cookies and stores no personal data. The privacy page says what is sent." },
      { q: "How do I update it?", a: "hoprs checks GitHub for a new release when it starts and once a day, and shows a card with an Update button (Settings › General turns the check off). Update downloads the right file, checks it against the release's SHA256SUMS and opens the system installer. You can also download the new version and install it over the old one: on Linux, sudo apt install ./hoprs_<version>-1_amd64.deb; on macOS, drag hoprs to Applications and replace it; on Windows, run the new .msi (it replaces the old version) or unzip the portable build over the old folder. Sites, workspaces, settings and saved passwords are kept." },
      { q: "How do I uninstall it?", a: "Linux: sudo apt remove hoprs. macOS: drag hoprs out of Applications. Windows: remove it in Installed apps. Your data stays; to remove it too, delete ~/.config/hoprs and ~/.local/share/hoprs (Linux), ~/Library/Application Support/hoprs (macOS) or %APPDATA%\\hoprs (Windows), and the keyring entries for service hoprs/<site-id> (shown as password.hoprs/<site-id> and key-passphrase.hoprs/<site-id> in the Windows Credential Manager) and, if you connected Google Drive, the entry for service hoprs/google-drive (refresh-token.hoprs/google-drive there)." },
      { q: "How do I report a bug?", a: "Open an issue on GitHub. Report vulnerabilities privately through GitHub security advisories." },
    ],
  },
  footer: {
    label: "Footer",
    releases: "Releases",
    changelog: "Changelog",
    security: "Security",
    license: "License",
    licensed: "Licensed MIT OR Apache-2.0.",
    madeBy: "Made by",
    updated: "Updated",
    privacy: "Privacy",
  },
  privacy: {
    title: "Privacy policy",
    metaTitle: "Privacy policy | hoprs",
    metaDescription:
      "What hoprs does with your data: the app collects nothing, the update check sends one request to GitHub, and Google Drive is used only when you choose it.",
    updated: "Last changed: November 22, 2026.",
    back: "Back to hoprs",
    sections: [
      {
        heading: "Who is responsible",
        paragraphs: [
          "hoprs is an open source desktop app published by {responsible}. He is responsible for what this page describes.",
          "Questions about privacy: {email}.",
        ],
      },
      {
        heading: "The app collects nothing",
        paragraphs: [
          "hoprs has no account, no telemetry and no server of its own. Your sites, workspaces and settings stay in files on your computer, and passwords stay in the system keyring.",
          "In versions that offer it, agent access (MCP) is off by default. When you turn it on, a program you start on this computer, such as a coding agent, can talk to hoprs through a channel that stays on the computer and admits only your user. hoprs itself sends nothing anywhere because of it. What an agent reads through hoprs goes wherever that agent sends it, for example to the company that runs its model.",
          "Apart from the two things below (the update check and, in versions that offer it, Google Drive), the app connects only to the servers you add.",
        ],
      },
      {
        heading: "Update check",
        paragraphs: [
          "A few seconds after it starts, and once a day after that, hoprs makes one request to GitHub's API to ask for the latest release. The request carries no identifier: only the app's version in the User-Agent header (hoprs/<version>) and a standard Accept header.",
          "Settings → General turns the check off. When you click Update, the installer is downloaded from GitHub.",
        ],
      },
      {
        heading: "Google Drive (in versions that offer it)",
        paragraphs: [
          "hoprs talks to Google only when you click Connect, Disconnect or export to Google Drive. What goes to Google is the OAuth authorisation exchange, the encrypted export file (uploaded to a folder named hoprs) and, on Disconnect, the request to revoke the authorisation.",
          "The only permission hoprs asks for is drive.file: access to the files the app itself created. hoprs cannot see, list or change anything else in your Drive.",
          "The file is encrypted on your computer with a password that never leaves it. The authorisation is kept in the system keyring, and your account's email address is read only to show which account is connected.",
          "Disconnect in hoprs removes the authorisation from this computer's keyring and asks Google to revoke it; if that request fails (for example, offline), the authorisation is gone from this computer but may remain on Google's side. To remove it from Google's side for certain, use https://myaccount.google.com/permissions.",
          "hoprs's use of information received from Google APIs adheres to the Google API Services User Data Policy, including the Limited Use requirements.",
        ],
      },
      {
        heading: "This website",
        paragraphs: [
          "Visits to hoprs.online are counted with Cloudflare Web Analytics, which sets no cookies and stores no personal data. The site sits behind Cloudflare.",
        ],
      },
      {
        heading: "Changes",
        paragraphs: ["If this policy changes, the new text is published on this page, and the date above changes with it."],
      },
    ],
  },
};

export type Dict = typeof en;
