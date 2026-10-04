//! Parse a git diff and apply it to the files it touches, without a checkout.
//!
//! The change flow commits through GitHub's Git Data API, so there is no
//! working tree to run `git apply` in. This module does the part of `git
//! apply` the flow needs: it reads the unified diff a coding seat produced,
//! names the paths whose base content must be fetched, and applies each file's
//! hunks to that content. Anything it cannot apply exactly is refused with a
//! reason rather than guessed at: a commit that differs from the reviewed diff
//! is worse than no commit.
//!
//! Supported: modified, added, deleted and renamed text files, mode changes
//! between `100644` and `100755`, `\ No newline at end of file`, and hunks that
//! moved by some lines (git's offset search; no fuzz). Refused: binary patches,
//! copies, symlinks, submodules, quoted (C-escaped) paths, and any path that
//! leaves the repository or enters `.git`.

use std::collections::HashSet;

/// Why a diff was refused. The text is recorded on the delivery row and said
/// in Slack, so it names the file and hunk.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct PatchError(pub String);

fn refuse<T>(msg: impl Into<String>) -> Result<T, PatchError> {
    Err(PatchError(msg.into()))
}

/// The regular-file mode git uses when the diff does not say.
pub const MODE_FILE: &str = "100644";
const MODE_EXECUTABLE: &str = "100755";

/// One file's part of the diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePatch {
    /// `None` for a new file.
    pub old_path: Option<String>,
    /// `None` for a deleted file.
    pub new_path: Option<String>,
    /// The mode the new path is written with.
    pub mode: String,
    hunks: Vec<Hunk>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Hunk {
    old_start: usize,
    old: Vec<Vec<u8>>,
    new: Vec<Vec<u8>>,
}

/// One tree change the commit is built from. `content: None` deletes `path`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeChange {
    pub path: String,
    pub mode: String,
    pub content: Option<Vec<u8>>,
}

impl FilePatch {
    /// Apply this file's hunks to `old` (the file at the base commit; `None`
    /// when it does not exist there) and return the tree changes it makes.
    pub fn apply(&self, old: Option<&[u8]>) -> Result<Vec<TreeChange>, PatchError> {
        let shown = self
            .new_path
            .as_deref()
            .or(self.old_path.as_deref())
            .unwrap_or_default();
        let base: &[u8] = match (&self.old_path, old) {
            (Some(_), Some(bytes)) => bytes,
            (Some(path), None) => {
                return refuse(format!("{path} does not exist at the base commit"))
            }
            (None, Some(_)) => return refuse(format!("{shown} already exists at the base commit")),
            (None, None) => &[],
        };
        let patched =
            apply_hunks(base, &self.hunks).map_err(|e| PatchError(format!("{shown}: {}", e.0)))?;
        let mut changes = Vec::new();
        match (&self.old_path, &self.new_path) {
            (Some(_), None) => {
                if !patched.is_empty() {
                    return refuse(format!(
                        "{shown}: the diff deletes the file but does not remove all of it"
                    ));
                }
                changes.push(TreeChange {
                    path: shown.to_string(),
                    mode: self.mode.clone(),
                    content: None,
                });
            }
            (old_path, Some(new_path)) => {
                if let Some(old_path) = old_path.as_ref().filter(|p| *p != new_path) {
                    changes.push(TreeChange {
                        path: old_path.clone(),
                        mode: self.mode.clone(),
                        content: None,
                    });
                }
                changes.push(TreeChange {
                    path: new_path.clone(),
                    mode: self.mode.clone(),
                    content: Some(patched),
                });
            }
            (None, None) => return refuse("a file patch with neither an old nor a new path"),
        }
        Ok(changes)
    }
}

fn split_lines(bytes: &[u8]) -> Vec<&[u8]> {
    bytes.split_inclusive(|b| *b == b'\n').collect()
}

