//! Dependency fetching (`parameters.kapitan.dependencies`): git repositories,
//! http(s) files and helm charts land in their `output_path` before the
//! targets compile, so the inputs that read them see the files and the
//! manifest records them like any other read.
//!
//! Semantics follow kapitan's `dependency_manager`: a dependency is only
//! written where nothing exists yet unless it is forced (`--force-fetch`, or
//! `force_fetch: true` on the item), in which case existing files are
//! overwritten. One difference: a dependency whose output path already
//! exists is not fetched at all (kapitan does the same for helm charts, and
//! for git and http would only add files that are missing), so a compile
//! with `fetch: true` in `.kapitan` stays offline once everything is there.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rayon::prelude::*;
use serde::Serialize;
use serde_json::Value as Json;
use sha2::{Digest, Sha256};

use crate::inputs::helm::{helm_binary, run_helm};

pub const DEPENDENCIES_PATH: &str = "parameters.kapitan.dependencies";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Kind {
    Git {
        #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
        git_ref: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        subdir: Option<String>,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        submodules: bool,
    },
    Http {
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        unpack: bool,
    },
    Helm {
        chart_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        version: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        helm_path: Option<String>,
    },
    /// An OCI artifact pulled with oras; not implemented natively.
    Oci,
}

impl Kind {
    pub fn name(&self) -> &'static str {
        match self {
            Kind::Git { .. } => "git",
            Kind::Http { .. } => "http",
            Kind::Helm { .. } => "helm",
            Kind::Oci => "oci",
        }
    }
}

/// One `dependencies` item as a target declares it.
#[derive(Clone, Debug, Serialize)]
pub struct Dependency {
    /// The target that declares it.
    pub target: String,
    #[serde(flatten)]
    pub kind: Kind,
    pub source: String,
    /// `output_path` as written in the inventory.
    pub output_path: String,
    /// Where it lands: `output_path` under the compile output directory, normalised.
    #[serde(skip)]
    pub dest: PathBuf,
    pub force_fetch: bool,
}

/// The `dependencies` of one target's rendered document.
pub fn dependencies(
    target: &str,
    doc: &Json,
    output_root: &Path,
) -> Result<Vec<Dependency>, String> {
    let items = match doc.as_array() {
        Some(a) => a,
        None if doc.is_null() => return Ok(vec![]),
        None => {
            return Err(format!(
                "target {target}: {DEPENDENCIES_PATH} must be a list"
            ));
        }
    };
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let s = |k: &str| item.get(k).and_then(Json::as_str).map(str::to_string);
            let b = |k: &str| item.get(k).and_then(Json::as_bool).unwrap_or(false);
            let required = |k: &str| {
                s(k).ok_or_else(|| {
                    format!("target {target}: {DEPENDENCIES_PATH}[{i}] has no `{k}`")
                })
            };
            let ty = required("type")?;
            let kind = match ty.as_str() {
                "git" => Kind::Git {
                    git_ref: s("ref"),
                    subdir: s("subdir"),
                    submodules: b("submodules"),
                },
                "http" | "https" => Kind::Http { unpack: b("unpack") },
                "helm" => Kind::Helm {
                    chart_name: required("chart_name")?,
                    version: s("version"),
                    helm_path: s("helm_path"),
                },
                "oci" => Kind::Oci,
                other => {
                    return Err(format!(
                        "target {target}: {DEPENDENCIES_PATH}[{i}] has unknown type `{other}` (git, http, https, helm, oci)"
                    ));
                }
            };
            let output_path = required("output_path")?;
            Ok(Dependency {
                target: target.to_string(),
                kind,
                source: required("source")?,
                dest: normalise_join(output_root, &output_path),
                output_path,
                force_fetch: b("force_fetch"),
            })
        })
        .collect()
}

/// kapitan `normalise_join_path`: `os.path.normpath(os.path.join(root, path))`.
pub fn normalise_join(root: &Path, path: &str) -> PathBuf {
    let joined = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        root.join(path)
    };
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir) | Some(Component::Prefix(_)) => {}
                _ => out.push(".."),
            },
            c => out.push(c.as_os_str()),
        }
    }
    out
}

pub struct FetchOptions<'a> {
    pub repo_root: &'a Path,
    /// Every dependency is considered (`--fetch`); otherwise only items
    /// with `force_fetch: true`.
    pub fetch_all: bool,
    /// Overwrite what exists (`--force-fetch`).
    pub force: bool,
    pub dry_run: bool,
    pub parallelism: usize,
    /// Where versioned helm charts are kept across runs
    /// (`$XDG_CACHE_HOME/kapitan`).
    pub cache_dir: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FetchStatus {
    Fetched { ms: u64 },
    WouldFetch,
    Skipped,
    Failed { error: String },
}

