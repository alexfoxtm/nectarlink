// SPDX-License-Identifier: GPL-3.0-or-later
//! `cargo xtask notices <out> [--target <triple>] [--package <crate>] [--license-texts <dir>]`:
//! the third-party notices shipped with the desktop app. Every crate compiled
//! into the app (normal dependencies for that target; not build or dev ones)
//! with its license and the license files it ships, then Qt and the bundled
//! fonts. The Windows target is the default; an `*-apple-darwin` target writes
//! the macOS (Waypair Link) notices instead. `--package` names another
//! workspace crate, such as `nectarlink-ffi` for Android, and writes only its
//! crates.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use serde_json::Value;

const ROOT_PACKAGE: &str = "nectarlink-desktop";
const DEFAULT_TARGET: &str = "x86_64-pc-windows-msvc";
const USAGE: &str =
    "usage: cargo xtask notices <out file> [--target <triple>] [--package <crate>] [--license-texts <dir>]";

/// The wording of one platform's notices.
struct Platform {
    title: &'static str,
    intro: &'static str,
    qt: &'static str,
    fonts: &'static str,
    /// macOS notices (and any `--package` other than the desktop app) are
    /// made offline, and every crate must carry license text: its own files,
    /// or the standard texts from `--license-texts`.
    mac: bool,
}

static WINDOWS: Platform = Platform {
    title: "NectarLink for Windows: third-party notices",
    intro: "\nNectarLink is free software under the GNU General Public License, version 3\n\
            or later (LICENSE.txt). It includes the software below, under these terms.\n",
    qt: "This app uses the Qt framework (https://www.qt.io), version 6, under the GNU\n\
         Lesser General Public License version 3 (below). Qt's libraries are the\n\
         separate Qt6*.dll files and plugins in this folder; you may replace them with\n\
         your own builds of the same version. Qt's source code is available from\n\
         https://download.qt.io/official_releases/qt/ and https://code.qt.io.\n",
    fonts: "Figtree, Instrument Serif and Space Mono, under the SIL Open Font License 1.1:",
    mac: false,
};

static MAC: Platform = Platform {
    title: "Waypair Link for macOS: third-party notices",
    intro: "\nWaypair Link is an independently maintained fork of NectarLink, free software under the GNU\n\
            General Public License, version 3 or later (LICENSE.txt in the app's Resources\n\
            folder). It includes the software below, under these terms. Notices for the\n\
            Qt, Homebrew, Sparkle and ADB components bundled with it, and where their\n\
            exact sources are, are in the other files of this folder.\n",
    qt: "This app uses the Qt framework (https://www.qt.io), version 6, under the GNU\n\
         Lesser General Public License version 3 (below). Qt's libraries are the\n\
         separate Qt*.framework bundles in Contents/Frameworks and the plugins in\n\
         Contents/PlugIns. QT-NOTICES.txt in this folder names the exact Qt version\n\
         and build, the third-party code Qt declares inside those modules, and where\n\
         the corresponding source and patches are.\n",
    fonts: "Figtree, Instrument Serif and Space Mono, compiled into the app, under the SIL\n\
            Open Font License 1.1 (each text is also in this folder as OFL-*.txt):",
    mac: true,
};

fn platform(target: &str) -> &'static Platform {
    if target.ends_with("-apple-darwin") { &MAC } else { &WINDOWS }
}

/// License-ish files a crate ships.
fn is_license_file(name: &str) -> bool {
    let upper = name.to_uppercase();
    ["LICENSE", "LICENCE", "COPYING", "NOTICE", "UNLICENSE"].iter().any(|p| upper.starts_with(p))
}

struct Crate {
    name: String,
    version: String,
    license: String,
    /// Whether `license` came from the manifest rather than the fallback.
    declared: bool,
    authors: Vec<String>,
    repository: String,
    files: Vec<(String, String)>,
}