fn apply_hunks(base: &[u8], hunks: &[Hunk]) -> Result<Vec<u8>, PatchError> {
    let lines = split_lines(base);
    let mut out: Vec<u8> = Vec::with_capacity(base.len());
    let mut pos = 0usize;
    for (n, hunk) in hunks.iter().enumerate() {
        // A hunk with no old lines (`git diff -U0`) inserts *after* line
        // `old_start` (`-0,0` is before the first line). Nothing anchors an
        // empty old side, so it goes exactly there or nowhere.
        let at = if hunk.old.is_empty() {
            if hunk.old_start < pos || hunk.old_start > lines.len() {
                return refuse(format!(
                    "hunk {} inserts after line {}, outside the base content left to patch",
                    n + 1,
                    hunk.old_start
                ));
            }
            hunk.old_start
        } else {
            let wanted = hunk.old_start.saturating_sub(1).max(pos);
            find_block(&lines, &hunk.old, wanted, pos).ok_or_else(|| {
                PatchError(format!(
                    "hunk {} (at line {}) does not match the base content",
                    n + 1,
                    hunk.old_start
                ))
            })?
        };
        for line in &lines[pos..at] {
            out.extend_from_slice(line);
        }
        for line in &hunk.new {
            out.extend_from_slice(line);
        }
        pos = at + hunk.old.len();
    }
    for line in &lines[pos..] {
        out.extend_from_slice(line);
    }
    Ok(out)
}

/// The first position at or after `floor`, nearest to `wanted`, where `block`
/// matches `lines` exactly.
fn find_block(lines: &[&[u8]], block: &[Vec<u8>], wanted: usize, floor: usize) -> Option<usize> {
    let fits = |at: usize| {
        at >= floor
            && at + block.len() <= lines.len()
            && block
                .iter()
                .zip(&lines[at..])
                .all(|(a, b)| a.as_slice() == *b)
    };
    let span = lines.len().max(wanted) + 1;
    (0..=span).find_map(|offset| {
        [wanted.checked_add(offset), wanted.checked_sub(offset)]
            .into_iter()
            .flatten()
            .find(|at| fits(*at))
    })
}

/// Parse a git (or plain unified) diff into per-file patches.
pub fn parse(diff: &str) -> Result<Vec<FilePatch>, PatchError> {
    let lines: Vec<&str> = diff
        .split_inclusive('\n')
        .map(|l| l.strip_suffix('\n').unwrap_or(l))
        .collect();
    let mut patches = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.starts_with("diff --git ") || line.starts_with("--- ") {
            let (patch, next) = parse_file(&lines, i)?;
            patches.push(patch);
            i = next;
        } else {
            i += 1;
        }
    }
    if patches.is_empty() {
        return refuse("the diff touches no files");
    }
    let mut seen = HashSet::new();
    for patch in &patches {
        for path in [&patch.old_path, &patch.new_path].into_iter().flatten() {
            check_path(path)?;
        }
        let key = patch.new_path.as_ref().or(patch.old_path.as_ref());
        if let Some(key) = key {
            if !seen.insert(key.clone()) {
                return refuse(format!("{key} appears twice in the diff"));
            }
        }
    }
    Ok(patches)
}

fn check_path(path: &str) -> Result<(), PatchError> {
    let bad = path.is_empty()
        || path.starts_with('/')
        || path.contains('\0')
        || path.split('/').any(|part| {
            part.is_empty() || part == "." || part == ".." || part.eq_ignore_ascii_case(".git")
        });
    if bad {
        return refuse(format!("refusing path {path:?}"));
    }
    Ok(())
}

fn strip_prefix_path(raw: &str) -> Result<Option<String>, PatchError> {
    // A plain unified diff may follow the name with a tab and a timestamp.
    let raw = raw.split('\t').next().unwrap_or(raw);
    if raw == "/dev/null" {
        return Ok(None);
    }
    if raw.starts_with('"') {
        return refuse(format!("quoted paths are not supported: {raw}"));
    }
    let path = raw
        .strip_prefix("a/")
        .or_else(|| raw.strip_prefix("b/"))
        .unwrap_or(raw);
    Ok(Some(path.to_string()))
}

/// `diff --git a/P b/P` names the path only when both sides are equal, which
/// is the case this header is needed for (no `---`/`+++`: an empty new file, a
/// mode change).
fn git_header_path(line: &str) -> Option<String> {
    let rest = line.strip_prefix("diff --git a/")?;
    if rest.len() < 3 || (rest.len() - 3) % 2 != 0 {
        return None;
    }
    let half = (rest.len() - 3) / 2;
    let (a, b) = (rest.get(..half)?, rest.get(half..)?);
    (b.strip_prefix(" b/") == Some(a)).then(|| a.to_string())
}

fn check_mode(mode: &str) -> Result<String, PatchError> {
    match mode {
        MODE_FILE | MODE_EXECUTABLE => Ok(mode.to_string()),
        "120000" => refuse("symlinks are not supported"),
        "160000" => refuse("submodules are not supported"),
        other => refuse(format!("unsupported file mode {other}")),
    }
}

