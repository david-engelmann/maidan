#!/usr/bin/env bash
# SQLite backup and restore drill: the evidence that the SQLite procedure in
# docs/Production.md works.
#
# A writer keeps inserting into a WAL-mode database while backup.sh takes its
# snapshot, as a running server would. The drill then checks that:
#   - the snapshot is a whole database (integrity_check), taken mid-write;
#   - restore.sh refuses a non-empty target without --force;
#   - with --force it replaces the target, and none of the old database comes
#     back: a killed server's -wal left beside the file would otherwise be
#     replayed over the snapshot the next time it is opened;
#   - the restored database holds exactly the snapshot's rows, and the
#     artifact archive round-trips;
#   - --force replaces a target that is not a database at all, keeping the
#     replaced file's mode;
#   - both scripts decode a percent-encoded path as SQLx does, a trailing
#     newline included;
#   - a restore through a symlink keeps the mode of the database it points at.
#
# Usage: scripts/sqlite-backup-drill.sh   (needs bash and the sqlite3 CLI)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
writer=""
cleanup() {
  [[ -n "$writer" ]] && { touch "$stop" 2>/dev/null; wait "$writer" 2>/dev/null; } || true
  rm -rf "$work"
}
trap cleanup EXIT

fail() { echo "sqlite-backup-drill: FAIL: $*" >&2; exit 1; }

src_db="$work/live/maidan.db"
mkdir -p "$work/live" "$work/artifacts"
# Every call on the live database waits out a lock: the writer below holds one
# for each insert.
live() { sqlite3 -cmd ".timeout 10000" "$src_db" "$@"; }
live "PRAGMA journal_mode=WAL; CREATE TABLE events (id INTEGER PRIMARY KEY, body TEXT NOT NULL);" >/dev/null
echo "blob" > "$work/artifacts/ab12"

# Keep a write in flight the whole time the snapshot runs. The writer stops
# at a flag and is waited for: killing its loop would leave the insert it is
# running holding the lock.
stop="$work/stop-writer"
(
  i=0
  while [[ ! -e "$stop" ]]; do
    i=$((i + 1))
    live "INSERT INTO events (body) VALUES ('event $i');" || true
  done
) &
writer=$!
until [[ "$(live "SELECT count(*) FROM events")" -ge 50 ]]; do :; done

DATABASE_URL="sqlite://$src_db?mode=rwc" ARTIFACT_LOCALFS_ROOT="$work/artifacts" \
  bash "$here/backup.sh" "$work/backup" >/dev/null
touch "$stop"
wait "$writer" 2>/dev/null || true
writer=""

snap="$work/backup/maidan.sqlite"
[[ "$(sqlite3 "$snap" "PRAGMA integrity_check")" == "ok" ]] || fail "snapshot integrity"
snap_rows="$(sqlite3 "$snap" "SELECT count(*) FROM events")"
live_rows="$(live "SELECT count(*) FROM events")"
[[ "$snap_rows" -ge 50 ]] || fail "snapshot holds $snap_rows rows, expected at least 50"
[[ "$snap_rows" -lt "$live_rows" ]] || fail "the writer never overlapped the snapshot ($snap_rows of $live_rows)"
grep -q '^database=sqlite$' "$work/backup/MANIFEST.txt" || fail "manifest does not say sqlite"

# A database whose server was killed: its -wal still holds pages no checkpoint
# has copied into the file. (A clean close checkpoints and deletes the -wal,
# which would prove nothing.)
killed_server_db() {
  local db="$1" feed holder
  mkdir -p "$(dirname "$db")"
  sqlite3 "$db" "PRAGMA journal_mode=WAL; CREATE TABLE other (x);" >/dev/null
  feed="$(dirname "$db")/feed"
  mkfifo "$feed"
  sqlite3 "$db" < "$feed" >/dev/null &
  holder=$!
  exec 3>"$feed"
  echo "PRAGMA wal_autocheckpoint=0; INSERT INTO other VALUES (1);" >&3
  until [[ -s "$db-wal" ]]; do :; done
  sleep 0.2
  kill -9 "$holder"
  wait "$holder" 2>/dev/null || true
  exec 3>&-
  rm -f "$feed"
  [[ -s "$db-wal" ]] || fail "could not leave a -wal behind to test against"
}

# The restore must hold the snapshot's rows and nothing of the old database.
# Opening the file is what would replay a leftover -wal over it.
check_restored() {
  local db="$1"
  [[ -z "$(sqlite3 "$db" "SELECT name FROM sqlite_schema WHERE name = 'other'")" ]] \
    || fail "$2: the old database's -wal was replayed onto the restore"
  [[ "$(sqlite3 "$db" "PRAGMA integrity_check")" == "ok" ]] || fail "$2: restored integrity"
  [[ "$(sqlite3 "$db" "SELECT count(*) FROM events")" == "$snap_rows" ]] || fail "$2: restored row count"
}

# 1. A live target is refused without --force and replaced with it.
dst_db="$work/target/maidan.db"
killed_server_db "$dst_db"
if DATABASE_URL="sqlite://$dst_db" ARTIFACT_LOCALFS_ROOT="$work/restored-artifacts" \
  bash "$here/restore.sh" "$work/backup" >/dev/null 2>&1; then
  fail "restore into a non-empty target succeeded without --force"