#[derive(Clone, Debug, Serialize)]
pub struct FetchOutcome {
    #[serde(rename = "type")]
    pub kind: String,
    pub source: String,
    /// Where it landed, relative to the repository root when it is inside it.
    pub output_path: String,
    pub target: String,
    #[serde(flatten)]
    pub status: FetchStatus,
    pub reason: String,
}

impl FetchOutcome {
    pub fn failed(&self) -> bool {
        matches!(self.status, FetchStatus::Failed { .. })
    }
}

/// A dependency that will be fetched, with the effective force flag.
#[derive(Clone, Debug)]
struct Wanted {
    dep: Dependency,
    force: bool,
    reason: String,
}

/// Fetch every dependency that needs it. Duplicates (same source and
/// destination, declared by several targets) are fetched once; sources are
/// fetched once per run and copied to each destination; distinct sources
/// are fetched in parallel.
pub fn fetch(deps: Vec<Dependency>, opts: &FetchOptions) -> Vec<FetchOutcome> {
    let mut outcomes = Vec::new();
    let mut seen: std::collections::BTreeSet<(String, PathBuf)> = Default::default();
    // Groups keyed like kapitan: the source (git, http) or the chart identity (helm).
    let mut groups: BTreeMap<String, Vec<Wanted>> = BTreeMap::new();
    for dep in deps {
        if !seen.insert((dep.source.clone(), dep.dest.clone())) {
            continue;
        }
        if !opts.fetch_all && !dep.force_fetch {
            continue;
        }
        let reason = if opts.force {
            "forced (--force-fetch)".to_string()
        } else if dep.force_fetch {
            "forced (force_fetch: true)".to_string()
        } else if dep.dest.symlink_metadata().is_err() {
            format!("{} missing", display_path(opts.repo_root, &dep.dest))
        } else {
            outcomes.push(outcome(
                opts.repo_root,
                &dep,
                FetchStatus::Skipped,
                "already present",
            ));
            continue;
        };
        let force = opts.force || dep.force_fetch;
        let key = match &dep.kind {
            Kind::Helm {
                chart_name,
                version,
                helm_path,
            } => format!(
                "helm\0{}\0{chart_name}\0{}\0{}",
                dep.source,
                version.as_deref().unwrap_or(""),
                helm_path.as_deref().unwrap_or("")
            ),
            k => format!("{}\0{}", k.name(), dep.source),
        };
        groups
            .entry(key)
            .or_default()
            .push(Wanted { dep, force, reason });
    }

    if opts.dry_run {
        for w in groups.into_values().flatten() {
            outcomes.push(outcome(
                opts.repo_root,
                &w.dep,
                FetchStatus::WouldFetch,
                &w.reason,
            ));
        }
        return outcomes;
    }
    if groups.is_empty() {
        return outcomes;
    }

    let save_dir = std::env::temp_dir().join(format!(
        "kapitan-fetch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    if let Err(e) = std::fs::create_dir_all(&save_dir) {
        for w in groups.into_values().flatten() {
            outcomes.push(outcome(
                opts.repo_root,
                &w.dep,
                FetchStatus::Failed {
                    error: format!("cannot create {}: {e}", save_dir.display()),
                },
                &w.reason,
            ));
        }
        return outcomes;
    }
    let counter = AtomicUsize::new(0);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(opts.parallelism.max(1))
        .build();
    let run = || -> Vec<FetchOutcome> {
        groups
            .par_iter()
            .flat_map_iter(|(_, wanted)| fetch_group(wanted, &save_dir, opts, &counter))
            .collect()
    };
    let mut fetched = match pool {
        Ok(pool) => pool.install(run),
        Err(_) => run(),
    };
    let _ = std::fs::remove_dir_all(&save_dir);
    outcomes.append(&mut fetched);
    outcomes
}

fn outcome(root: &Path, dep: &Dependency, status: FetchStatus, reason: &str) -> FetchOutcome {
    FetchOutcome {
        kind: dep.kind.name().to_string(),
        source: dep.source.clone(),
        output_path: display_path(root, &dep.dest),
        target: dep.target.clone(),
        status,
        reason: reason.to_string(),
    }
}

fn display_path(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

/// One source, every destination that wants it.
fn fetch_group(
    wanted: &[Wanted],
    save_dir: &Path,
    opts: &FetchOptions,
    counter: &AtomicUsize,
) -> Vec<FetchOutcome> {
    let started = Instant::now();
    let first = &wanted[0].dep;
    let results: Vec<Result<(), String>> = match &first.kind {
        Kind::Git { .. } => fetch_git(wanted, save_dir),
        Kind::Http { .. } => fetch_http(wanted, save_dir, counter),
        Kind::Helm { .. } => fetch_helm(wanted, save_dir, opts, counter),
        Kind::Oci => wanted
            .iter()
            .map(|_| {
                Err(format!(
                    "Dependency {}: oci dependencies are not supported natively yet; fetch them with the reference kapitan",
                    first.source
                ))
            })
            .collect(),
    };
    let ms = started.elapsed().as_millis() as u64;
    wanted
        .iter()
        .zip(results)
        .map(|(w, r)| {
            let status = match r {
                Ok(()) => FetchStatus::Fetched { ms },
                Err(error) => FetchStatus::Failed { error },
            };
            outcome(opts.repo_root, &w.dep, status, &w.reason)
        })
        .collect()
}

/// Results for every destination; a failure of the shared step fails all of them.
fn all_failed(wanted: &[Wanted], e: String) -> Vec<Result<(), String>> {
    wanted.iter().map(|_| Err(e.clone())).collect()
}

fn hash8(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))[..8].to_string()
}

