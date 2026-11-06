# Shared fail-closed guards for scripts/demo-board.sh and scripts/demo-handoff.sh.
# Sourced, not executed: validates URLs/tokens/paths before any bearer curl.
#
# shellcheck shell=bash

demo_validate_base() {
  local base="$1" authority host
  case "$base" in
    *://*@*) echo "demo-guards: MAIDAN_URL must not carry userinfo" >&2; return 1 ;;
  esac
  if [[ "$base" == http://* ]]; then
    authority="${base#http://}"; authority="${authority%%/*}"
    case "$authority" in
      "[::1]" | "[::1]:"*) host="[::1]" ;;
      *) host="${authority%%:*}" ;;
    esac
    case "$host" in
      127.0.0.1 | localhost | "[::1]") ;;
      *) echo "demo-guards: refusing plain http to $base; use https or loopback" >&2; return 1 ;;
    esac
  elif [[ "$base" != https://* ]]; then
    echo "demo-guards: MAIDAN_URL must be an http(s) URL, got $base" >&2; return 1
  fi
  case "$base" in
    *'?'* | *'#'*) echo "demo-guards: MAIDAN_URL must not carry a query or fragment" >&2; return 1 ;;
  esac
}

demo_validate_workspace() {
  local ws="$1"
  # Workspace ids are UUIDs (maidan init prints them in parentheses).
  if [[ ! "$ws" =~ ^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$ ]]; then
    echo "demo-guards: workspace id must be a UUID, got ${ws:0:32}" >&2; return 1
  fi
}

demo_validate_token() {
  local token="$1"
  if [[ -z "$token" || ${#token} -lt 16 || ${#token} -gt 512 ]]; then
    echo "demo-guards: bearer token must be 16..512 characters" >&2; return 1
  fi
  if [[ "$token" =~ [[:space:]] ]]; then
    echo "demo-guards: bearer token must not contain whitespace" >&2; return 1
  fi
}

demo_validate_pause() {
  local pause="$1"
  if [[ ! "$pause" =~ ^[0-9]+$ ]] || (( pause > 60 )); then
    echo "demo-guards: DEMO_PAUSE must be an integer 0..60 seconds" >&2; return 1
  fi
}

demo_validate_ttl() {
  local ttl="$1"
  [[ -z "$ttl" ]] && return 0
  if [[ ! "$ttl" =~ ^[1-9][0-9]{0,7}$ ]] || (( ttl < 60 || ttl > 86400 )); then
    echo "demo-guards: DEMO_TOKEN_TTL_SECS must be 60..86400" >&2; return 1
  fi
}

demo_validate_rest() {
  local method="$1" path="$2"
  case "$method" in
    GET|POST|PATCH|PUT|DELETE) ;;
    *) echo "demo-guards: refusing REST method $method" >&2; return 1 ;;
  esac
  case "$path" in
    /workspaces/*|/channels/*|/threads/*|/members/*) ;;
    *) echo "demo-guards: refusing REST path $path" >&2; return 1 ;;
  esac
  case "$path" in
    *..*) echo "demo-guards: refusing REST path with .." >&2; return 1 ;;
  esac
  if [[ "$path" == *'?'* || "$path" == *'#'* ]]; then
    echo "demo-guards: refusing REST path with query/fragment" >&2; return 1
  fi
}

demo_validate_mcp_tool() {
  local tool="$1"
  if [[ ! "$tool" =~ ^[A-Za-z][A-Za-z0-9_]{0,63}$ ]]; then
    echo "demo-guards: refusing MCP tool name $tool" >&2; return 1
  fi
}