fn parse_file(lines: &[&str], start: usize) -> Result<(FilePatch, usize), PatchError> {
    let mut i = start;
    let mut header_path = None;
    let mut old_path: Option<Option<String>> = None;
    let mut new_path: Option<Option<String>> = None;
    let mut rename_from = None;
    let mut rename_to = None;
    let mut mode = None;
    let mut is_new = false;
    let mut is_deleted = false;
    if let Some(line) = lines.get(i).filter(|l| l.starts_with("diff --git ")) {
        if line.contains(" \"") {
            return refuse(format!("quoted paths are not supported: {line}"));
        }
        header_path = git_header_path(line);
        i += 1;
        while let Some(line) = lines.get(i) {
            if line.starts_with("diff --git ") || line.starts_with("--- ") || line.starts_with("@@")
            {
                break;
            }
            if let Some(m) = line.strip_prefix("new file mode ") {
                is_new = true;
                mode = Some(check_mode(m.trim())?);
            } else if let Some(m) = line.strip_prefix("deleted file mode ") {
                is_deleted = true;
                mode = Some(check_mode(m.trim())?);
            } else if let Some(m) = line.strip_prefix("new mode ") {
                mode = Some(check_mode(m.trim())?);
            } else if let Some(m) = line.strip_prefix("old mode ") {
                check_mode(m.trim())?;
            } else if let Some(rest) = line.strip_prefix("index ") {
                if let Some(m) = rest.split_whitespace().nth(1) {
                    mode = Some(check_mode(m)?);
                }
            } else if let Some(p) = line.strip_prefix("rename from ") {
                rename_from = Some(p.to_string());
            } else if let Some(p) = line.strip_prefix("rename to ") {
                rename_to = Some(p.to_string());
            } else if line.starts_with("copy from ") || line.starts_with("copy to ") {
                return refuse("copies are not supported");
            } else if line.starts_with("GIT binary patch") || line.starts_with("Binary files ") {
                return refuse("binary patches are not supported");
            }
            i += 1;
        }
    }
    if let Some(line) = lines.get(i).filter(|l| l.starts_with("--- ")) {
        old_path = Some(strip_prefix_path(&line[4..])?);
        i += 1;
        let Some(plus) = lines.get(i).and_then(|l| l.strip_prefix("+++ ")) else {
            return refuse("a `---` line without its `+++` line");
        };
        new_path = Some(strip_prefix_path(plus)?);
        i += 1;
    }
    let mut hunks = Vec::new();
    while let Some(line) = lines.get(i) {
        if !line.starts_with("@@") {
            break;
        }
        let (hunk, next) = parse_hunk(lines, i)?;
        hunks.push(hunk);
        i = next;
    }
    let old = match (rename_from, old_path) {
        (Some(from), _) => Some(from),
        (None, Some(p)) => p,
        (None, None) if is_new => None,
        (None, None) => header_path.clone(),
    };
    let new = match (rename_to, new_path) {
        (Some(to), _) => Some(to),
        (None, Some(p)) => p,
        (None, None) if is_deleted => None,
        (None, None) => header_path,
    };
    let old = if is_new { None } else { old };
    let new = if is_deleted { None } else { new };
    if old.is_none() && new.is_none() {
        return refuse("a file in the diff has no path");
    }
    Ok((
        FilePatch {
            old_path: old,
            new_path: new,
            mode: mode.unwrap_or_else(|| MODE_FILE.to_string()),
            hunks,
        },
        i,
    ))
}

fn parse_range(s: &str) -> Option<(usize, usize)> {
    match s.split_once(',') {
        Some((start, len)) => Some((start.parse().ok()?, len.parse().ok()?)),
        None => Some((s.parse().ok()?, 1)),
    }
}