/// `os.path.dirname` / `os.path.basename` of a URL-ish source.
fn split_source(source: &str) -> (&str, &str) {
    match source.rsplit_once('/') {
        Some((dir, base)) => (dir, base),
        None => ("", source),
    }
}

fn git(args: &[&str], cwd: Option<&Path>) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null());
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    let out = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "git binary not found. git must be present in the PATH to fetch git dependencies"
                .to_string()
        } else {
            format!("cannot run git: {e}")
        }
    })?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// kapitan `fetch_git_dependency`: clone once, then per destination check
/// out the ref (the remote's default branch when none is given), update
/// submodules if asked, and copy the repository or its `subdir`.
fn fetch_git(wanted: &[Wanted], save_dir: &Path) -> Vec<Result<(), String>> {
    let source = &wanted[0].dep.source;
    let (dir, base) = split_source(source);
    let clone = save_dir.join(format!("{}{base}", hash8(dir)));
    let _ = std::fs::remove_dir_all(&clone);
    if let Err(e) = git(&["clone", source, &clone.to_string_lossy()], None) {
        return all_failed(
            wanted,
            format!("Dependency {source}: fetching unsuccessful\n{e}"),
        );
    }
    let default_branch = git(&["symbolic-ref", "--short", "HEAD"], Some(&clone))
        .ok()
        .or_else(|| {
            git(&["symbolic-ref", "refs/remotes/origin/HEAD"], Some(&clone))
                .ok()
                .and_then(|r| r.rsplit('/').next().map(str::to_string))
        });
    wanted
        .iter()
        .map(|w| {
            let Kind::Git {
                git_ref,
                subdir,
                submodules,
            } = &w.dep.kind
            else {
                unreachable!()
            };
            if let Some(r) = git_ref.as_deref().or(default_branch.as_deref()) {
                git(&["checkout", "--quiet", r], Some(&clone))
                    .map_err(|e| format!("Dependency {source}: cannot check out `{r}`\n{e}"))?;
            }
            if *submodules {
                git(&["submodule", "update", "--init"], Some(&clone))
                    .map_err(|e| format!("Dependency {source}: submodule update failed\n{e}"))?;
            }
            let src = match subdir {
                Some(sub) => {
                    let full = clone.join(sub);
                    if !full.is_dir() {
                        return Err(format!(
                            "Dependency {source}: subdir {sub} not found in repo"
                        ));
                    }
                    full
                }
                None => clone.clone(),
            };
            if w.force {
                copy_tree(&src, &w.dep.dest)
            } else {
                safe_copy_tree(&src, &w.dep.dest)
            }
            .map(|_| ())
            .map_err(|e| {
                format!(
                    "Dependency {source}: cannot copy to {}: {e}",
                    w.dep.dest.display()
                )
            })
        })
        .collect()
}

fn download(source: &str) -> Result<(Vec<u8>, Option<String>), String> {
    let agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .http_status_as_error(true)
        .user_agent(format!("kapitan/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent();
    let mut resp = agent.get(source).call().map_err(|e| e.to_string())?;
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        });
    let bytes = resp
        .body_mut()
        .with_config()
        .limit(u64::MAX)
        .read_to_vec()
        .map_err(|e| e.to_string())?;
    Ok((bytes, content_type))
}