pub fn run(args: &[&str]) -> Result<()> {
    let [out, flags @ ..] = args else { bail!("{USAGE}") };
    let (mut target, mut package, mut texts) = (DEFAULT_TARGET, ROOT_PACKAGE, None);
    let mut flags = flags.iter();
    while let Some(flag) = flags.next() {
        match (*flag, flags.next()) {
            ("--target", Some(value)) => target = *value,
            ("--package", Some(value)) => package = *value,
            ("--license-texts", Some(value)) => texts = Some(PathBuf::from(*value)),
            _ => bail!("{USAGE}"),
        }
    }
    let platform = platform(target);
    // Another workspace crate (the Android FFI library) gets its crates only;
    // the Qt and font sections are the desktop app's.
    let desktop = package == ROOT_PACKAGE;
    let strict = platform.mac || !desktop;
    let root = crate::repo_root();
    let mut metadata = Command::new(env_cargo());
    metadata.args(["metadata", "--format-version", "1", "--locked", "--filter-platform", target]);
    if strict {
        metadata.arg("--offline");
    }
    let output = metadata.current_dir(&root).output().context("running cargo metadata")?;
    if !output.status.success() {
        bail!("cargo metadata failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    let metadata: Value = serde_json::from_slice(&output.stdout).context("parsing cargo metadata")?;
    let crates = shipped_crates(&metadata, package)?;

    let mut text = String::new();
    if desktop {
        writeln!(text, "{}", platform.title)?;
        writeln!(text, "{}", "=".repeat(platform.title.chars().count()))?;
        writeln!(text, "{}", platform.intro)?;
        write_qt(&mut text, &root, platform)?;
        write_fonts(&mut text, &root, platform)?;
    } else {
        let title = format!("Rust crates in {package} for {target}: third-party notices");
        writeln!(text, "{title}\n{}", "=".repeat(title.chars().count()))?;
        writeln!(text, "\n{package} is compiled with the crates below, under these terms.")?;
    }
    if strict {
        let heading = format!("Rust crates ({} in the {target} build)", crates.len());
        writeln!(text, "\n{heading}\n{}", "-".repeat(heading.chars().count()))?;
    } else {
        writeln!(text, "\nRust crates ({} in this build)\n{}", crates.len(), "-".repeat(30))?;
    }
    for c in &crates {
        writeln!(text, "\n{} {}  ·  {}", c.name, c.version, c.license)?;
        if !c.repository.is_empty() {
            writeln!(text, "{}", c.repository)?;
        }
        if strict && c.files.is_empty() && !c.authors.is_empty() {
            writeln!(text, "Authors: {}", c.authors.join(", "))?;
        }
        for (file, body) in &c.files {
            writeln!(text, "\n--- {} ({file}) ---\n{}", c.name, body.trim_end())?;
        }
    }
    if strict {
        write_standard_texts(&mut text, &crates, texts.as_deref())?;
        // Match Git's text normalization so pinned notice hashes survive a
        // fresh checkout and source-archive extraction.
        text = text.replace("\r\n", "\n");
    }
    let out = PathBuf::from(*out);
    if let Some(dir) = out.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&out, text).with_context(|| format!("writing {}", out.display()))?;
    println!("wrote {} ({} crates, {target})", out.display(), crates.len());
    Ok(())
}

fn env_cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

/// The crates in `root_package`'s normal dependency graph, workspace crates
/// left out (they're NectarLink's own).
fn shipped_crates(metadata: &Value, root_package: &str) -> Result<Vec<Crate>> {
    let packages: BTreeMap<&str, &Value> = metadata["packages"]
        .as_array()
        .context("no packages")?
        .iter()
        .filter_map(|p| Some((p["id"].as_str()?, p)))
        .collect();
    let workspace: BTreeSet<&str> = metadata["workspace_members"]
        .as_array()
        .context("no members")?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let nodes: BTreeMap<&str, &Value> = metadata["resolve"]["nodes"]
        .as_array()
        .context("no resolve graph")?
        .iter()
        .filter_map(|n| Some((n["id"].as_str()?, n)))
        .collect();
    let start = packages
        .iter()
        .find(|(id, p)| p["name"] == root_package && workspace.contains(*id))
        .map(|(id, _)| *id)
        .with_context(|| format!("{root_package} isn't in the workspace"))?;

    let mut seen = BTreeSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(id) = queue.pop_front() {
        let Some(node) = nodes.get(id) else { continue };
        for dep in node["deps"].as_array().into_iter().flatten() {
            // Only what's linked into the app: not build scripts or tests.
            let normal = dep["dep_kinds"].as_array().into_iter().flatten().any(|k| k["kind"].is_null());
            if let Some(dep_id) = dep["pkg"].as_str()
                && normal
                && seen.insert(dep_id)
            {
                queue.push_back(dep_id);
            }
        }
    }

    let mut crates: Vec<Crate> = seen
        .into_iter()
        .filter(|id| !workspace.contains(id))
        .filter_map(|id| packages.get(id))
        .map(|p| {
            let dir =
                Path::new(p["manifest_path"].as_str().unwrap_or_default()).parent().map(Path::to_path_buf);
            let mut files: Vec<(String, String)> = dir
                .and_then(|d| fs::read_dir(d).ok())
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_file() && is_license_file(&e.file_name().to_string_lossy()))
                .filter_map(|e| {
                    Some((e.file_name().to_string_lossy().into_owned(), fs::read_to_string(e.path()).ok()?))
                })
                .collect();
            files.sort();
            Crate {
                name: p["name"].as_str().unwrap_or_default().to_owned(),
                version: p["version"].as_str().unwrap_or_default().to_owned(),
                license: p["license"].as_str().unwrap_or("see its files").to_owned(),
                declared: p["license"].is_string(),
                authors: p["authors"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect(),
                repository: p["repository"].as_str().unwrap_or_default().to_owned(),
                files,
            }
        })
        .collect();
    crates.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    Ok(crates)
}

fn write_qt(text: &mut String, root: &Path, platform: &Platform) -> Result<()> {
    writeln!(text, "Qt\n--")?;
    writeln!(text, "{}", platform.qt)?;
    let lgpl = fs::read_to_string(root.join("LICENSES/LGPL-3.0-only.txt")).context("reading the LGPL")?;
    writeln!(text, "{}\n", lgpl.trim_end())?;
    Ok(())
}

fn write_fonts(text: &mut String, root: &Path, platform: &Platform) -> Result<()> {
    writeln!(text, "\nFonts\n-----")?;
    writeln!(text, "{}", platform.fonts)?;
    let mut ofl: Vec<PathBuf> = fs::read_dir(root.join("assets/fonts"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("OFL-")))
        .collect();
    ofl.sort();
    if platform.mac && ofl.is_empty() {
        bail!("no OFL-*.txt beside the fonts in assets/fonts");
    }
    for path in ofl {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        writeln!(text, "\n--- {name} ---\n{}", fs::read_to_string(&path)?.trim_end())?;
    }
    Ok(())
}

/// The license identifiers an SPDX expression names, exceptions included.
fn license_ids(expression: &str) -> BTreeSet<String> {
    expression
        .split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '/'))
        .filter(|t| !t.is_empty() && !matches!(*t, "OR" | "AND" | "WITH"))
        .map(str::to_owned)
        .collect()
}