fn parse_hunk(lines: &[&str], start: usize) -> Result<(Hunk, usize), PatchError> {
    let header = lines[start];
    let bad = || PatchError(format!("malformed hunk header: {header}"));
    let mut parts = header.split_whitespace().skip(1);
    let old = parts
        .next()
        .and_then(|p| p.strip_prefix('-'))
        .ok_or_else(bad)?;
    let new = parts
        .next()
        .and_then(|p| p.strip_prefix('+'))
        .ok_or_else(bad)?;
    let (old_start, mut old_left) = parse_range(old).ok_or_else(bad)?;
    let (_, mut new_left) = parse_range(new).ok_or_else(bad)?;
    let mut hunk = Hunk {
        old_start,
        old: Vec::new(),
        new: Vec::new(),
    };
    // Which side(s) the last line went to, for `\ No newline at end of file`.
    let mut last = (false, false);
    let mut i = start + 1;
    while let Some(line) = lines.get(i) {
        if let Some(rest) = line.strip_prefix('\\') {
            if !rest.trim_start().starts_with("No newline") {
                return refuse(format!("unexpected line in hunk: {line}"));
            }
            if last.0 {
                strip_newline(hunk.old.last_mut());
            }
            if last.1 {
                strip_newline(hunk.new.last_mut());
            }
            i += 1;
            continue;
        }
        if old_left == 0 && new_left == 0 {
            break;
        }
        // Some tools drop the space of an empty context line.
        let (tag, body) = match line.chars().next() {
            None => (' ', ""),
            Some(c) => (c, &line[c.len_utf8()..]),
        };
        let mut text = body.as_bytes().to_vec();
        text.push(b'\n');
        match tag {
            ' ' if old_left > 0 && new_left > 0 => {
                hunk.old.push(text.clone());
                hunk.new.push(text);
                old_left -= 1;
                new_left -= 1;
                last = (true, true);
            }
            '-' if old_left > 0 => {
                hunk.old.push(text);
                old_left -= 1;
                last = (true, false);
            }
            '+' if new_left > 0 => {
                hunk.new.push(text);
                new_left -= 1;
                last = (false, true);
            }
            _ => return refuse(format!("hunk {header} has the wrong number of lines")),
        }
        i += 1;
    }
    if old_left != 0 || new_left != 0 {
        return refuse(format!("hunk {header} ends early"));
    }
    Ok((hunk, i))
}

