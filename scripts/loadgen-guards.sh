# Shared fail-closed guards for scripts/loadgen.sh.
# Sourced, not executed: validates env knobs before any cargo invocation.
#
# shellcheck shell=bash

loadgen_validate_url() {
  local base="$1" authority host
  [[ -z "$base" ]] && return 0
  case "$base" in
    *://*@*) echo "loadgen-guards: MAIDAN_LOADGEN_URL must not carry userinfo" >&2; return 1 ;;
  esac
  if [[ "$base" == http://* ]]; then
    authority="${base#http://}"; authority="${authority%%/*}"
    case "$authority" in
      "[::1]" | "[::1]:"*) host="[::1]" ;;
      *) host="${authority%%:*}" ;;
    esac
    case "$host" in
      127.0.0.1 | localhost | "[::1]") ;;
      *) echo "loadgen-guards: refusing plain http to $base; use https or loopback" >&2; return 1 ;;
    esac
  elif [[ "$base" != https://* ]]; then
    echo "loadgen-guards: MAIDAN_LOADGEN_URL must be an http(s) URL, got ${base:0:64}" >&2; return 1
  fi
  case "$base" in
    *'?'* | *'#'*) echo "loadgen-guards: MAIDAN_LOADGEN_URL must not carry a query or fragment" >&2; return 1 ;;
  esac
}

loadgen_validate_bearer() {
  local token="$1" url="$2"
  if [[ -z "$url" ]]; then
    [[ -z "$token" ]] && return 0
    echo "loadgen-guards: MAIDAN_LOADGEN_BEARER is set without MAIDAN_LOADGEN_URL" >&2
    return 1
  fi
  if [[ -z "$token" || ${#token} -lt 16 || ${#token} -gt 512 ]]; then
    echo "loadgen-guards: MAIDAN_LOADGEN_BEARER must be 16..512 characters when targeting a URL" >&2
    return 1
  fi
  if [[ "$token" =~ [[:space:]] ]]; then
    echo "loadgen-guards: MAIDAN_LOADGEN_BEARER must not contain whitespace" >&2
    return 1
  fi
}

loadgen_validate_ids() {
  local ids="$1" url="$2"
  local -a parts
  if [[ -z "$url" ]]; then
    [[ -z "$ids" ]] && return 0
    echo "loadgen-guards: MAIDAN_LOADGEN_IDS is set without MAIDAN_LOADGEN_URL" >&2
    return 1
  fi
  if [[ -z "$ids" ]]; then
    echo "loadgen-guards: MAIDAN_LOADGEN_IDS (workspace|channel|thread|member) is required with a URL" >&2
    return 1
  fi
  IFS='|' read -r -a parts <<<"$ids"
  if [[ ${#parts[@]} -ne 4 ]]; then
    echo "loadgen-guards: MAIDAN_LOADGEN_IDS must be workspace|channel|thread|member" >&2
    return 1
  fi
  local part
  for part in "${parts[@]}"; do
    if [[ ! "$part" =~ ^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$ ]]; then
      echo "loadgen-guards: each MAIDAN_LOADGEN_IDS field must be a UUID" >&2
      return 1
    fi
  done
}

loadgen_validate_int_bounds() {
  local name="$1" value="$2" min="$3" max="$4"
  if [[ ! "$value" =~ ^[0-9]+$ ]]; then
    echo "loadgen-guards: $name must be an integer, got ${value:0:32}" >&2
    return 1
  fi
  if (( value < min || value > max )); then
    echo "loadgen-guards: $name must be $min..$max, got $value" >&2
    return 1
  fi
}

loadgen_validate_knobs() {
  local concurrency="${MAIDAN_LOADGEN_CONCURRENCY:-8}"
  local ops="${MAIDAN_LOADGEN_OPS:-50}"
  local duration="${MAIDAN_LOADGEN_DURATION_SECS:-0}"
  local url="${MAIDAN_LOADGEN_URL:-}"
  local bearer="${MAIDAN_LOADGEN_BEARER:-}"
  local ids="${MAIDAN_LOADGEN_IDS:-}"

  loadgen_validate_int_bounds MAIDAN_LOADGEN_CONCURRENCY "$concurrency" 1 256 || return 1
  loadgen_validate_int_bounds MAIDAN_LOADGEN_OPS "$ops" 1 100000 || return 1
  loadgen_validate_int_bounds MAIDAN_LOADGEN_DURATION_SECS "$duration" 0 86400 || return 1
  loadgen_validate_url "$url" || return 1
  loadgen_validate_bearer "$bearer" "$url" || return 1
  loadgen_validate_ids "$ids" "$url" || return 1
}

replica_validate_port() {
  local name="$1" port="$2"
  if [[ ! "$port" =~ ^[0-9]+$ ]]; then
    echo "replica-harness: $name must be an integer port, got ${port:0:32}" >&2
    return 1
  fi
  if (( port < 1024 || port > 65535 )); then
    echo "replica-harness: $name must be 1024..65535, got $port" >&2
    return 1
  fi
}

replica_validate_ports() {
  local primary="$1" replica="$2"
  replica_validate_port MAIDAN_PRIMARY_PORT "$primary" || return 1
  replica_validate_port MAIDAN_REPLICA_PORT "$replica" || return 1
  if [[ "$primary" == "$replica" ]]; then
    echo "replica-harness: primary and replica ports must differ" >&2
    return 1
  fi
}
