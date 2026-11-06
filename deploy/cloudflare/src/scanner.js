// Paths no Maidan route serves, which scanners probe on every new domain
// within hours of its certificate: `/.env`, `/.git/config`, credentials
// files, WordPress and PHP. Kept apart from the Worker so it is testable
// without the Workers runtime.

const SCANNER_PATH =
  /(^|\/)(\.env|\.git|\.aws|\.ssh|\.htaccess|\.ds_store|web\.config|docker-compose|id_rsa|wp-[a-z]+|wordpress|phpmyadmin|cgi-bin|actuator)(\/|$|\.)|\.(php|aspx?|jsp|cgi)$/i;

/**
 * Decode and collapse a URL pathname before the scanner regex runs, so
 * `/%2e%2e/.env` and `/./.env` match the same way as `/.env`.
 */
export function normalizeScannerPath(pathname) {
  let path = typeof pathname === "string" && pathname.length > 0 ? pathname : "/";
  try {
    path = decodeURIComponent(path);
  } catch {
    // Malformed percent-encoding: keep the raw path and still regex it.
  }
  path = path.replace(/\/{2,}/g, "/");
  while (path.includes("/./")) {
    path = path.replace(/\/\.\//g, "/");
  }
  if (path.endsWith("/.")) {
    path = path.slice(0, -1) || "/";
  }
  return path;
}

export function isScannerProbe(pathname) {
  return SCANNER_PATH.test(normalizeScannerPath(pathname));
}