fn strip_newline(line: Option<&mut Vec<u8>>) {
    if let Some(line) = line {
        if line.last() == Some(&b'\n') {
            line.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply_one(diff: &str, old: Option<&str>) -> Result<Vec<TreeChange>, PatchError> {
        let patches = parse(diff)?;
        assert_eq!(patches.len(), 1);
        patches[0].apply(old.map(str::as_bytes))
    }

    fn text(change: &TreeChange) -> &str {
        std::str::from_utf8(change.content.as_deref().unwrap()).unwrap()
    }

    #[test]
    fn a_modification_applies_with_its_mode_from_the_index_line() {
        let diff = "diff --git a/src/a.txt b/src/a.txt\nindex 1111111..2222222 100755\n--- a/src/a.txt\n+++ b/src/a.txt\n@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n";
        let changes = apply_one(diff, Some("one\ntwo\nthree\n")).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "src/a.txt");
        assert_eq!(changes[0].mode, "100755");
        assert_eq!(text(&changes[0]), "one\nTWO\nthree\n");
    }

    #[test]
    fn a_hunk_that_moved_still_applies_but_changed_context_does_not() {
        let diff = "--- a/a.txt\n+++ b/a.txt\n@@ -1,2 +1,2 @@\n x\n-y\n+Y\n";
        let moved = apply_one(diff, Some("new\nlines\nx\ny\n")).unwrap();
        assert_eq!(text(&moved[0]), "new\nlines\nx\nY\n");
        let err = apply_one(diff, Some("x\nz\n")).unwrap_err();
        assert!(err.0.contains("does not match"), "{err}");
    }

    #[test]
    fn new_deleted_and_renamed_files_become_tree_changes() {
        let new = "diff --git a/n.sh b/n.sh\nnew file mode 100755\nindex 0000000..1111111\n--- /dev/null\n+++ b/n.sh\n@@ -0,0 +1,2 @@\n+#!/bin/sh\n+echo hi\n";
        let changes = apply_one(new, None).unwrap();
        assert_eq!(changes[0].mode, "100755");
        assert_eq!(text(&changes[0]), "#!/bin/sh\necho hi\n");
        assert!(
            apply_one(new, Some("x\n")).is_err(),
            "a new file that exists is refused"
        );

        let gone = "diff --git a/d.txt b/d.txt\ndeleted file mode 100644\nindex 1111111..0000000\n--- a/d.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-bye\n";
        let changes = apply_one(gone, Some("bye\n")).unwrap();
        assert_eq!(changes[0].content, None);
        assert!(apply_one(gone, Some("bye\nmore\n")).is_err());

        let renamed = "diff --git a/old.txt b/new.txt\nsimilarity index 90%\nrename from old.txt\nrename to new.txt\nindex 1111111..2222222 100644\n--- a/old.txt\n+++ b/new.txt\n@@ -1 +1 @@\n-a\n+b\n";
        let changes = apply_one(renamed, Some("a\n")).unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(
            (changes[0].path.as_str(), changes[0].content.as_ref()),
            ("old.txt", None)
        );
        assert_eq!(changes[1].path, "new.txt");
        assert_eq!(text(&changes[1]), "b\n");
    }

    #[test]
    fn a_rename_sent_as_a_delete_plus_an_add_applies_as_two_changes() {
        let diff = "diff --git a/old.txt b/old.txt\ndeleted file mode 100644\nindex 1111111..0000000\n--- a/old.txt\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-a\n-b\ndiff --git a/new/place.txt b/new/place.txt\nnew file mode 100644\nindex 0000000..2222222\n--- /dev/null\n+++ b/new/place.txt\n@@ -0,0 +1,2 @@\n+a\n+B\n";
        let patches = parse(diff).unwrap();
        assert_eq!(patches.len(), 2);
        let gone = patches[0].apply(Some(b"a\nb\n")).unwrap();
        assert_eq!(
            (gone[0].path.as_str(), gone[0].content.as_ref()),
            ("old.txt", None)
        );
        let added = patches[1].apply(None).unwrap();
        assert_eq!(added[0].path, "new/place.txt");
        assert_eq!(text(&added[0]), "a\nB\n");
    }

    #[test]
    fn a_context_free_insertion_lands_after_its_line() {
        let diff = "--- a/f\n+++ b/f\n@@ -2,0 +3 @@\n+x\n";
        let changes = apply_one(diff, Some("1\n2\n3\n")).unwrap();
        assert_eq!(text(&changes[0]), "1\n2\nx\n3\n");
        let at_top = "--- a/f\n+++ b/f\n@@ -0,0 +1 @@\n+x\n";
        let changes = apply_one(at_top, Some("1\n2\n")).unwrap();
        assert_eq!(text(&changes[0]), "x\n1\n2\n");
        let past_end = "--- a/f\n+++ b/f\n@@ -9,0 +10 @@\n+x\n";
        assert!(apply_one(past_end, Some("1\n2\n")).is_err());
    }

    #[test]
    fn a_missing_trailing_newline_is_honoured_on_both_sides() {
        let diff = "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n\\ No newline at end of file\n+b\n";
        let changes = apply_one(diff, Some("a")).unwrap();
        assert_eq!(text(&changes[0]), "b\n");
        assert!(
            apply_one(diff, Some("a\n")).is_err(),
            "the base had a newline the diff says it lacks"
        );
    }

    #[test]
    fn unsupported_and_unsafe_diffs_are_refused() {
        for diff in [
            "",
            "not a diff\n",
            "diff --git a/b.png b/b.png\nindex 1..2 100644\nBinary files a/b.png and b/b.png differ\n",
            "diff --git a/l b/l\nnew file mode 120000\n--- /dev/null\n+++ b/l\n@@ -0,0 +1 @@\n+target\n",
            "--- a/../etc/passwd\n+++ b/../etc/passwd\n@@ -1 +1 @@\n-a\n+b\n",
            "--- a/.git/config\n+++ b/.git/config\n@@ -1 +1 @@\n-a\n+b\n",
            "--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@\n-a\n+b\n",
            "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n",
        ] {
            assert!(parse(diff).is_err(), "expected {diff:?} to be refused");
        }
    }

    #[test]
    fn several_files_and_hunks_parse_in_order() {
        let diff = "diff --git a/a b/a\nindex 1..2 100644\n--- a/a\n+++ b/a\n@@ -1,2 +1,2 @@\n-1\n+one\n 2\n@@ -5,2 +5,2 @@\n 5\n-6\n+six\ndiff --git a/b b/b\nindex 1..2 100644\n--- a/b\n+++ b/b\n@@ -1 +1 @@\n-x\n+y\n";
        let patches = parse(diff).unwrap();
        assert_eq!(patches.len(), 2);
        let a = patches[0].apply(Some(b"1\n2\n3\n4\n5\n6\n7\n")).unwrap();
        assert_eq!(text(&a[0]), "one\n2\n3\n4\n5\nsix\n7\n");
        assert_eq!(patches[1].new_path.as_deref(), Some("b"));
    }
}