/// kapitan `fetch_http_dependency`: download once, then per destination
/// either unpack the archive into it or save the file there.
fn fetch_http(
    wanted: &[Wanted],
    save_dir: &Path,
    counter: &AtomicUsize,
) -> Vec<Result<(), String>> {
    let source = &wanted[0].dep.source;
    let (dir, base) = split_source(source);
    let file = save_dir.join(format!("{}{base}", hash8(dir)));
    let content_type = match download(source) {
        Ok((bytes, ct)) => match std::fs::write(&file, bytes) {
            Ok(()) => ct,
            Err(e) => {
                return all_failed(
                    wanted,
                    format!("Dependency {source}: cannot save download: {e}"),
                );
            }
        },
        Err(e) => {
            return all_failed(
                wanted,
                format!("Dependency {source}: fetching unsuccessful\n{e}"),
            );
        }
    };
    wanted
        .iter()
        .map(|w| {
            let Kind::Http { unpack } = &w.dep.kind else {
                unreachable!()
            };
            let dest = &w.dep.dest;
            if *unpack {
                std::fs::create_dir_all(dest).map_err(|e| {
                    format!("Dependency {source}: cannot create {}: {e}", dest.display())
                })?;
                let unpacked = if w.force {
                    unpack_file(&file, dest, content_type.as_deref())?
                } else {
                    let tmp = save_dir.join(format!(
                        "extracted-{}",
                        counter.fetch_add(1, Ordering::Relaxed)
                    ));
                    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
                    let ok = unpack_file(&file, &tmp, content_type.as_deref())?;
                    if ok {
                        safe_copy_tree(&tmp, dest).map_err(|e| {
                            format!(
                                "Dependency {source}: cannot copy to {}: {e}",
                                dest.display()
                            )
                        })?;
                    }
                    let _ = std::fs::remove_dir_all(&tmp);
                    ok
                };
                if !unpacked {
                    return Err(format!(
                        "Dependency {source}: Content-Type {} is not supported for unpack",
                        content_type.as_deref().unwrap_or("unknown")
                    ));
                }
                Ok(())
            } else {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        format!(
                            "Dependency {source}: cannot create {}: {e}",
                            parent.display()
                        )
                    })?;
                }
                if w.force || !dest.is_file() {
                    std::fs::copy(&file, dest).map_err(|e| {
                        format!("Dependency {source}: cannot write {}: {e}", dest.display())
                    })?;
                }
                Ok(())
            }
        })
        .collect()
}

/// kapitan `unpack_downloaded_file`: tar, zip and gzipped tar by content
/// type (with the archive's magic bytes deciding when the type is generic).
/// `Ok(false)` when the type is not one that can be unpacked.
pub fn unpack_file(file: &Path, dest: &Path, content_type: Option<&str>) -> Result<bool, String> {
    let bytes = std::fs::read(file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    let is_zip = bytes.starts_with(b"PK\x03\x04");
    let is_gzip = bytes.starts_with(&[0x1f, 0x8b]);
    let name = file.to_string_lossy();
    let content_type = match content_type {
        None | Some("application/octet-stream") if is_zip => "application/zip",
        Some(ct) => ct,
        None => "",
    };
    let untar = |gz: bool| -> Result<(), String> {
        let mut archive: tar::Archive<Box<dyn Read>> = if gz {
            tar::Archive::new(Box::new(flate2::read::GzDecoder::new(&bytes[..])))
        } else {
            tar::Archive::new(Box::new(&bytes[..]))
        };
        archive.set_overwrite(true);
        archive
            .unpack(dest)
            .map_err(|e| format!("cannot unpack {name}: {e}"))
    };
    match content_type {
        "application/x-tar" => {
            untar(is_gzip)?;
            Ok(true)
        }
        "application/zip" => {
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..]))
                .map_err(|e| format!("cannot open {name}: {e}"))?;
            zip.extract(dest)
                .map_err(|e| format!("cannot unpack {name}: {e}"))?;
            Ok(true)
        }
        "application/gzip"
        | "application/octet-stream"
        | "application/x-gzip"
        | "application/x-compressed"
        | "application/x-compressed-tar" => {
            if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
                untar(is_gzip)?;
                Ok(true)
            } else {
                Ok(false)
            }
        }
        _ => Ok(false),
    }
}