/// Crates that ship no license file get the standard text of every license
/// they name, read from `dir` as `<SPDX id>.txt` (the SPDX license list's
/// file names).
fn write_standard_texts(text: &mut String, crates: &[Crate], dir: Option<&Path>) -> Result<()> {
    let bare: Vec<&Crate> = crates.iter().filter(|c| c.files.is_empty()).collect();
    if bare.is_empty() {
        return Ok(());
    }
    if let Some(c) = bare.iter().find(|c| !c.declared) {
        bail!("{} {} declares no license and ships no license file", c.name, c.version);
    }
    let ids: BTreeSet<String> = bare.iter().flat_map(|c| license_ids(&c.license)).collect();
    if let Some(id) =
        ids.iter().find(|id| !id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '+')))
    {
        bail!("unexpected license identifier {id:?}");
    }
    let wanted = || ids.iter().map(|id| format!("{id}.txt")).collect::<Vec<_>>().join(", ");
    let Some(dir) = dir else {
        bail!("{} crates ship no license file; pass --license-texts <dir> holding {}", bare.len(), wanted());
    };
    let missing: Vec<String> =
        ids.iter().map(|id| format!("{id}.txt")).filter(|file| !dir.join(file).is_file()).collect();
    if !missing.is_empty() {
        bail!("{} lacks the license texts {}", dir.display(), missing.join(", "));
    }
    let heading = "Standard license texts";
    writeln!(text, "\n{heading}\n{}", "-".repeat(heading.len()))?;
    let names: Vec<String> = bare.iter().map(|c| format!("{} {}", c.name, c.version)).collect();
    writeln!(
        text,
        "These crates ship no license file of their own, so the standard text of each\n\
         license they name follows: {}.",
        names.join(", ")
    )?;
    for id in &ids {
        let body = fs::read_to_string(dir.join(format!("{id}.txt")))
            .with_context(|| format!("reading the {id} text"))?;
        writeln!(text, "\n--- {id} ---\n{}", body.trim_end())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bare_crate(license: &str) -> Crate {
        Crate {
            name: "objc2".into(),
            version: "0.6.4".into(),
            license: license.into(),
            declared: true,
            authors: Vec::new(),
            repository: String::new(),
            files: Vec::new(),
        }
    }

    #[test]
    fn recognizes_license_files() {
        for name in ["LICENSE", "LICENSE-MIT", "license-apache", "COPYING", "NOTICE.md", "UNLICENSE"] {
            assert!(is_license_file(name), "{name}");
        }
        for name in ["README.md", "Cargo.toml", "src"] {
            assert!(!is_license_file(name), "{name}");
        }
    }

    fn windows_stays_the_default_wording() {
        assert!(!platform(DEFAULT_TARGET).mac);
        assert_eq!(platform(DEFAULT_TARGET).title, "NectarLink for Windows: third-party notices");
        assert_eq!(platform(DEFAULT_TARGET).title.chars().count(), 43);
        assert!(!platform("aarch64-pc-windows-msvc").mac);
    }

    fn apple_targets_get_the_mac_wording() {
        for target in ["aarch64-apple-darwin", "x86_64-apple-darwin"] {
            let p = platform(target);
            assert!(p.mac, "{target}");
            assert!(p.title.starts_with("Waypair Link for macOS"));
            assert!(p.qt.contains("Qt*.framework") && !p.qt.contains(".dll"));
        }
    }

    fn splits_spdx_expressions() {
        let ids = |e| license_ids(e).into_iter().collect::<Vec<_>>();
        assert_eq!(ids("MIT OR Apache-2.0"), ["Apache-2.0", "MIT"]);
        assert_eq!(ids("(MIT OR Apache-2.0) AND Unicode-3.0"), ["Apache-2.0", "MIT", "Unicode-3.0"]);
        assert_eq!(ids("Apache-2.0 WITH LLVM-exception"), ["Apache-2.0", "LLVM-exception"]);
        assert_eq!(ids("MIT/Apache-2.0"), ["Apache-2.0", "MIT"]);
    }

    fn crates_with_their_own_files_need_no_standard_texts() {
        let mut c = bare_crate("MIT");
        c.files.push(("LICENSE".into(), "text".into()));
        let mut text = String::new();
        write_standard_texts(&mut text, &[c], None).unwrap();
        assert!(text.is_empty());
    }

    #[test]
    fn notices_preserve_platforms_and_complete_license_material() {
        windows_stays_the_default_wording();
        apple_targets_get_the_mac_wording();
        splits_spdx_expressions();
        crates_with_their_own_files_need_no_standard_texts();
        let dir = std::env::temp_dir().join(format!("waypair-notice-text-{}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("MIT.txt"), "fixture license body").unwrap();
        let mut text = String::new();
        let result = write_standard_texts(&mut text, &[bare_crate("MIT")], Some(&dir));
        fs::remove_dir_all(&dir).unwrap();
        result.unwrap();
        assert!(text.contains("objc2 0.6.4") && text.contains("--- MIT ---\nfixture license body"));
    }

    #[test]
    fn a_bare_crate_without_texts_fails_closed() {
        let mut text = String::new();
        let err = write_standard_texts(&mut text, &[bare_crate("Zlib OR Apache-2.0 OR MIT")], None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("--license-texts") && err.contains("Zlib.txt"), "{err}");
        let missing = Path::new("/nonexistent/waypair-license-texts");
        let err =
            write_standard_texts(&mut text, &[bare_crate("MIT")], Some(missing)).unwrap_err().to_string();
        assert!(err.contains("MIT.txt"), "{err}");
        let mut undeclared = bare_crate("see its files");
        undeclared.declared = false;
        assert!(write_standard_texts(&mut text, &[undeclared], None).is_err());
        assert!(text.is_empty());
    }
}
