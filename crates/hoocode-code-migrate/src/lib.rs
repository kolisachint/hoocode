//! The one-time merge of the old `~/.cortexcode` and `<repo>/.cortexcode/`
//! folders into `~/.hoocode` and `<repo>/.hoocode/` (naming-and-paths.md §3).
//!
//! Copy, never move or delete. `.cortexcode` wins a conflict; every file that
//! changes is backed up first. A marker stops a second merge, and a lock
//! directory stops two processes merging at once. Nothing else reads the old
//! folders after this runs.

use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hoocode_code_auth::{AuthStorageBackend, FileAuthStorageBackend};
use hoocode_code_paths::{home_dir, CONFIG_DIR_NAME};
use serde_json::{Map, Value};

/// The folder the merge reads (the name hoocode used before 1.2).
pub const OLD_DIR_NAME: &str = ".cortexcode";
/// Written into the destination when a merge completes; its presence stops a re-run.
pub const MARKER_NAME: &str = ".merged-from-cortexcode";
const LOCK_NAME: &str = ".merge.lock";
const BACKUP_PREFIX: &str = "backup-cortexcode-";
const REPORT_PREFIX: &str = "merge-report-";
const AUTH_FILE: &str = "auth.json";
/// Top-level JSON files merged key by key (`.cortexcode` wins a leaf conflict).
const DEEP_MERGE_FILES: [&str; 4] = [
    "settings.json",
    "models.json",
    "keybindings.json",
    "hoo-config.json",
];
/// Home: top-level entries that are not merged, with the reason shown in the report.
pub const HOME_SKIP: &[(&str, &str)] = &[
    ("bin", "re-downloaded, not merged"),
    ("cache", "regenerated, not merged"),
    ("embsearch", "regenerated, not merged"),
    ("cortex-debug.log", "debug log, not merged"),
];
/// Project: top-level entries that are not merged.
pub const PROJECT_SKIP: &[(&str, &str)] = &[("dispatch", "per-run subagent state, not merged")];

const LOCK_ATTEMPTS: u32 = 100;
const LOCK_DELAY: Duration = Duration::from_millis(100);
const LOCK_STALE: Duration = Duration::from_secs(120);

/// How one merge runs.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Top-level names left out, each with the reason the report shows.
    pub skip: &'static [(&'static str, &'static str)],
    /// Plan only: nothing is written (no backups, marker, report or lock).
    pub dry_run: bool,
    /// Also save the report as `merge-report-<stamp>.txt` in the destination.
    pub save_report: bool,
}

/// What one merge did to one file (or one provider in `auth.json`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// A new file, or keys added to an existing JSON file. `backup` is the
    /// copy of the file as it was, when one existed.
    Added {
        detail: String,
        backup: Option<PathBuf>,
    },
    /// An existing value was replaced; `backup` holds the file as it was.
    Replaced { detail: String, backup: PathBuf },
    /// Already the same in both folders.
    Unchanged,
    /// Left alone; `reason` says why.
    Skipped { reason: String },
}

/// One line of the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Relative to the source folder, `/`-separated; directories end in `/`.
    pub path: String,
    pub action: Action,
}

/// The result of a merge that ran (or would run, for a dry run).
#[derive(Debug, Clone)]
pub struct Outcome {
    pub source: PathBuf,
    pub dest: PathBuf,
    pub stamp: String,
    pub dry_run: bool,
    pub entries: Vec<Entry>,
}

/// Counts per kind of action, for summaries.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub added: usize,
    pub replaced: usize,
    pub unchanged: usize,
    pub skipped: usize,
}

impl Outcome {
    pub fn counts(&self) -> Counts {
        let mut c = Counts::default();
        for entry in &self.entries {
            match entry.action {
                Action::Added { .. } => c.added += 1,
                Action::Replaced { .. } => c.replaced += 1,
                Action::Unchanged => c.unchanged += 1,
                Action::Skipped { .. } => c.skipped += 1,
            }
        }
        c
    }

    /// Where this merge's backups went (or would go).
    pub fn backup_dir(&self) -> PathBuf {
        self.dest.join(format!("{BACKUP_PREFIX}{}", self.stamp))
    }

