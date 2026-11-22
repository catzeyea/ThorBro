//! Copying save folders between PC and Thor, plus backups and closing running emulators.
//!
//! The tricky part is the Thor side:
//! - Private app folders (`/data/data/<pkg>`) are only reachable via `run-as`, so data goes
//!   through a tar archive in `/data/local/tmp`.
//! - Since Android 11, adbd may not create folders under `Android/data`, but the shell may. So we
//!   push to `/data/local/tmp` first and copy from there with `cp`.
//! - A new save is first written next to the old one (`<folder>.thor-save-neu`) and only swapped
//!   in at the end, so a failed copy never leaves a half-written save behind.
//!
//! Used by `sync.rs` and `import.rs`.

use super::config::{backups_root, slug, state_dir};
use crate::adb::{self, posix_basename, posix_dirname, sh, shell_quote, tcmd};
use crate::util;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Number of backups kept per game; older ones are deleted.
const KEEP_BACKUPS: usize = 15;

/// Temporary tar file on the Thor for pushing into private app folders.
const REMOTE_PUSH_TAR: &str = "/data/local/tmp/thor-save-push.tar";

/// Temporary folder on the Thor for pushing into shared storage.
const REMOTE_PUSH_DIR: &str = "/data/local/tmp/thor-save-push";

/// Suffix of the staging folder next to the target, where the new save is assembled.
const STAGING_SUFFIX: &str = ".thor-save-neu";

/// Printed by our shell commands when every step succeeded.
const SUCCESS_MARKER: &str = "OK";

/// Creates the backup folder of `game` and picks a unique timestamp prefix for a new backup.
///
/// Returns `(backup folder, timestamp)`; callers create `<folder>/<timestamp>-<side>...`.
/// If a backup with this second's timestamp already exists, `.2`, `.3` ... is appended.
pub fn backup_dir(game: &str) -> Result<(PathBuf, String), String> {
    let dir = backups_root().join(slug(game));
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let base = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let existing: Vec<String> = fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    let (mut timestamp, mut counter) = (base.clone(), 1);
    while existing.iter().any(|name| name.starts_with(&format!("{timestamp}-"))) {
        counter += 1;
        timestamp = format!("{base}.{counter}");
    }
    Ok((dir, timestamp))
}

/// Deletes the oldest backups so only `KEEP_BACKUPS` remain. Names start with a timestamp,
/// so alphabetical order is chronological order.
pub fn prune_backups(dir: &Path) {
    let Ok(read_dir) = fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = read_dir.flatten().map(|entry| entry.path()).collect();
    entries.sort();
    if entries.len() > KEEP_BACKUPS {
        for old in &entries[..entries.len() - KEEP_BACKUPS] {
            let _ = fs::remove_dir_all(old);
        }
    }
}

/// Creates a temporary folder in the data folder that is *not* deleted automatically
/// (`keep()`); the caller is responsible for removing it.
pub fn tempdir(prefix: &str) -> Result<PathBuf, String> {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(state_dir())
        .map(|temp| temp.keep())
        .map_err(|e| e.to_string())
}

/// Runs `command` via `adb exec-out` and extracts the tar stream it prints into `dest`.
/// Modification times are preserved.
pub fn untar_from_thor(command: &str, dest: &Path) -> Result<(), String> {
    let mut child = adb::exec_out(command).map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let mut archive = tar::Archive::new(stdout);
    archive.set_preserve_mtime(true);
    let unpack_result = archive.unpack(dest);

    let mut stderr_text = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_string(&mut stderr_text);
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if let Err(e) = unpack_result {
        return Err(format!("Extraction failed: {e}\n{stderr_text}"));
    }
    if !status.success() {
        return Err(format!("Fetching from the Thor failed:\n{stderr_text}"));
    }
    Ok(())
}

/// Fetches a Thor folder to `dest` on the PC. `dest` must not exist yet.
///
/// The folder is downloaded into a temporary folder first and then moved into place.
pub fn pull_dir(remote: &str, dest: &Path) -> Result<(), String> {
    let temp = tempdir("pull-")?;
    let name = posix_basename(remote).to_string();
    // Rust has no `finally`. We run the steps in a helper function and clean up afterwards,
    // whatever its result was.
    let result = pull_dir_steps(remote, &name, &temp, dest);
    let _ = fs::remove_dir_all(&temp);
    result
}

