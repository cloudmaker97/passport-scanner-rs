//! Updating from the project's GitHub releases.
//!
//! Every release carries one archive per platform, named as [`ASSET`], holding
//! the files of the program folder. Installing moves each file into place
//! beside the running program and keeps the one it replaces as `*.replaced`
//! until the next start: Windows will not delete a running executable, but it
//! will rename one.
use passport_scanner_ui::Update;
use semver::Version;
use sha2::{Digest, Sha256};
use std::{
    fs,
    future::Future,
    io::{self, Cursor},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

/// This platform's archive in a release; the release workflow uses the same names.
const ASSET: Option<&str> = if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
    Some("passport-scanner-windows-x64.zip")
} else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    Some("passport-scanner-linux-x64.zip")
} else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    Some("passport-scanner-macos-arm64.zip")
} else {
    None
};
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
/// The desktop program's file name in a release; the release workflow and
/// `package.ps1` name it so.
const PROGRAM: &str = if cfg!(windows) { "Passport Scanner.exe" } else { "Passport Scanner" };
/// Files that releases before the tools/ and debug-tools/ folders kept beside
/// the program. An update brings their replacements in the new places, so
/// whatever of these is left over is out of date.
const SUPERSEDED: [&str; 16] = [
    "passport-scanner-desktop",
    "passport-scanner-scan",
    "passport-scanner-dump",
    "passport-scanner-lamps",
    "passport-scanner-serve",
    "passport-scanner-read",
    "passport-scanner-expose",
    "passport-scanner-controls",
    "passport-scanner-exposure",
    "passport-scanner-probe",
    "passport-scanner-time",
    "passport-scanner-stages",
    "passport-scanner-survey",
    "passport-scanner-calibrate",
    "passport-scanner-web",
    "passport-scanner-mcp",
];
const REPLACED: &str = "replaced";
const STAGING: &str = ".update";
/// Far above any real archive; stops a broken server from filling the memory.
const DOWNLOAD_LIMIT: u64 = 512 * 1024 * 1024;

static RESTART: AtomicBool = AtomicBool::new(false);

fn latest_release_url() -> Option<String> {
    let slug = REPOSITORY.strip_prefix("https://github.com/")?.trim_end_matches('/');
    Some(format!("https://api.github.com/repos/{slug}/releases/latest"))
}

/// Whether this platform has a release archive to look for.
pub(crate) fn checks() -> bool {
    ASSET.is_some() && latest_release_url().is_some()
}

/// Release builds only: a development build would replace itself in `target/`.
pub(crate) fn installs() -> bool {
    !cfg!(debug_assertions) && checks()
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .user_agent(concat!("passport-scanner/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// The latest release, if it is newer than `current` and has an archive here.
pub(crate) fn check(current: &str) -> Result<Option<Update>, String> {
    let (Some(asset), Some(url)) = (ASSET, latest_release_url()) else { return Ok(None) };
    let body = match agent().get(&url).header("Accept", "application/vnd.github+json").call() {
        Ok(mut response) => response.body_mut().read_to_string().map_err(|e| e.to_string())?,
        // No release published yet.
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let release: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    newer(&release, current, asset)
}

fn newer(
    release: &serde_json::Value,
    current: &str,
    asset: &str,
) -> Result<Option<Update>, String> {
    let tag = release["tag_name"].as_str().ok_or("the release has no tag")?;
    let version = tag.strip_prefix('v').unwrap_or(tag);
    let latest = Version::parse(version).map_err(|e| format!("{tag}: {e}"))?;
    let running = Version::parse(current).map_err(|e| format!("{current}: {e}"))?;
    if latest <= running {
        return Ok(None);
    }
    let Some(file) = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|file| file["name"].as_str() == Some(asset))
    else {
        return Ok(None);
    };
    let download = file["browser_download_url"].as_str().ok_or("the archive has no address")?;
    Ok(Some(Update {
        version: version.to_owned(),
        download: download.to_owned(),
        digest: file["digest"].as_str().map(str::to_owned),
    }))
}

/// Download, verify and unpack `update` over the folder of the running program.
pub(crate) fn install(update: &Update) -> Result<(), String> {
    let folder = program_folder().map_err(|e| e.to_string())?;
    let archive = agent()
        .get(&update.download)
        .call()
        .and_then(|mut response| {
            response.body_mut().with_config().limit(DOWNLOAD_LIMIT).read_to_vec()
        })
        .map_err(|e| e.to_string())?;
    verify(&archive, update.digest.as_deref())?;
    unpack(&archive, &folder).map_err(|e| e.to_string())
}

fn program_folder() -> io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    exe.parent().map(Path::to_path_buf).ok_or_else(|| invalid("the program has no folder"))
}

/// GitHub reports `sha256:<hex>` for assets; without one, TLS is all there is.
fn verify(archive: &[u8], digest: Option<&str>) -> Result<(), String> {
    let Some(expected) = digest.and_then(|digest| digest.strip_prefix("sha256:")) else {
        return Ok(());
    };
    let actual: String = Sha256::digest(archive).iter().map(|byte| format!("{byte:02x}")).collect();
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err("the download does not match the release checksum".into())
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Unpack everything first, then swap the files in; a failed swap restores
/// the files already replaced.
fn unpack(archive: &[u8], folder: &Path) -> io::Result<()> {
    let mut zip = zip::ZipArchive::new(Cursor::new(archive)).map_err(io::Error::other)?;
    let staging = folder.join(STAGING);
    let _ = fs::remove_dir_all(&staging);
    let mut files = Vec::new();
    let staged = (|| -> io::Result<()> {
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index).map_err(io::Error::other)?;
            let name =
                entry.enclosed_name().ok_or_else(|| invalid("unsafe path in the archive"))?;
            if entry.is_dir() {
                continue;
            }
            let path = staging.join(&name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            io::copy(&mut entry, &mut fs::File::create(&path)?)?;
            #[cfg(unix)]
            if let Some(mode) = entry.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(mode & 0o777))?;
            }
            files.push(name);
        }
        if files.is_empty() { Err(invalid("the archive is empty")) } else { Ok(()) }
    })();
    let result = staged.and_then(|()| swap(&staging, folder, &files));
    let _ = fs::remove_dir_all(&staging);
    result
}