    /// The report: one line per file, printed and (for home) saved.
    pub fn report(&self) -> String {
        let c = self.counts();
        let mut out = format!(
            "merge {} -> {} ({}){}\n",
            self.source.display(),
            self.dest.display(),
            self.stamp,
            if self.dry_run {
                " [dry run: nothing written]"
            } else {
                ""
            }
        );
        out.push_str(&format!(
            "  added {}, overwritten {}, unchanged {}, skipped {}\n",
            c.added, c.replaced, c.unchanged, c.skipped
        ));
        for entry in &self.entries {
            let line = match &entry.action {
                Action::Added { detail, backup } => match backup {
                    Some(b) => format!(
                        "  added       {}: {detail}; backup {}",
                        entry.path,
                        b.display()
                    ),
                    None => format!("  added       {}: {detail}", entry.path),
                },
                Action::Replaced { detail, backup } => format!(
                    "  overwritten {}: {detail}; backup {}",
                    entry.path,
                    backup.display()
                ),
                Action::Unchanged => format!("  unchanged   {}", entry.path),
                Action::Skipped { reason } => format!("  skipped     {}: {reason}", entry.path),
            };
            out.push_str(&line);
            out.push('\n');
        }
        out
    }
}

/// What [`merge_dirs`] found to do.
#[derive(Debug)]
pub enum MergeResult {
    /// The source folder does not exist: nothing to merge.
    NoSource,
    /// A merge already ran into this destination; `marker` says so.
    AlreadyMerged {
        marker: PathBuf,
    },
    Merged(Outcome),
}

/// Merges `source` into `dest` by the rules in naming-and-paths.md §3.
pub fn merge_dirs(source: &Path, dest: &Path, opts: &Options) -> Result<MergeResult, String> {
    if !source.is_dir() {
        return Ok(MergeResult::NoSource);
    }
    let marker = dest.join(MARKER_NAME);
    if marker.exists() {
        return Ok(MergeResult::AlreadyMerged { marker });
    }
    let stamp = stamp_now();
    let ctx = Ctx {
        source,
        dest,
        stamp: &stamp,
        dry_run: opts.dry_run,
    };
    if opts.dry_run {
        let entries = run_entries(&ctx, opts.skip)?;
        return Ok(MergeResult::Merged(ctx.outcome(entries)));
    }
    std::fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let _lock = DirLock::acquire(dest)?;
    if marker.exists() {
        return Ok(MergeResult::AlreadyMerged { marker });
    }
    let entries = run_entries(&ctx, opts.skip)?;
    put(
        &ctx,
        &marker,
        format!("merged from {} at {stamp}\n", source.display()).as_bytes(),
    )?;
    let outcome = ctx.outcome(entries);
    if opts.save_report {
        put(
            &ctx,
            &dest.join(format!("{REPORT_PREFIX}{stamp}.txt")),
            outcome.report().as_bytes(),
        )?;
    }
    Ok(MergeResult::Merged(outcome))
}

/// The home merge: `~/.cortexcode` into `~/.hoocode`.
pub fn merge_home(dry_run: bool) -> Result<MergeResult, String> {
    let home = home_dir();
    merge_dirs(
        &home.join(OLD_DIR_NAME),
        &home.join(CONFIG_DIR_NAME),
        &Options {
            skip: HOME_SKIP,
            dry_run,
            save_report: true,
        },
    )
}

/// The project merge: `<cwd>/.cortexcode` into `<cwd>/.hoocode`. The home folder
/// is left to [`merge_home`].
pub fn merge_project(cwd: &Path, dry_run: bool) -> Result<MergeResult, String> {
    if cwd == home_dir().as_path() {
        return Ok(MergeResult::NoSource);
    }
    merge_dirs(
        &cwd.join(OLD_DIR_NAME),
        &cwd.join(CONFIG_DIR_NAME),
        &Options {
            skip: PROJECT_SKIP,
            dry_run,
            save_report: false,
        },
    )
}