/// kapitan `fetch_helm_chart`: `helm pull --untar` once per chart identity,
/// then copy the chart directory to each destination. Charts with a version
/// are kept under `$XDG_CACHE_HOME/kapitan/charts` (a published version is
/// immutable); forced fetches pull again.
fn fetch_helm(
    wanted: &[Wanted],
    save_dir: &Path,
    opts: &FetchOptions,
    counter: &AtomicUsize,
) -> Vec<Result<(), String>> {
    let first = &wanted[0];
    let Kind::Helm {
        chart_name,
        version,
        helm_path,
    } = &first.dep.kind
    else {
        unreachable!()
    };
    let repo = &first.dep.source;
    let label = format!(
        "Dependency helm chart {chart_name} and version {}",
        version.as_deref().unwrap_or("latest")
    );
    let cached = match version {
        Some(v) => opts
            .cache_dir
            .join("charts")
            .join(hash8(repo))
            .join(format!("{chart_name}-{v}")),
        None => save_dir
            .join(hash8(repo))
            .join(format!("{chart_name}-latest")),
    };
    let force = wanted.iter().any(|w| w.force);
    if force || !cached.is_dir() {
        let tmp = save_dir.join(format!(
            "helm-{}-{}",
            hash8(repo),
            counter.fetch_add(1, Ordering::Relaxed)
        ));
        if let Err(e) = std::fs::create_dir_all(&tmp) {
            return all_failed(
                wanted,
                format!("{label}: cannot create {}: {e}", tmp.display()),
            );
        }
        let mut args = vec![
            "pull".to_string(),
            "--destination".to_string(),
            tmp.to_string_lossy().into_owned(),
            "--untar".to_string(),
        ];
        if let Some(v) = version {
            args.push("--version".into());
            args.push(v.clone());
        }
        if repo.starts_with("oci://") {
            args.push(repo.clone());
        } else {
            args.push("--repo".into());
            args.push(repo.clone());
            args.push(chart_name.clone());
        }
        let binary = helm_binary(helm_path.as_deref());
        if let Err(e) = run_helm(&binary, &args, opts.repo_root) {
            return all_failed(wanted, format!("{label}: {}", e.trim()));
        }
        let pulled = tmp.join(chart_name);
        if !pulled.is_dir() {
            return all_failed(
                wanted,
                format!(
                    "{label}: helm pull produced no directory named {chart_name} (is chart_name the chart's name?)"
                ),
            );
        }
        let _ = std::fs::remove_dir_all(&cached);
        if let Some(parent) = cached.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            return all_failed(
                wanted,
                format!("{label}: cannot create {}: {e}", parent.display()),
            );
        }
        if std::fs::rename(&pulled, &cached).is_err()
            && let Err(e) = copy_tree(&pulled, &cached)
        {
            return all_failed(wanted, format!("{label}: cannot cache chart: {e}"));
        }
    }
    wanted
        .iter()
        .map(|w| {
            let dest = &w.dep.dest;
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("{label}: cannot create {}: {e}", parent.display()))?;
            }
            if w.force {
                copy_tree(&cached, dest)
            } else {
                safe_copy_tree(&cached, dest)
            }
            .map(|_| ())
            .map_err(|e| format!("{label}: cannot copy to {}: {e}", dest.display()))
        })
        .collect()
}

/// kapitan `safe_copy_tree`: copy `src` into `dst` without overwriting any
/// existing file and without copying entries whose name starts with `.`.
/// Returns how many files were copied.
pub fn safe_copy_tree(src: &Path, dst: &Path) -> std::io::Result<usize> {
    if !src.is_dir() {
        return Err(std::io::Error::other(format!(
            "Cannot copy tree {}: not a directory",
            src.display()
        )));
    }
    std::fs::create_dir_all(dst)?;
    let mut copied = 0;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        if from.is_dir() {
            copied += safe_copy_tree(&from, &to)?;
        } else if !to.is_file() {
            std::fs::copy(&from, &to)?;
            copied += 1;
        }
    }
    Ok(copied)
}