/// The actual work of `pull_dir`: download `remote` into `temp/<name>`, then move it to `dest`.
fn pull_dir_steps(remote: &str, name: &str, temp: &Path, dest: &Path) -> Result<(), String> {
    if adb::private_pkg(remote).is_some() {
        // Private folder: let the app itself tar it up (run-as), stream it via exec-out.
        let parent = posix_dirname(remote);
        let tar_command = format!("tar -cf - -C {} {}", shell_quote(parent), shell_quote(name));
        untar_from_thor(&tcmd(remote, &tar_command), temp)?;
    } else {
        let output = adb::run(&["pull", "-a", remote, &temp.to_string_lossy()]);
        if !output.ok {
            return Err(format!("adb pull failed:\n{}", output.err_text()));
        }
    }
    let downloaded = temp.join(name);
    if !downloaded.is_dir() {
        return Err(format!("Fetching failed, {name} didn't arrive"));
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // `rename` fails across file systems; then copy instead.
    if fs::rename(&downloaded, dest).is_err() {
        util::copy_tree(&downloaded, dest, false)?;
    }
    Ok(())
}

/// Packs the local folder `local` into the tar file `out` (the folder itself is the top entry).
fn tar_dir(local: &Path, out: &Path) -> Result<(), String> {
    let file = fs::File::create(out).map_err(|e| e.to_string())?;
    let mut builder = tar::Builder::new(file);
    builder.follow_symlinks(false);
    let name = local.file_name().ok_or("no folder name")?;
    builder.append_dir_all(name, local).map_err(|e| e.to_string())?;
    builder.finish().map_err(|e| e.to_string())
}

/// Pushes `local` as a tar to `/data/local/tmp`, then extracts it via run-as into `stage/<name>`.
/// The tar on the Thor is always removed afterwards.
fn push_private(local: &Path, stage: &str) -> Result<(), String> {
    // `NamedTempFile` deletes the local tar automatically when it goes out of scope.
    let local_tar = tempfile::Builder::new()
        .prefix("push-")
        .suffix(".tar")
        .tempfile_in(state_dir())
        .map_err(|e| e.to_string())?;
    let result = push_private_steps(local, local_tar.path(), stage);
    sh(&format!("rm -f {}", shell_quote(REMOTE_PUSH_TAR)));
    result
}

/// The actual work of `push_private`.
fn push_private_steps(local: &Path, local_tar: &Path, stage: &str) -> Result<(), String> {
    tar_dir(local, local_tar)?;
    let output = adb::run(&["push", &local_tar.to_string_lossy(), REMOTE_PUSH_TAR]);
    if !output.ok {
        return Err(format!("adb push failed:\n{}", output.err_text()));
    }
    // The app user (run-as) must be able to read the tar that the shell user pushed.
    sh(&format!("chmod 644 {}", shell_quote(REMOTE_PUSH_TAR)));
    let extract = format!(
        "rm -rf {s} && mkdir -p {s} && tar -xf {t} -C {s} && echo OK",
        s = shell_quote(stage),
        t = shell_quote(REMOTE_PUSH_TAR)
    );
    let output = sh(&tcmd(stage, &extract));
    if !output.stdout.contains(SUCCESS_MARKER) {
        sh(&tcmd(stage, &format!("rm -rf {}", shell_quote(stage))));
        return Err(format!("Extracting via run-as failed:\n{}", output.err_text()));
    }
    Ok(())
}

/// Replaces the Thor folder `remote` with the local folder `local`.
///
/// The new content is assembled in a staging folder next to `remote` and swapped in at the end.
pub fn push_dir(local: &Path, remote: &str) -> Result<(), String> {
    let remote = remote.trim_end_matches('/');
    let stage = format!("{remote}{STAGING_SUFFIX}");
    let local_name = local.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    // Where the uploaded folder ends up inside the staging folder.
    let staged_folder = format!("{stage}/{local_name}");
    if adb::private_pkg(remote).is_some() {
        push_dir_private(local, remote, &stage, &staged_folder)
    } else {
        push_dir_shared(local, remote, &stage, &local_name, &staged_folder)
    }
}

/// `push_dir` for private app folders (everything via run-as).
fn push_dir_private(local: &Path, remote: &str, stage: &str, staged_folder: &str) -> Result<(), String> {
    push_private(local, stage)?;
    let swap = format!(
        "test -d {i} && rm -rf {r} && mv {i} {r} && rmdir {s} && echo OK",
        i = shell_quote(staged_folder),
        r = shell_quote(remote),
        s = shell_quote(stage)
    );
    let output = sh(&tcmd(remote, &swap));
    if !output.stdout.contains(SUCCESS_MARKER) {
        return Err(format!(
            "Couldn't move the new save into place on the Thor:\n{}\nThe new save may still be in {stage}",
            output.stderr
        ));
    }
    Ok(())
}

/// `push_dir` for shared storage (`/sdcard/...`, `Android/data/...`).
///
/// adbd may not create folders under `Android/data` since Android 11, the shell may.
/// So: push to `/data/local/tmp` first, then `cp` into the staging folder, then swap.
fn push_dir_shared(local: &Path, remote: &str, stage: &str, local_name: &str, staged_folder: &str) -> Result<(), String> {
    let quoted_stage = shell_quote(stage);
    let quoted_tmp = shell_quote(REMOTE_PUSH_DIR);
    sh(&format!("rm -rf {quoted_stage} {quoted_tmp}; mkdir -p {quoted_stage} {quoted_tmp}"));

    let output = adb::run(&["push", &local.to_string_lossy(), &format!("{REMOTE_PUSH_DIR}/")]);
    if !output.ok {
        sh(&format!("rm -rf {quoted_stage} {quoted_tmp}"));
        return Err(format!("adb push failed:\n{}", output.err_text()));
    }

    let pushed = format!("{REMOTE_PUSH_DIR}/{local_name}");
    let output = sh(&format!(
        "cp -r --preserve=t {} {quoted_stage}/ && echo OK; rm -rf {quoted_tmp}",
        shell_quote(&pushed)
    ));
    if !output.stdout.contains(SUCCESS_MARKER) {
        sh(&format!("rm -rf {quoted_stage}"));
        return Err(format!("Copying on the Thor failed:\n{}", output.err_text()));
    }

    let output = sh(&format!(
        "rm -rf {r} && mv {i} {r} && rmdir {quoted_stage} && echo OK",
        r = shell_quote(remote),
        i = shell_quote(staged_folder)
    ));
    if !output.stdout.contains(SUCCESS_MARKER) {
        return Err(format!(
            "Couldn't move the new save into place on the Thor:\n{}\nThe new save is still in {stage}",
            output.stderr
        ));
    }
    Ok(())
}

/// Package name of the emulator that owns `path`, if it is currently running on the Thor.
pub fn thor_running_pkg(path: Option<&str>) -> Option<String> {
    let pkg = adb::thor_pkg(path?)?;
    let pid = sh(&format!("pidof {}", shell_quote(&pkg))).stdout;
    if pid.trim().is_empty() { None } else { Some(pkg) }
}

/// Force-closes an app on the Thor (so it can't overwrite the save we are writing).
pub fn stop_thor_pkg(pkg: &str) {
    sh(&format!("am force-stop {}", shell_quote(pkg)));
}

/// Creates the parent folder of a Thor save folder (via run-as if needed).
pub fn ensure_thor_parent(thor_path: &str) {
    sh(&tcmd(thor_path, &format!("mkdir -p {}", shell_quote(posix_dirname(thor_path)))));
}

/// Backs up the save folder `path` on `side` to `backup` (PC: copy, Thor: download).
pub fn backup_side(side: &str, path: &str, backup: &Path) -> Result<(), String> {
    if side == "pc" {
        util::copy_tree(Path::new(path), backup, false)
    } else {
        pull_dir(path, backup)
    }
}