/// Run at every start, in every mode. Notices go to `err`; a failed merge never
/// stops hoocode from starting.
pub fn run_startup(cwd: &Path, err: &mut dyn Write) {
    match merge_home(false) {
        Ok(MergeResult::Merged(outcome)) => {
            let _ = write!(err, "{}", outcome.report());
            let _ = writeln!(
                err,
                "hoocode: merged ~/{OLD_DIR_NAME} into ~/{CONFIG_DIR_NAME}; backups in {}",
                outcome.backup_dir().display()
            );
        }
        Ok(_) => {}
        Err(e) => {
            let _ = writeln!(
                err,
                "hoocode: merge of ~/{OLD_DIR_NAME} failed ({e}); continuing without it"
            );
        }
    }
    match merge_project(cwd, false) {
        Ok(MergeResult::Merged(outcome)) => {
            let _ = writeln!(
                err,
                "hoocode: merged `{OLD_DIR_NAME}/` into `{CONFIG_DIR_NAME}/`; review with `git status`; \
                 add `{CONFIG_DIR_NAME}/dispatch/` and `{CONFIG_DIR_NAME}/backup-*/` to `.gitignore`"
            );
            let _ = writeln!(
                err,
                "  {} added, {} overwritten (backups, if any, in {})",
                outcome.counts().added,
                outcome.counts().replaced,
                outcome.backup_dir().display()
            );
        }
        Ok(_) => {}
        Err(e) => {
            let _ = writeln!(
                err,
                "hoocode: merge of `{OLD_DIR_NAME}/` failed ({e}); continuing without it"
            );
        }
    }
}

/// `hoocode migrate [--dry-run]`: runs both merges now and prints what happened.
/// Returns the exit code.
pub fn run_migrate_command(
    args: &[String],
    cwd: &Path,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let dry_run = match args {
        [] => false,
        [flag] if flag == "--dry-run" => true,
        _ => {
            let _ = writeln!(err, "usage: hoocode migrate [--dry-run]");
            return 2;
        }
    };
    let mut code = 0;
    let runs = [
        ("~/".to_string() + OLD_DIR_NAME, merge_home(dry_run)),
        (
            format!("{}/", cwd.join(OLD_DIR_NAME).display()),
            merge_project(cwd, dry_run),
        ),
    ];
    for (label, result) in runs {
        let text = match result {
            Ok(MergeResult::NoSource) => format!("{label}: nothing to merge\n"),
            Ok(MergeResult::AlreadyMerged { marker }) => {
                format!("{label}: already merged (see {})\n", marker.display())
            }
            Ok(MergeResult::Merged(outcome)) => outcome.report(),
            Err(e) => {
                code = 1;
                format!("{label}: merge failed: {e}\n")
            }
        };
        let _ = write!(out, "{text}");
    }
    code
}

// ---------------------------------------------------------------------------
// The merge itself
// ---------------------------------------------------------------------------

struct Ctx<'a> {
    source: &'a Path,
    dest: &'a Path,
    stamp: &'a str,
    dry_run: bool,
}

impl Ctx<'_> {
    fn outcome(&self, entries: Vec<Entry>) -> Outcome {
        Outcome {
            source: self.source.to_path_buf(),
            dest: self.dest.to_path_buf(),
            stamp: self.stamp.to_string(),
            dry_run: self.dry_run,
            entries,
        }
    }
}