/// kapitan `copy_tree(clobber_files=True)`: copy everything, dot-entries
/// included, replacing existing files. Returns how many files were copied.
pub fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<usize> {
    if !src.is_dir() {
        return Err(std::io::Error::other(format!(
            "Cannot copy tree {}: not a directory",
            src.display()
        )));
    }
    if dst.exists() && !dst.is_dir() {
        return Err(std::io::Error::other(format!(
            "Cannot copy tree to {}: destination exists but not a directory",
            dst.display()
        )));
    }
    std::fs::create_dir_all(dst)?;
    let mut copied = 0;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copied += copy_tree(&from, &to)?;
        } else {
            if to.is_file() {
                // Read-only files (git pack files) cannot be overwritten in place.
                std::fs::remove_file(&to)?;
            }
            std::fs::copy(&from, &to)?;
            copied += 1;
        }
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn tmp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kapitan-fetch-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    fn opts(root: &Path, fetch_all: bool, force: bool) -> FetchOptions<'_> {
        FetchOptions {
            repo_root: root,
            fetch_all,
            force,
            dry_run: false,
            parallelism: 2,
            cache_dir: root.join(".cache"),
        }
    }

    #[test]
    fn parses_every_kind_and_normalises_paths() {
        let root = Path::new("/repo");
        let deps = dependencies(
            "t",
            &json!([
                {"type": "git", "source": "https://x/y.git", "output_path": "system/lib/", "ref": "main", "subdir": "lib/", "force_fetch": true},
                {"type": "https", "source": "https://x/f.tgz", "output_path": "./a/../b/c", "unpack": true},
                {"type": "helm", "source": "https://charts", "output_path": "charts/x/1.0", "chart_name": "x", "version": "1.0"},
                {"type": "oci", "source": "ghcr.io/a/b:1", "output_path": "oci"},
            ]),
            root,
        )
        .unwrap();
        assert_eq!(deps.len(), 4);
        assert_eq!(
            deps[0].kind,
            Kind::Git {
                git_ref: Some("main".into()),
                subdir: Some("lib/".into()),
                submodules: false
            }
        );
        assert!(deps[0].force_fetch);
        assert_eq!(deps[0].dest, PathBuf::from("/repo/system/lib"));
        assert_eq!(deps[1].kind, Kind::Http { unpack: true });
        assert_eq!(deps[1].dest, PathBuf::from("/repo/b/c"));
        assert_eq!(
            deps[2].kind,
            Kind::Helm {
                chart_name: "x".into(),
                version: Some("1.0".into()),
                helm_path: None
            }
        );
        assert_eq!(deps[3].kind, Kind::Oci);
        assert!(dependencies("t", &json!(null), root).unwrap().is_empty());
        assert!(
            dependencies(
                "t",
                &json!([{"type": "svn", "source": "s", "output_path": "o"}]),
                root
            )
            .unwrap_err()
            .contains("unknown type `svn`")
        );
        assert!(
            dependencies(
                "t",
                &json!([{"type": "helm", "source": "s", "output_path": "o"}]),
                root
            )
            .unwrap_err()
            .contains("no `chart_name`")
        );
    }

    #[test]
    fn normalise_join_matches_normpath() {
        assert_eq!(
            normalise_join(Path::new("/r"), "a/./b/"),
            PathBuf::from("/r/a/b")
        );
        assert_eq!(normalise_join(Path::new("/r"), "../x"), PathBuf::from("/x"));
        assert_eq!(
            normalise_join(Path::new("/r"), "/abs/p"),
            PathBuf::from("/abs/p")
        );
        assert_eq!(
            normalise_join(Path::new("r"), "../../x"),
            PathBuf::from("../x")
        );
    }

    #[test]
    fn safe_copy_never_overwrites_and_skips_dotfiles() {
        let dir = tmp("safe-copy");
        let (src, dst) = (dir.join("src"), dir.join("dst"));
        write(&src.join("a.txt"), "new");
        write(&src.join("sub/b.txt"), "b");
        write(&src.join(".git/HEAD"), "ref");
        write(&dst.join("a.txt"), "old");
        assert_eq!(safe_copy_tree(&src, &dst).unwrap(), 1);
        assert_eq!(read(&dst.join("a.txt")), "old");
        assert_eq!(read(&dst.join("sub/b.txt")), "b");
        assert!(!dst.join(".git").exists());

        assert_eq!(copy_tree(&src, &dst).unwrap(), 3);
        assert_eq!(read(&dst.join("a.txt")), "new");
        assert!(dst.join(".git/HEAD").exists());
        assert!(copy_tree(&src.join("a.txt"), &dst).is_err());
    }

    fn tar_gz(files: &[(&str, &str)]) -> Vec<u8> {
        let enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut tar = tar::Builder::new(enc);
        for (name, text) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(text.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            tar.append_data(&mut h, name, text.as_bytes()).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }

    fn zip_bytes(files: &[(&str, &str)]) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, text) in files {
            z.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(text.as_bytes()).unwrap();
        }
        z.finish().unwrap().into_inner()
    }

    #[test]
    fn unpack_dispatches_on_content_type_and_magic() {
        let dir = tmp("unpack");
        let tgz = dir.join("f.tgz");
        std::fs::write(&tgz, tar_gz(&[("d/x.txt", "x")])).unwrap();
        let out = dir.join("out1");
        assert!(unpack_file(&tgz, &out, Some("application/gzip")).unwrap());
        assert_eq!(read(&out.join("d/x.txt")), "x");
        // A gzipped tar served as plain tar still unpacks (tarfile.open sniffs too).
        assert!(unpack_file(&tgz, &dir.join("out2"), Some("application/x-tar")).unwrap());
        // gzip with an unknown extension is not unpacked.
        let gz = dir.join("f.bin");
        std::fs::copy(&tgz, &gz).unwrap();
        assert!(!unpack_file(&gz, &dir.join("out3"), Some("application/gzip")).unwrap());
        // zip by magic when the type is generic.
        let zf = dir.join("f.dat");
        std::fs::write(&zf, zip_bytes(&[("z/y.txt", "y")])).unwrap();
        assert!(unpack_file(&zf, &dir.join("out4"), Some("application/octet-stream")).unwrap());
        assert_eq!(read(&dir.join("out4/z/y.txt")), "y");
        assert!(!unpack_file(&zf, &dir.join("out5"), Some("text/plain")).unwrap());
    }

    fn git_ok(args: &[&str], cwd: &Path) {
        git(args, Some(cwd)).unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    }

    fn make_repo(dir: &Path) -> String {
        std::fs::create_dir_all(dir).unwrap();
        git_ok(&["init", "-q", "-b", "main"], dir);
        git_ok(&["config", "user.email", "t@t"], dir);
        git_ok(&["config", "user.name", "t"], dir);
        write(&dir.join("lib/a.py"), "main");
        write(&dir.join("top.txt"), "top");
        git_ok(&["add", "."], dir);
        git_ok(&["commit", "-q", "-m", "one"], dir);
        git_ok(&["tag", "v1"], dir);
        write(&dir.join("lib/a.py"), "v2");
        git_ok(&["commit", "-q", "-am", "two"], dir);
        dir.to_string_lossy().into_owned()
    }

    #[test]
    fn git_dependency_checks_out_ref_and_copies_subdir() {
        let dir = tmp("git");
        let source = make_repo(&dir.join("origin"));
        let root = dir.join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let deps = dependencies(
            "t",
            &json!([
                {"type": "git", "source": source, "output_path": "system/lib", "subdir": "lib"},
                {"type": "git", "source": source, "output_path": "system/v1", "ref": "v1", "subdir": "lib"},
                {"type": "git", "source": source, "output_path": "system/all"},
                {"type": "git", "source": source, "output_path": "system/lib"},
                {"type": "git", "source": source, "output_path": "system/missing", "subdir": "nope"},
            ]),
            &root,
        )
        .unwrap();
        let out = fetch(deps, &opts(&root, true, false));
        // The duplicate (same source and destination) is not reported.
        assert_eq!(out.len(), 4, "{out:?}");
        assert_eq!(read(&root.join("system/lib/a.py")), "v2");
        assert_eq!(read(&root.join("system/v1/a.py")), "main");
        assert_eq!(read(&root.join("system/all/top.txt")), "top");
        assert!(
            !root.join("system/all/.git").exists(),
            "safe copy skips dot entries"
        );
        let failed: Vec<_> = out.iter().filter(|o| o.failed()).collect();
        assert_eq!(failed.len(), 1);
        assert!(
            matches!(&failed[0].status, FetchStatus::Failed { error } if error.contains("subdir nope not found"))
        );
        assert_eq!(failed[0].output_path, "system/missing");
        assert!(out.iter().any(|o| o.reason == "system/lib missing"));

        // Present outputs are left alone without --force-fetch...
        write(&root.join("system/lib/a.py"), "edited");
        let deps = dependencies(
            "t",
            &json!([{"type": "git", "source": source, "output_path": "system/lib", "subdir": "lib"}]),
            &root,
        )
        .unwrap();
        let out = fetch(deps.clone(), &opts(&root, true, false));
        assert!(matches!(out[0].status, FetchStatus::Skipped));
        assert_eq!(read(&root.join("system/lib/a.py")), "edited");
        // ...and nothing at all is considered without --fetch unless the item forces it.
        assert!(fetch(deps.clone(), &opts(&root, false, false)).is_empty());
        // --force-fetch overwrites.
        let out = fetch(deps, &opts(&root, true, true));
        assert!(
            matches!(out[0].status, FetchStatus::Fetched { .. }),
            "{out:?}"
        );
        assert_eq!(out[0].reason, "forced (--force-fetch)");
        assert_eq!(read(&root.join("system/lib/a.py")), "v2");
    }

    #[test]
    fn item_force_fetch_applies_without_fetch_flag() {
        let dir = tmp("git-force-item");
        let source = make_repo(&dir.join("origin"));
        let root = dir.join("repo");
        write(&root.join("system/lib/a.py"), "edited");
        let deps = dependencies(
            "t",
            &json!([
                {"type": "git", "source": source, "output_path": "system/lib", "subdir": "lib", "force_fetch": true},
                {"type": "git", "source": source, "output_path": "system/other", "subdir": "lib"},
            ]),
            &root,
        )
        .unwrap();
        let out = fetch(deps, &opts(&root, false, false));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].reason, "forced (force_fetch: true)");
        assert_eq!(read(&root.join("system/lib/a.py")), "v2");
        assert!(!root.join("system/other").exists());
    }

    #[test]
    fn dry_run_reports_without_fetching() {
        let dir = tmp("dry");
        let root = dir.join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let deps = dependencies(
            "t",
            &json!([{"type": "git", "source": "https://example.invalid/x.git", "output_path": "x"}]),
            &root,
        )
        .unwrap();
        let out = fetch(
            deps,
            &FetchOptions {
                dry_run: true,
                ..opts(&root, true, false)
            },
        );
        assert!(matches!(out[0].status, FetchStatus::WouldFetch));
        assert_eq!(out[0].reason, "x missing");
        assert!(!root.join("x").exists());
    }

    /// A one-shot HTTP server on localhost serving `body` with `content_type`.
    fn serve(body: Vec<u8>, content_type: &str, hits: usize) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let content_type = content_type.to_string();
        std::thread::spawn(move || {
            for _ in 0..hits {
                let (mut s, _) = listener.accept().unwrap();
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                s.write_all(head.as_bytes()).unwrap();
                s.write_all(&body).unwrap();
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn http_dependency_saves_or_unpacks() {
        let dir = tmp("http");
        let root = dir.join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let base = serve(
            tar_gz(&[("pkg/f.txt", "hello")]),
            "application/x-gzip; charset=binary",
            1,
        );
        let deps = dependencies(
            "t",
            &json!([
                {"type": "https", "source": format!("{base}/dl/pkg.tgz"), "output_path": "vendor/pkg", "unpack": true},
                {"type": "https", "source": format!("{base}/dl/pkg.tgz"), "output_path": "vendor/raw.tgz"},
            ]),
            &root,
        )
        .unwrap();
        // Two destinations, one download.
        let out = fetch(deps, &opts(&root, true, false));
        assert!(out.iter().all(|o| !o.failed()), "{out:?}");
        assert_eq!(read(&root.join("vendor/pkg/pkg/f.txt")), "hello");
        assert!(root.join("vendor/raw.tgz").is_file());

        let base = serve(b"plain".to_vec(), "text/plain", 1);
        let deps = dependencies(
            "t",
            &json!([{"type": "http", "source": format!("{base}/x.txt"), "output_path": "vendor/x", "unpack": true}]),
            &root,
        )
        .unwrap();
        let out = fetch(deps, &opts(&root, true, false));
        assert!(
            matches!(&out[0].status, FetchStatus::Failed { error } if error.contains("not supported for unpack"))
        );

        let deps = dependencies(
            "t",
            &json!([{"type": "http", "source": "http://127.0.0.1:1/none", "output_path": "vendor/none"}]),
            &root,
        )
        .unwrap();
        let out = fetch(deps, &opts(&root, true, false));
        assert!(
            matches!(&out[0].status, FetchStatus::Failed { error } if error.contains("fetching unsuccessful"))
        );
    }

    #[test]
    fn helm_dependency_pulls_once_and_copies() {
        let dir = tmp("helm");
        let root = dir.join("repo");
        std::fs::create_dir_all(&root).unwrap();
        // A stand-in helm that records its arguments and "untars" a chart.
        let fake = dir.join("helm");
        write(
            &fake,
            "#!/bin/sh\necho \"$@\" >> \"$(dirname \"$0\")/calls\"\nwhile [ $# -gt 0 ]; do case $1 in --destination) dest=$2; shift;; esac; shift; done\nmkdir -p \"$dest/mychart/templates\"\necho 'name: mychart' > \"$dest/mychart/Chart.yaml\"\n",
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let helm_path = fake.to_string_lossy().into_owned();
        let deps = dependencies(
            "t",
            &json!([
                {"type": "helm", "source": "https://charts.example/repo", "chart_name": "mychart", "version": "1.2.3", "output_path": "charts/mychart/1.2.3", "helm_path": helm_path},
                {"type": "helm", "source": "https://charts.example/repo", "chart_name": "mychart", "version": "1.2.3", "output_path": "charts/copy", "helm_path": helm_path},
                {"type": "helm", "source": "oci://ghcr.io/org/mychart", "chart_name": "mychart", "output_path": "charts/oci", "helm_path": helm_path},
            ]),
            &root,
        )
        .unwrap();
        let out = fetch(deps, &opts(&root, true, false));
        assert!(out.iter().all(|o| !o.failed()), "{out:?}");
        assert_eq!(
            read(&root.join("charts/mychart/1.2.3/Chart.yaml")),
            "name: mychart\n"
        );
        assert_eq!(
            read(&root.join("charts/copy/Chart.yaml")),
            "name: mychart\n"
        );
        assert!(root.join("charts/oci/Chart.yaml").is_file());
        let calls = read(&dir.join("calls"));
        let lines: Vec<&str> = calls.lines().collect();
        assert_eq!(lines.len(), 2, "one pull per chart identity: {calls}");
        assert!(
            lines.iter().any(|l| l
                .contains("--untar --version 1.2.3 --repo https://charts.example/repo mychart")),
            "{calls}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.ends_with("--untar oci://ghcr.io/org/mychart")),
            "{calls}"
        );
        assert!(root.join(".cache/charts").is_dir());

        // Cached: a new destination for the same version needs no pull.
        let deps = dependencies(
            "t",
            &json!([{"type": "helm", "source": "https://charts.example/repo", "chart_name": "mychart", "version": "1.2.3", "output_path": "charts/again", "helm_path": helm_path}]),
            &root,
        )
        .unwrap();
        let out = fetch(deps.clone(), &opts(&root, true, false));
        assert!(matches!(out[0].status, FetchStatus::Fetched { .. }));
        assert_eq!(read(&dir.join("calls")).lines().count(), 2);
        // Forced: pulled again.
        let out = fetch(deps, &opts(&root, true, true));
        assert!(matches!(out[0].status, FetchStatus::Fetched { .. }));
        assert_eq!(read(&dir.join("calls")).lines().count(), 3);
    }
}
