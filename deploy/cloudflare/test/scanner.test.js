import { test } from "node:test";
import assert from "node:assert/strict";
import { isScannerProbe } from "../src/scanner.js";

test("the probes seen against dev.maidan.world are answered at the edge", () => {
  for (const path of [
    "/.env",
    "/api/.env",
    "/functions/.env",
    "/application/.env",
    "/.env.local",
    "/.git/config",
    "/.aws/credentials",
    "/wp-login.php",
    "/wp-admin/",
    "/phpmyadmin/",
    "/index.php",
    "/cgi-bin/luci",
  ]) {
    assert.ok(isScannerProbe(path), path);
  }
});

test("every path a Maidan route serves reaches the container", () => {
  for (const path of [
    "/",
    "/health",
    "/mcp",
    "/mcp/stream",
    "/ui/",
    "/ui/static/main.js",
    "/artifacts/ab12cd",
    "/oauth/token",
    "/.well-known/oauth-authorization-server",
    "/.well-known/mcp/server-card.json",
    "/workspaces/x/channels",
    "/ws/subscribe",
    "/favicon.ico",
    "/environments",
  ]) {
    assert.ok(!isScannerProbe(path), path);
  }
});

test("encoded and dotted scanner paths normalize before matching", () => {
  for (const path of [
    "/%2e%2e/.env",
    "/./.env",
    "//.env",
    "/api/%2eenv",
    "/.git%2fconfig",
    "/%2eaws/credentials",
  ]) {
    assert.ok(isScannerProbe(path), path);
  }
});

test("extra probe families the edge now refuses", () => {
  for (const path of [
    "/.htaccess",
    "/web.config",
    "/docker-compose.yml",
    "/.ssh/id_rsa",
    "/id_rsa",
    "/.DS_Store",
    "/actuator/env",
    "/actuator/health",
  ]) {
    assert.ok(isScannerProbe(path), path);
  }
});

test("normalizeScannerPath collapses slashes and decodes once", async () => {
  const { normalizeScannerPath } = await import("../src/scanner.js");
  assert.equal(normalizeScannerPath("/./.env"), "/.env");
  assert.equal(normalizeScannerPath("//api//.env"), "/api/.env");
  assert.equal(normalizeScannerPath("/%2egit/config"), "/.git/config");
  assert.equal(normalizeScannerPath(""), "/");
});