/// Every file to merge, then each one's action. Writes unless it is a dry run.
fn run_entries(ctx: &Ctx, skip: &[(&str, &str)]) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for (name, reason) in skip {
        let path = ctx.source.join(name);
        if path.exists() {
            let shown = if path.is_dir() {
                format!("{name}/")
            } else {
                name.to_string()
            };
            entries.push(Entry {
                path: shown,
                action: Action::Skipped {
                    reason: reason.to_string(),
                },
            });
        }
    }
    let mut files = Vec::new();
    collect(ctx.source, Path::new(""), skip, &mut files)
        .map_err(|e| format!("{}: {e}", ctx.source.display()))?;
    for rel in files {
        let action = merge_file(ctx, &rel)?;
        entries.push(Entry {
            path: rel
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/"),
            action,
        });
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

/// Regular files under `dir`, skipping the top-level names in `skip` and the
/// merge's own bookkeeping.
fn collect(
    dir: &Path,
    rel: &Path,
    skip: &[(&str, &str)],
    out: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    let mut children: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    children.sort_by_key(|e| e.file_name());
    for child in children {
        let name = child.file_name();
        let name_str = name.to_string_lossy().into_owned();
        if rel.as_os_str().is_empty() && skip.iter().any(|(s, _)| *s == name_str) {
            continue;
        }
        if name_str == MARKER_NAME
            || name_str == LOCK_NAME
            || name_str.starts_with(BACKUP_PREFIX)
            || name_str.starts_with(REPORT_PREFIX)
        {
            continue;
        }
        let path = child.path();
        let child_rel = rel.join(&name);
        let meta = std::fs::metadata(&path)?;
        if meta.is_dir() {
            collect(&path, &child_rel, skip, out)?;
        } else if meta.is_file() {
            out.push(child_rel);
        }
    }
    Ok(())
}

fn merge_file(ctx: &Ctx, rel: &Path) -> Result<Action, String> {
    let src = ctx.source.join(rel);
    let dst = ctx.dest.join(rel);
    let top_level = rel.components().count() == 1;
    let name = rel.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    if top_level && name == AUTH_FILE {
        return merge_auth(ctx, rel, &src, &dst);
    }
    if top_level && DEEP_MERGE_FILES.contains(&name) {
        return merge_json(ctx, rel, &src, &dst);
    }
    copy_file(ctx, rel, &src, &dst)
}

/// Any other file: copied; a different existing file is backed up and replaced.
fn copy_file(ctx: &Ctx, rel: &Path, src: &Path, dst: &Path) -> Result<Action, String> {
    let new = match std::fs::read(src) {
        Ok(bytes) => bytes,
        Err(e) => {
            return Ok(Action::Skipped {
                reason: format!("unreadable ({e})"),
            })
        }
    };
    match std::fs::read(dst) {
        Ok(old) if old == new => Ok(Action::Unchanged),
        Ok(old) => {
            let backup = back_up(ctx, rel, &old)?;
            put(ctx, dst, &new)?;
            Ok(Action::Replaced {
                detail: "file replaced".into(),
                backup,
            })
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            put(ctx, dst, &new)?;
            Ok(Action::Added {
                detail: "new file".into(),
                backup: None,
            })
        }
        Err(e) => Ok(Action::Skipped {
            reason: format!("destination unreadable ({e})"),
        }),
    }
}

/// A JSON settings file: keys merged, `.cortexcode` winning each leaf, arrays
/// replaced whole. Keys only in the destination stay.
fn merge_json(ctx: &Ctx, rel: &Path, src: &Path, dst: &Path) -> Result<Action, String> {
    let src_value: Value = match read_json(src) {
        Ok(v) => v,
        Err(reason) => {
            return Ok(Action::Skipped {
                reason: format!("source is not valid JSON, left alone ({reason})"),
            })
        }
    };
    let old = match std::fs::read(dst) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == ErrorKind::NotFound => return copy_file(ctx, rel, src, dst),
        Err(e) => {
            return Ok(Action::Skipped {
                reason: format!("destination unreadable ({e})"),
            })
        }
    };
    let mut dst_value: Value = match serde_json::from_slice(&old) {
        Ok(v) => v,
        Err(e) => {
            return Ok(Action::Skipped {
                reason: format!("destination is not valid JSON, left alone ({e})"),
            })
        }
    };
    let mut stats = Stats::default();
    deep_merge(&mut dst_value, &src_value, &mut stats);
    if stats.total() == 0 {
        return Ok(Action::Unchanged);
    }
    let text = pretty(&dst_value)?;
    let backup = back_up(ctx, rel, &old)?;
    put(ctx, dst, text.as_bytes())?;
    Ok(change_action(&stats, Some(backup)))
}

/// `auth.json`: per provider, the `.cortexcode` entry replaces the destination's.
/// Written under the same `proper-lockfile` lock hoocode uses for `auth.json`.
fn merge_auth(ctx: &Ctx, rel: &Path, src: &Path, dst: &Path) -> Result<Action, String> {
    let src_map = match read_json(src).and_then(object_or_err) {
        Ok(m) => m,
        Err(reason) => {
            return Ok(Action::Skipped {
                reason: format!("source is not a JSON object, left alone ({reason})"),
            })
        }
    };
    let existed = dst.exists();
    let mut outcome: Option<Action> = None;
    let mut apply = |current: Option<&str>| -> Result<Option<String>, String> {
        let mut map = match current.map(|t| {
            serde_json::from_str::<Value>(t)
                .map_err(|e| e.to_string())
                .and_then(object_or_err)
        }) {
            None => Map::new(),
            Some(Ok(m)) => m,
            Some(Err(reason)) => {
                outcome = Some(Action::Skipped {
                    reason: format!("destination is not a JSON object, left alone ({reason})"),
                });
                return Ok(None);
            }
        };
        let stats = merge_providers(&mut map, &src_map);
        if stats.total() == 0 {
            outcome = Some(Action::Unchanged);
            return Ok(None);
        }
        let backup = match (existed, current) {
            (true, Some(text)) => Some(back_up(ctx, rel, text.as_bytes())?),
            _ => None,
        };
        outcome = Some(change_action(&stats, backup));
        Ok(Some(pretty(&Value::Object(map))?))
    };
    if ctx.dry_run {
        let current = if existed {
            Some(std::fs::read_to_string(dst).map_err(|e| format!("{}: {e}", dst.display()))?)
        } else {
            None
        };
        apply(current.as_deref())?;
    } else {
        FileAuthStorageBackend::new(dst).with_lock(&mut apply)?;
    }
    Ok(outcome.unwrap_or(Action::Unchanged))
}

