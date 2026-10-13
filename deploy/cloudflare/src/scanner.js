// Paths no Maidan route serves, which scanners probe on every new domain
// within hours of its certificate: `/.env`, `/.git/config`, credentials
// files, WordPress and PHP. Kept apart from the Worker so it is testable
// without the Workers runtime.
const SCANNER_PATH =
  /(^|\/)(\.env|\.git|\.aws|\.ssh|wp-[a-z]+|wordpress|phpmyadmin|cgi-bin)(\/|$|\.)|\.(php|aspx?|jsp|cgi)$/i;

export function isScannerProbe(pathname) {
  return SCANNER_PATH.test(pathname);
}