fi
DATABASE_URL="sqlite://$dst_db" ARTIFACT_LOCALFS_ROOT="$work/restored-artifacts" \
  bash "$here/restore.sh" "$work/backup" --force >/dev/null
check_restored "$dst_db" "--force over a live target"

# 2. The database file was deleted but its -wal was not: the target looks
# empty, so no --force is needed, and the orphaned -wal must not survive.
orphan_db="$work/orphan/maidan.db"
killed_server_db "$orphan_db"
rm -f "$orphan_db"
DATABASE_URL="sqlite://$orphan_db" ARTIFACT_LOCALFS_ROOT="$work/restored-artifacts-2" \
  bash "$here/restore.sh" "$work/backup" >/dev/null
check_restored "$orphan_db" "an orphaned -wal"
cmp -s "$work/artifacts/ab12" "$work/restored-artifacts/ab12" || fail "artifact archive round trip"

# 3. A target that is not a database, which is when a restore is most needed.
# Without --force it is refused and left alone; --force replaces it without
# opening it, and the restore keeps the mode of the file it replaced.
garbage_db="$work/garbage/maidan.db"
mkdir -p "$(dirname "$garbage_db")"
echo "not a database" > "$garbage_db"
chmod 600 "$garbage_db"
if DATABASE_URL="sqlite://$garbage_db" ARTIFACT_LOCALFS_ROOT="$work/restored-artifacts-3" \
  bash "$here/restore.sh" "$work/backup" >/dev/null 2>&1; then
  fail "restore over a corrupt target succeeded without --force"
fi
[[ "$(cat "$garbage_db")" == "not a database" ]] || fail "a refused restore changed the corrupt target"
DATABASE_URL="sqlite://$garbage_db" ARTIFACT_LOCALFS_ROOT="$work/restored-artifacts-3" \
  bash "$here/restore.sh" "$work/backup" --force >/dev/null \
  || fail "restore --force could not replace a corrupt target"
check_restored "$garbage_db" "--force over a corrupt target"
mode="$(stat -c '%a' "$garbage_db" 2>/dev/null || stat -f '%Lp' "$garbage_db")"
[[ "$mode" == 600 ]] || fail "the restore did not keep the replaced file's mode (600, got $mode)"

# 4. SQLx percent-decodes the path, so `%3F` is a `?` in the file name, not the
# start of the options. Both scripts must touch the file the server opens.
encoded_url="sqlite://$work/encoded/room%3Farchive.db?mode=rwc"
DATABASE_URL="$encoded_url" ARTIFACT_LOCALFS_ROOT="$work/restored-artifacts-4" \
  bash "$here/restore.sh" "$work/backup" >/dev/null
[[ -f "$work/encoded/room?archive.db" ]] || fail "restore.sh did not decode a percent-encoded path"
check_restored "$work/encoded/room?archive.db" "a percent-encoded path"
DATABASE_URL="$encoded_url" bash "$here/backup.sh" "$work/backup-encoded" >/dev/null 2>&1 \
  || fail "backup.sh did not decode a percent-encoded path"
[[ "$(sqlite3 "$work/backup-encoded/maidan.sqlite" "SELECT count(*) FROM events")" == "$snap_rows" ]] \
  || fail "backup of a percent-encoded path"

# 5. A decoded `%0A` at the end is part of the name. A command substitution
# would drop it and land on the sibling without the newline.
mkdir -p "$work/newline"
echo "the sibling" > "$work/newline/room.db"
newline_db="$work/newline/room.db"$'\n'
DATABASE_URL="sqlite://$work/newline/room.db%0A" ARTIFACT_LOCALFS_ROOT="$work/restored-artifacts-5" \
  bash "$here/restore.sh" "$work/backup" >/dev/null
[[ "$(cat "$work/newline/room.db")" == "the sibling" ]] || fail "restore.sh wrote the sibling of a newline-ending path"
check_restored "$newline_db" "a newline-ending path"
DATABASE_URL="sqlite://$work/newline/room.db%0A" bash "$here/backup.sh" "$work/backup-newline" >/dev/null 2>&1 \
  || fail "backup.sh did not read a newline-ending path"
[[ "$(sqlite3 "$work/backup-newline/maidan.sqlite" "SELECT count(*) FROM events")" == "$snap_rows" ]] \
  || fail "backup of a newline-ending path"

# 6. A symlink to the database: the restore takes the mode of the file it points
# at, not the link's (777 on Linux), which would open the database to everyone.
mkdir -p "$work/linked/real"
echo "not a database" > "$work/linked/real/maidan.db"
chmod 600 "$work/linked/real/maidan.db"
ln -s "$work/linked/real/maidan.db" "$work/linked/maidan.db"
DATABASE_URL="sqlite://$work/linked/maidan.db" ARTIFACT_LOCALFS_ROOT="$work/restored-artifacts-6" \
  bash "$here/restore.sh" "$work/backup" --force >/dev/null
check_restored "$work/linked/maidan.db" "--force through a symlink"
mode="$(stat -c '%a' "$work/linked/maidan.db" 2>/dev/null || stat -f '%Lp' "$work/linked/maidan.db")"
[[ "$mode" == 600 ]] || fail "a restore through a symlink took the link's mode ($mode), not the database's (600)"

echo "sqlite-backup-drill: ok ($snap_rows rows snapshotted mid-write of $live_rows, restored exactly)"