#[derive(Default)]
struct Stats {
    added: usize,
    replaced: usize,
}

impl Stats {
    fn total(&self) -> usize {
        self.added + self.replaced
    }

    fn detail(&self) -> String {
        format!(
            "{} key(s) added, {} value(s) replaced",
            self.added, self.replaced
        )
    }
}

fn deep_merge(dst: &mut Value, src: &Value, stats: &mut Stats) {
    match (dst, src) {
        (Value::Object(d), Value::Object(s)) => {
            for (key, src_value) in s {
                match d.get_mut(key) {
                    Some(dst_value) => deep_merge(dst_value, src_value, stats),
                    None => {
                        d.insert(key.clone(), src_value.clone());
                        stats.added += 1;
                    }
                }
            }
        }
        (dst, src) => {
            if dst != src {
                *dst = src.clone();
                stats.replaced += 1;
            }
        }
    }
}

fn merge_providers(dst: &mut Map<String, Value>, src: &Map<String, Value>) -> Stats {
    let mut stats = Stats::default();
    for (provider, src_entry) in src {
        match dst.get(provider) {
            None => {
                dst.insert(provider.clone(), src_entry.clone());
                stats.added += 1;
            }
            Some(dst_entry) if dst_entry != src_entry => {
                dst.insert(provider.clone(), src_entry.clone());
                stats.replaced += 1;
            }
            Some(_) => {}
        }
    }
    stats
}

fn change_action(stats: &Stats, backup: Option<PathBuf>) -> Action {
    match (stats.replaced, backup) {
        (replaced, Some(backup)) if replaced > 0 => Action::Replaced {
            detail: stats.detail(),
            backup,
        },
        (_, backup) => Action::Added {
            detail: stats.detail(),
            backup,
        },
    }
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

fn object_or_err(value: Value) -> Result<Map<String, Value>, String> {
    match value {
        Value::Object(map) => Ok(map),
        _ => Err("not an object".into()),
    }
}

fn pretty(value: &Value) -> Result<String, String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    Ok(format!("{text}\n"))
}

/// Copies the file's old bytes under the merge's backup folder, keeping its
/// relative path. Dry run: the path is named, nothing is written.
fn back_up(ctx: &Ctx, rel: &Path, old: &[u8]) -> Result<PathBuf, String> {
    let path = ctx
        .dest
        .join(format!("{BACKUP_PREFIX}{}", ctx.stamp))
        .join(rel);
    put(ctx, &path, old)?;
    Ok(path)
}

/// Writes `bytes` to `path`, creating parent folders. Does nothing on a dry run.
fn put(ctx: &Ctx, path: &Path, bytes: &[u8]) -> Result<(), String> {
    if ctx.dry_run {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// A `mkdir` lock in the destination, released on drop.
struct DirLock(PathBuf);

impl DirLock {
    fn acquire(dest: &Path) -> Result<Self, String> {
        let path = dest.join(LOCK_NAME);
        for _ in 0..LOCK_ATTEMPTS {
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                    if is_stale(&path) {
                        let _ = std::fs::remove_dir(&path);
                    } else {
                        thread::sleep(LOCK_DELAY);
                    }
                }
                Err(e) => return Err(format!("{}: {e}", path.display())),
            }
        }
        Err(format!(
            "another hoocode is merging {}; try again",
            dest.display()
        ))
    }
}

impl Drop for DirLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}

fn is_stale(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age > LOCK_STALE)
}

/// `YYYYMMDD-HHMMSS` in UTC, for backup folders and reports.
pub fn stamp_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to (year, month, day); Howard Hinnant's algorithm.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::civil_from_days;

    #[test]
    fn civil_dates_match_the_calendar() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(10_957), (2000, 1, 1));
        assert_eq!(civil_from_days(20_369), (2025, 10, 8));
    }
}
