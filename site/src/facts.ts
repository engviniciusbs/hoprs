// Filled by hand per release. facts.test.ts checks the version against the workspace Cargo.toml.
const maintainer = "engviniciusbs";
// The person responsible for the app and the contact of the privacy page.
const responsible = "Vinícius Sarmento";
const privacyEmail = "privacy@hoprs.online";
const maintainerUrl = `https://github.com/${maintainer}`;
const repoUrl = `${maintainerUrl}/hoprs`;
const version = "1.5.0";
// The date of the GitHub Release above (CHANGELOG.md), shown as "Updated" in the footer.
const releaseDate = "2026-11-18";
const debName = `hoprs_${version}-1_amd64.deb`;
const dmgName = `hoprs_${version}_macos_universal.dmg`;
const msiName = `hoprs_${version}_windows_x64.msi`;
const zipName = `hoprs_${version}_windows_x64.zip`;
const releaseBase = `${repoUrl}/releases/download/v${version}`;

export const facts = {
  siteUrl: "https://hoprs.online",
  version,
  releaseDate,
  maintainer,
  responsible,
  privacyEmail,
  maintainerUrl,
  debName,
  debUrl: `${releaseBase}/${debName}`,
  dmgName,
  dmgUrl: `${releaseBase}/${dmgName}`,
  msiName,
  msiUrl: `${releaseBase}/${msiName}`,
  zipName,
  zipUrl: `${releaseBase}/${zipName}`,
  sumsUrl: `${releaseBase}/SHA256SUMS`,
  repoUrl,
  releasesUrl: `${repoUrl}/releases`,
  changelogUrl: `${repoUrl}/blob/main/CHANGELOG.md`,
  securityUrl: `${repoUrl}#security`,
  perfDocUrl: `${repoUrl}/blob/main/docs/perf/README.md`,
  licenseUrl: `${repoUrl}/blob/main/LICENSE-MIT`,
  apacheLicenseUrl: `${repoUrl}/blob/main/LICENSE-APACHE`,
  issuesUrl: `${repoUrl}/issues`,
  perf: { startupMs: 84, idleMb: 33, rowsP95Ms: 0.58, rows: 100000, sftpVsScp: 0.99, measuredOn: "2026-11-13" },
} as const;