fn swap(staging: &Path, folder: &Path, files: &[PathBuf]) -> io::Result<()> {
    let mut done: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    for name in files {
        let target = folder.join(name);
        match swap_one(&staging.join(name), &target) {
            Ok(kept) => done.push((target, kept)),
            Err(error) => {
                for (target, kept) in done.into_iter().rev() {
                    let _ = fs::remove_file(&target);
                    if let Some(kept) = kept {
                        let _ = fs::rename(kept, &target);
                    }
                }
                return Err(io::Error::new(error.kind(), format!("{}: {error}", target.display())));
            }
        }
    }
    Ok(())
}

/// Returns where the replaced file went, if there was one.
fn swap_one(staged: &Path, target: &Path) -> io::Result<Option<PathBuf>> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let kept = if target.exists() {
        let kept = replaced(target);
        let _ = fs::remove_file(&kept);
        fs::rename(target, &kept)?;
        Some(kept)
    } else {
        None
    };
    if let Err(error) = fs::rename(staged, target) {
        if let Some(kept) = &kept {
            let _ = fs::rename(kept, target);
        }
        return Err(error);
    }
    Ok(kept)
}

fn replaced(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".");
    name.push(REPLACED);
    name.into()
}

/// Delete what the last update replaced, in the program folder and one level
/// below it. Best effort: a file still in use goes on the next start.
pub(crate) fn clean_up() {
    let Ok(folder) = program_folder() else { return };
    clean_folder(&folder, 1);
    retire_superseded(&folder);
}

/// Remove the loose programs of an older layout, once this program runs as
/// the release names it - never in a development build, where those files
/// are the build itself.
fn retire_superseded(folder: &Path) {
    if !folder.join(PROGRAM).exists() || !folder.join(passport_scanner_app::TOOLS_DIR).is_dir() {
        return;
    }
    for name in SUPERSEDED {
        let file = folder.join(if cfg!(windows) { format!("{name}.exe") } else { name.to_owned() });
        // The old program may be the one still running after an update from
        // it; renamed, it goes on the next start like any replaced file.
        if fs::remove_file(&file).is_err() && file.exists() {
            let _ = fs::rename(&file, replaced(&file));
        }
    }
}

fn clean_folder(folder: &Path, depth: u8) {
    let Ok(entries) = fs::read_dir(folder) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            if depth > 0 {
                clean_folder(&path, depth - 1);
            }
        } else if path.extension().is_some_and(|extension| extension == REPLACED) {
            let _ = fs::remove_file(path);
        }
    }
}

/// Ask for the program to start again once its window has closed.
pub(crate) fn request_restart() {
    RESTART.store(true, Ordering::SeqCst);
}

/// Start the program again if a restart was asked for; the old session is
/// gone by now, so the new one finds the scanner free. That is the program as
/// the installed release names it, which need not be the name `exe` - the
/// running one - had.
pub(crate) fn restart_if_requested(exe: Option<PathBuf>) {
    if RESTART.load(Ordering::SeqCst)
        && let Some(exe) = exe
    {
        let named = exe.with_file_name(PROGRAM);
        let _ = std::process::Command::new(if named.exists() { named } else { exe }).spawn();
    }
}

/// Run blocking work on its own thread so the interface keeps drawing.
pub(crate) fn off_thread<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> impl Future<Output = Result<T, String>> {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = sender.send(work());
    });
    async move { receiver.await.unwrap_or_else(|_| Err("the update stopped unexpectedly".into())) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn release(tag: &str) -> serde_json::Value {
        json!({
            "tag_name": tag,
            "assets": [
                {"name": "Passport-Scanner-User-Guide.pdf", "browser_download_url": "https://example/guide.pdf"},
                {"name": "here.zip", "browser_download_url": "https://example/here.zip", "digest": "sha256:ab"}
            ]
        })
    }

    #[test]
    fn only_a_newer_release_with_an_archive_here_is_offered() {
        let update = newer(&release("v0.2.0"), "0.1.0", "here.zip").unwrap().unwrap();
        assert_eq!(update.version, "0.2.0");
        assert_eq!(update.download, "https://example/here.zip");
        assert_eq!(update.digest.as_deref(), Some("sha256:ab"));
        assert_eq!(newer(&release("v0.1.0"), "0.1.0", "here.zip").unwrap(), None);
        assert_eq!(newer(&release("0.0.9"), "0.1.0", "here.zip").unwrap(), None);
        assert_eq!(newer(&release("v0.2.0"), "0.1.0", "elsewhere.zip").unwrap(), None);
        assert!(newer(&release("latest"), "0.1.0", "here.zip").is_err());
        // A pre-release sorts before its release.
        assert_eq!(newer(&release("v0.2.0-rc.1"), "0.2.0", "here.zip").unwrap(), None);
    }

    #[test]
    fn checksums_are_enforced_when_reported() {
        let empty = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert!(verify(b"", Some(empty)).is_ok());
        assert!(verify(b"x", Some(empty)).is_err());
        assert!(verify(b"x", None).is_ok());
    }

    fn scratch() -> PathBuf {
        let folder = std::env::temp_dir().join(format!(
            "passport-scanner-update-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn archive(files: &[(&str, &str)]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn unpacking_replaces_program_files_and_keeps_the_rest() {
        let folder = scratch();
        fs::write(folder.join("program.exe"), "old").unwrap();
        fs::write(folder.join("user-notes.txt"), "mine").unwrap();
        unpack(&archive(&[("program.exe", "new"), ("www/index.html", "page")]), &folder).unwrap();
        assert_eq!(fs::read_to_string(folder.join("program.exe")).unwrap(), "new");
        assert_eq!(fs::read_to_string(folder.join("program.exe.replaced")).unwrap(), "old");
        assert_eq!(fs::read_to_string(folder.join("www/index.html")).unwrap(), "page");
        assert_eq!(fs::read_to_string(folder.join("user-notes.txt")).unwrap(), "mine");
        assert!(!folder.join(STAGING).exists());
        clean_folder(&folder, 1);
        assert!(!folder.join("program.exe.replaced").exists());
        assert!(folder.join("program.exe").exists());
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn an_old_flat_layout_is_cleared_once_the_new_one_is_in_place() {
        let folder = scratch();
        let exe = |name: &str| if cfg!(windows) { format!("{name}.exe") } else { name.to_owned() };
        fs::write(folder.join(exe("passport-scanner-desktop")), "old").unwrap();
        fs::write(folder.join(exe("passport-scanner-mcp")), "old").unwrap();
        fs::write(folder.join("user-notes.txt"), "mine").unwrap();
        // Not yet: without the new layout these are the program itself.
        retire_superseded(&folder);
        assert!(folder.join(exe("passport-scanner-desktop")).exists());

        fs::write(folder.join(PROGRAM), "new").unwrap();
        fs::create_dir_all(folder.join(passport_scanner_app::TOOLS_DIR)).unwrap();
        retire_superseded(&folder);
        assert!(!folder.join(exe("passport-scanner-desktop")).exists());
        assert!(!folder.join(exe("passport-scanner-mcp")).exists());
        assert!(folder.join(PROGRAM).exists());
        assert!(folder.join("user-notes.txt").exists());
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn archives_cannot_write_outside_the_program_folder() {
        let folder = scratch();
        let program = folder.join("program");
        fs::create_dir_all(&program).unwrap();
        assert!(unpack(&archive(&[("../escaped.txt", "x")]), &program).is_err());
        assert!(!folder.join("escaped.txt").exists());
        fs::remove_dir_all(folder).unwrap();
    }
}
