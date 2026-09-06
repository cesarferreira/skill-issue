#![allow(dead_code)]

use anyhow::{Context, Result, anyhow, bail};
use blake3::Hasher;
use dialoguer::{Confirm, Input, Select, theme::ColorfulTheme};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Serialize;
use similar::TextDiff;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, BufReader, IsTerminal, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use walkdir::{DirEntry, WalkDir};

mod apply;
mod cli;
mod model;
mod theme;
mod tui;

pub use cli::Cli;
use cli::{Command, ConfigCommand, LinkArgs, TargetCommand, UnlinkArgs};
pub use model::{
    CanonicalSkill, Config, Installation, InstallationKind, ScanResult, SkillGroup, SkillStatus,
    TargetConfig,
};

const EXIT_OK: u8 = 0;
const EXIT_ISSUES: u8 = 2;
const EXIT_CONFLICTS: u8 = 3;
const EXIT_CONFIG: u8 = 4;
static TEMP_SEQUENCE: AtomicUsize = AtomicUsize::new(0);
static VERBOSITY: AtomicU8 = AtomicU8::new(0);
static JSON_OUTPUT: AtomicBool = AtomicBool::new(false);
static ASSUME_YES: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug)]
struct AdoptionPlan {
    display_name: String,
    canonical: PathBuf,
    fingerprint: String,
    transfer: CanonicalTransfer,
    replacements: Vec<Replacement>,
    preserved: Vec<PreservedCopy>,
    ignore: Vec<String>,
    relative_links: bool,
}

#[derive(Clone, Debug)]
struct PreservedCopy {
    source: PathBuf,
    destination: PathBuf,
    fingerprint: String,
    target: String,
}

#[derive(Default)]
struct MigrationStats {
    skills: usize,
    installations: usize,
    adopted: usize,
    skipped: usize,
    links: usize,
    preserved: usize,
}

#[derive(Clone, Debug)]
enum CanonicalTransfer {
    Existing,
    Move { source: PathBuf },
    Copy { source: PathBuf, temporary: PathBuf },
}

#[derive(Clone, Debug)]
struct Replacement {
    original: PathBuf,
    backup: Option<PathBuf>,
}

/// Parse command line arguments, honouring `--no-color` for help and errors.
pub fn parse_cli() -> Cli {
    cli::parse()
}

/// Coloured `Error:` prefix used by the binary's top-level handler.
pub fn error_prefix() -> String {
    theme::error_prefix()
}

pub fn run(cli: Cli) -> Result<u8> {
    set_color(cli.no_color);
    VERBOSITY.store(cli.verbose, Ordering::Relaxed);
    JSON_OUTPUT.store(cli.json, Ordering::Relaxed);
    ASSUME_YES.store(cli.yes, Ordering::Relaxed);
    match cli.command {
        Some(Command::Setup {
            root,
            targets,
            ignore,
        }) => setup_command(root, targets, ignore, cli.dry_run),
        Some(command) => {
            let config = match Config::load() {
                Ok(config) => config,
                Err(error) if is_missing_config(&error) => {
                    eprintln!(
                        "{} skill-issue is not configured. Run {}.",
                        theme::warn_err(),
                        theme::hint_err("si setup ~/code/skills")
                    );
                    return Ok(EXIT_CONFIG);
                }
                Err(error) => {
                    eprintln!("{} Invalid configuration: {error:#}", theme::bad_err());
                    return Ok(EXIT_CONFIG);
                }
            };
            if let Err(error) = validate_config(&config) {
                eprintln!("{} Invalid configuration: {error:#}", theme::bad_err());
                return Ok(EXIT_CONFIG);
            }
            match command {
                Command::Tui { project } => tui::run(config, project, cli.dry_run, cli.no_color),
                Command::Sync => apply::run(&config, cli.dry_run),
                Command::Status => status_command(&config, true),
                Command::Diff { skill, content } => diff_command(&config, &skill, content),
                Command::Targets { command } => targets_command(config, command, cli.dry_run),
                Command::Config { command } => config_command(config, command, cli.dry_run),
                Command::Setup { .. } => unreachable!(),
            }
        }
        None => default_command(),
    }
}

fn set_color(no_color: bool) {
    if no_color || std::env::var_os("NO_COLOR").is_some() {
        console::set_colors_enabled(false);
        console::set_colors_enabled_stderr(false);
    }
}

impl Config {
    fn path() -> Result<PathBuf> {
        if let Some(path) = std::env::var_os("SKILL_ISSUE_CONFIG") {
            return Ok(PathBuf::from(path));
        }
        // Keep accepting the original variable so existing installations do not break.
        if let Some(path) = std::env::var_os("SKILLISSUE_CONFIG") {
            return Ok(PathBuf::from(path));
        }
        let dir = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|home| home.join(".config")))
            .ok_or_else(|| anyhow!("could not determine the configuration directory"))?;
        let current = dir.join("skill-issue/config.toml");
        let legacy = dir.join("skillissue/config.toml");
        Ok(if current.exists() || !legacy.exists() {
            current
        } else {
            legacy
        })
    }

    fn load() -> Result<Self> {
        let path = Self::path()?;
        let text = fs::read_to_string(&path)
            .with_context(|| format!("configuration not found at {}", path.display()))?;
        let mut config: Self = toml::from_str(&text)
            .with_context(|| format!("invalid configuration at {}", path.display()))?;
        config.root = absolute_path(&config.root)?;
        for target in config.targets.values_mut() {
            target.path = absolute_path(&target.path)?;
        }
        Ok(config)
    }

    fn save(&self) -> Result<()> {
        let path = Self::path()?;
        let parent = path
            .parent()
            .ok_or_else(|| anyhow!("invalid configuration path"))?;
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        let text = toml::to_string_pretty(self)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(text.as_bytes())?;
        temp.as_file().sync_all()?;
        temp.persist(&path).map_err(|error| error.error)?;
        Ok(())
    }
}

fn is_missing_config(error: &anyhow::Error) -> bool {
    error.chain().any(|source| {
        source
            .downcast_ref::<io::Error>()
            .is_some_and(|e| e.kind() == io::ErrorKind::NotFound)
    })
}

fn expand_path(path: &Path) -> Result<PathBuf> {
    let text = path.to_string_lossy();
    if text == "~" || text.starts_with("~/") {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("could not determine home directory"))?;
        return Ok(if text == "~" {
            home
        } else {
            home.join(&text[2..])
        });
    }
    Ok(path.to_path_buf())
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    let expanded = expand_path(path)?;
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()?.join(expanded)
    };
    Ok(normalize_lexical(&absolute))
}

fn normalize_lexical(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

fn best_effort_canonical(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(file_name)) => best_effort_canonical(parent).join(file_name),
        _ => path.to_path_buf(),
    }
}

fn validate_config(config: &Config) -> Result<()> {
    build_ignore_set(&config.ignore)?;
    for (id, target) in config.targets.iter().filter(|(_, t)| t.enabled) {
        let root = best_effort_canonical(&config.root);
        let target_path = best_effort_canonical(&target.path);
        if root == target_path
            || path_contains(&root, &target_path)
            || path_contains(&target_path, &root)
        {
            bail!(
                "unsafe topology: canonical root {} overlaps target {id} at {}",
                config.root.display(),
                target.path.display()
            );
        }
    }
    Ok(())
}

fn path_contains(parent: &Path, child: &Path) -> bool {
    child != parent && child.starts_with(parent)
}

fn init(
    root: Option<PathBuf>,
    target_args: &[String],
    ignore_args: &[String],
    dry_run: bool,
) -> Result<u8> {
    let config = initial_setup_config(root, target_args, ignore_args)?;
    let root = config.root.clone();
    let config_path = Config::path()?;
    if config_path.exists() {
        let current = Config::load()?;
        if current.root == config.root {
            println!("skill-issue is already configured with {}", root.display());
            return Ok(EXIT_OK);
        }
        bail!(
            "configuration already exists at {}; use `si config set-root`",
            config_path.display()
        );
    }
    println!("{}", theme::heading("INIT PLAN"));
    println!(
        "{}  {}",
        theme::action("CREATE"),
        theme::path_text(&root.display().to_string())
    );
    println!(
        "{}   {}",
        theme::action("WRITE"),
        theme::path_text(&config_path.display().to_string())
    );
    if dry_run {
        println!("{}", theme::dim("Dry run; no files changed."));
        return Ok(EXIT_OK);
    }
    fs::create_dir_all(&root)
        .with_context(|| format!("create canonical root {}", root.display()))?;
    config.save()?;
    println!(
        "{} Canonical directory {}",
        theme::ok(),
        theme::path_text(&display_path(&root))
    );
    println!("{}", theme::heading("DETECTED"));
    let mut detected: Vec<_> = config.targets.keys().collect();
    detected.sort_by_key(|id| target_rank(id));
    for id in detected {
        println!("  {} {}", theme::ok(), theme::agent(&target_label(id)));
    }
    if VERBOSITY.load(Ordering::Relaxed) > 0 {
        println!("{} Configuration saved", theme::ok());
    }
    Ok(EXIT_OK)
}

fn initial_setup_config(
    root: Option<PathBuf>,
    target_args: &[String],
    ignore_args: &[String],
) -> Result<Config> {
    let root = match root {
        Some(path) => absolute_path(&path)?,
        None if io::stdin().is_terminal() => {
            let default = dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("skills");
            let entered: String = Input::with_theme(&ColorfulTheme::default())
                .with_prompt("Where should your canonical skills live?")
                .default(default.display().to_string())
                .interact_text()?;
            absolute_path(Path::new(&entered))?
        }
        None => bail!("a canonical root is required when stdin is not interactive"),
    };
    let mut targets = BTreeMap::new();
    if let Some(home) = dirs::home_dir() {
        for (id, relative) in [
            ("claude", ".claude/skills"),
            ("codex", ".codex/skills"),
            ("gemini", ".gemini/skills"),
            ("agents", ".agents/skills"),
            ("opencode", ".config/opencode/skills"),
            ("hermes", ".hermes/skills"),
            ("cursor", ".cursor/skills"),
        ] {
            let path = home.join(relative);
            if path.is_dir() {
                targets.insert(
                    id.to_string(),
                    TargetConfig {
                        path,
                        enabled: true,
                    },
                );
            }
        }
    }
    let config = merge_setup_inputs(
        Config {
            root: root.clone(),
            targets,
            relative_links: false,
            ignore: Vec::new(),
        },
        target_args,
        ignore_args,
    )?;
    validate_config(&config)?;
    Ok(config)
}

fn setup_command(
    root: Option<PathBuf>,
    target_args: Vec<String>,
    ignore_args: Vec<String>,
    dry_run: bool,
) -> Result<u8> {
    let config = match Config::load() {
        Ok(config) => {
            if let Some(root) = root {
                let requested = absolute_path(&root)?;
                if requested != config.root {
                    bail!(
                        "configuration already uses {}; run `si config set-root {}` first",
                        config.root.display(),
                        requested.display()
                    );
                }
            }
            let merged = merge_setup_inputs(config, &target_args, &ignore_args)?;
            if !dry_run {
                merged.save()?;
            }
            merged
        }
        Err(error) if is_missing_config(&error) => {
            if dry_run {
                initial_setup_config(root, &target_args, &ignore_args)?
            } else {
                init(root, &target_args, &ignore_args, false)?;
                Config::load()?
            }
        }
        Err(error) => return Err(error),
    };
    validate_config(&config)?;
    let result = scan(&config)?;
    adopt_from_scan(&config, &result, None, dry_run, true)?;
    if dry_run {
        return Ok(EXIT_OK);
    }
    apply::run(&config, false)?;
    Ok(result_exit(&scan(&config)?))
}

fn merge_setup_inputs(
    mut config: Config,
    target_args: &[String],
    ignore_args: &[String],
) -> Result<Config> {
    build_ignore_set(ignore_args)?;
    for value in target_args {
        let (id, path) = parse_setup_target(value)?;
        match config.targets.get(&id) {
            Some(existing) if existing.path == path => {}
            Some(existing) => bail!(
                "target `{id}` is already configured at {}; remove it before using {}",
                existing.path.display(),
                path.display()
            ),
            None => {
                config.targets.insert(
                    id,
                    TargetConfig {
                        path,
                        enabled: true,
                    },
                );
            }
        }
    }
    for pattern in ignore_args {
        if !config.ignore.contains(pattern) {
            config.ignore.push(pattern.clone());
        }
    }
    validate_config(&config)?;
    Ok(config)
}

fn parse_setup_target(value: &str) -> Result<(String, PathBuf)> {
    let (id, path) = value
        .split_once('=')
        .ok_or_else(|| anyhow!("target must use ID=PATH: {value}"))?;
    validate_target_id(id)?;
    if path.is_empty() {
        bail!("target path must not be empty: {value}");
    }
    Ok((id.to_string(), absolute_path(Path::new(path))?))
}

pub fn scan(config: &Config) -> Result<ScanResult> {
    let ignores = build_ignore_set(&config.ignore)?;
    let mut groups = BTreeMap::<String, SkillGroup>::new();
    let mut logical_names = BTreeMap::<String, String>::new();
    if config.root.exists() {
        for entry in sorted_children(&config.root)? {
            let kind = entry.file_type()?;
            let name = utf8_name(&entry.path())?;
            if ignored_top_level(&name)
                || ignores.is_match(&name)
                || (!kind.is_dir() && !kind.is_symlink())
            {
                continue;
            }
            if kind.is_symlink() {
                bail!(
                    "canonical skill must be a real directory, not a symlink: {}",
                    entry.path().display()
                );
            }
            check_case_collision(&mut logical_names, &name, "configured skill locations")?;
            let fp = fingerprint_with_matcher(&entry.path(), &ignores)?;
            groups
                .entry(name.clone())
                .or_insert_with(|| empty_group(&name))
                .canonical = Some(CanonicalSkill {
                path: entry.path(),
                fingerprint: fp,
            });
        }
    }
    let mut target_counts = BTreeMap::new();
    let mut missing_targets = Vec::new();
    for (target_id, target) in config.targets.iter().filter(|(_, t)| t.enabled) {
        if !target.path.is_dir() {
            missing_targets.push(target_id.clone());
            target_counts.insert(target_id.clone(), 0);
            continue;
        }
        let mut count = 0;
        for entry in sorted_children(&target.path)? {
            let name = utf8_name(&entry.path())?;
            if ignored_top_level(&name) || ignores.is_match(&name) {
                continue;
            }
            check_case_collision(&mut logical_names, &name, "configured skill locations")?;
            let installation = inspect_installation(config, &ignores, target_id, entry.path())?;
            groups
                .entry(name.clone())
                .or_insert_with(|| empty_group(&name))
                .installations
                .push(installation);
            count += 1;
        }
        target_counts.insert(target_id.clone(), count);
    }
    Ok(ScanResult {
        groups,
        target_counts,
        missing_targets,
    })
}

fn sorted_children(path: &Path) -> Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(path)
        .with_context(|| format!("read {}", path.display()))?
        .collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

fn utf8_name(path: &Path) -> Result<String> {
    path.file_name()
        .and_then(OsStr::to_str)
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("skill name is not valid UTF-8: {}", path.display()))
}

fn check_case_collision(
    seen: &mut BTreeMap<String, String>,
    name: &str,
    location: &str,
) -> Result<()> {
    let folded = name.to_lowercase();
    if let Some(existing) = seen.insert(folded, name.to_string()) {
        if existing != name {
            bail!("case-insensitive name collision in {location}: {existing} and {name}");
        }
    }
    Ok(())
}

fn empty_group(name: &str) -> SkillGroup {
    SkillGroup {
        name: name.to_string(),
        canonical: None,
        installations: Vec::new(),
    }
}

fn ignored_top_level(name: &str) -> bool {
    // Skills are directories or symlinks with real names; agents sometimes drop
    // their own bookkeeping files (manifests, caches, markers) alongside them.
    name.starts_with('.')
}

fn inspect_installation(
    config: &Config,
    ignores: &GlobSet,
    target: &str,
    path: PathBuf,
) -> Result<Installation> {
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() {
        let raw_target = fs::read_link(&path)?;
        let resolved = resolve_link_path(&path, &raw_target);
        let exists = resolved.is_dir();
        let managed = is_canonical_skill_path(&config.root, &resolved);
        let kind = if !exists {
            InstallationKind::BrokenSymlink
        } else if managed {
            InstallationKind::ManagedSymlink
        } else {
            InstallationKind::ForeignSymlink
        };
        let fingerprint = if exists {
            Some(fingerprint_with_matcher(&resolved, ignores)?)
        } else {
            None
        };
        Ok(Installation {
            target: target.to_string(),
            path,
            kind,
            fingerprint,
            link_target: Some(resolved),
        })
    } else if metadata.is_dir() {
        let fingerprint = Some(fingerprint_with_matcher(&path, ignores)?);
        Ok(Installation {
            target: target.to_string(),
            path,
            kind: InstallationKind::Physical,
            fingerprint,
            link_target: None,
        })
    } else {
        bail!(
            "skill installation is neither a directory nor symlink: {}",
            path.display()
        )
    }
}

fn is_canonical_skill_path(root: &Path, path: &Path) -> bool {
    path.parent() == Some(root) && path.file_name().is_some()
}

pub fn fingerprint(root: &Path) -> Result<String> {
    fingerprint_with_ignores(root, &[])
}

pub fn fingerprint_with_ignores(root: &Path, patterns: &[String]) -> Result<String> {
    let ignores = build_ignore_set(patterns)?;
    fingerprint_with_matcher(root, &ignores)
}

fn build_ignore_set(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(
            Glob::new(pattern).with_context(|| format!("invalid ignore pattern `{pattern}`"))?,
        );
    }
    Ok(builder.build()?)
}

fn fingerprint_with_matcher(root: &Path, ignores: &GlobSet) -> Result<String> {
    if !root.is_dir() {
        bail!("not a readable skill directory: {}", root.display());
    }
    let mut records = Vec::<(String, u8, String)>::new();
    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| include_entry(entry, root, ignores));
    for entry in walker {
        let entry = entry.with_context(|| format!("walk {}", root.display()))?;
        if entry.path() == root {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        let file_type = entry.file_type();
        if file_type.is_dir() {
            continue;
        }
        if file_type.is_file() {
            let mut hasher = Hasher::new();
            let mut reader = BufReader::new(File::open(entry.path())?);
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let read = reader.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
            }
            records.push((relative, b'F', hasher.finalize().to_hex().to_string()));
        } else if file_type.is_symlink() {
            records.push((
                relative,
                b'L',
                fs::read_link(entry.path())?.to_string_lossy().to_string(),
            ));
        } else {
            bail!(
                "unsupported special file in skill: {}",
                entry.path().display()
            );
        }
    }
    records.sort();
    let mut directory = Hasher::new();
    for (path, kind, hash) in records {
        directory.update(&[kind]);
        directory.update(path.as_bytes());
        directory.update(&[0]);
        directory.update(hash.as_bytes());
        directory.update(&[0]);
    }
    Ok(directory.finalize().to_hex().to_string())
}

fn include_entry(entry: &DirEntry, root: &Path, ignores: &GlobSet) -> bool {
    let name = entry.file_name();
    if name == OsStr::new(".git") || name == OsStr::new(".DS_Store") {
        return false;
    }
    entry
        .path()
        .strip_prefix(root)
        .map(|relative| !ignores.is_match(relative))
        .unwrap_or(true)
}

impl SkillGroup {
    pub fn status(&self) -> SkillStatus {
        if self
            .installations
            .iter()
            .any(|i| i.kind == InstallationKind::BrokenSymlink)
        {
            return SkillStatus::Broken;
        }
        if self
            .installations
            .iter()
            .any(|i| i.kind == InstallationKind::ForeignSymlink)
        {
            return SkillStatus::Divergent;
        }
        let fingerprints: BTreeSet<&str> = self
            .installations
            .iter()
            .filter(|i| i.kind != InstallationKind::ManagedSymlink)
            .filter_map(|i| i.fingerprint.as_deref())
            .chain(self.canonical.as_ref().map(|c| c.fingerprint.as_str()))
            .collect();
        if fingerprints.len() > 1 {
            return SkillStatus::Divergent;
        }
        let physical = self
            .installations
            .iter()
            .filter(|i| i.kind == InstallationKind::Physical)
            .count();
        if self.canonical.is_none()
            && physical == 0
            && self
                .installations
                .iter()
                .all(|i| i.kind == InstallationKind::ManagedSymlink)
        {
            SkillStatus::Managed
        } else if self.canonical.is_some() && physical > 0 {
            SkillStatus::IdenticalDuplicate
        } else if self.canonical.is_some() {
            SkillStatus::Managed
        } else if physical > 1 {
            SkillStatus::IdenticalDuplicate
        } else {
            SkillStatus::Unique
        }
    }
}

fn scan_command(config: &Config, project: Option<&Path>) -> Result<u8> {
    let effective = config_with_project(config, project)?;
    let result = scan(&effective)?;
    if JSON_OUTPUT.load(Ordering::Relaxed) {
        let skills: Vec<_> = result
            .groups
            .values()
            .map(|group| {
                serde_json::json!({
                    "name": group.name,
                    "status": group.status(),
                    "canonical": group.canonical,
                    "installations": group.installations,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "targets": result.target_counts,
                "missing_targets": result.missing_targets,
                "skills": skills,
            }))?
        );
        return Ok(result_exit(&result));
    }
    println!("{}", theme::banner("Scanning known skill directories..."));
    println!();
    let mut target_counts: Vec<_> = result.target_counts.iter().collect();
    target_counts.sort_by_key(|(id, _)| target_rank(id));
    for (target, count) in target_counts {
        println!(
            "  {} {} {}",
            theme::agent_padded(&target_label(target), 10),
            theme::path_text(&format!(
                "{:<38}",
                display_path(&effective.targets[target].path)
            )),
            theme::count(format!("{count:>3}"))
        );
    }
    for target in &result.missing_targets {
        println!("  {:<12} {} missing", theme::agent(target), theme::warn());
    }
    let installations: usize = result.target_counts.values().sum();
    let count = |status| {
        result
            .groups
            .values()
            .filter(|group| group.status() == status)
            .count()
    };
    println!();
    println!("Found {} installations", theme::count(installations));
    println!("Found {} unique skills", theme::count(result.groups.len()));
    println!(
        "{} {} identical duplicates",
        theme::ok(),
        theme::good_count(count(SkillStatus::IdenticalDuplicate))
    );
    let divergent = count(SkillStatus::Divergent);
    println!(
        "{} {} divergent",
        if divergent == 0 {
            theme::ok()
        } else {
            theme::warn()
        },
        theme::warn_count(divergent)
    );
    println!(
        "{} {} unique",
        theme::dot(),
        theme::count(count(SkillStatus::Unique))
    );
    if VERBOSITY.load(Ordering::Relaxed) > 0 {
        print_groups(&result);
    }
    Ok(result_exit(&result))
}

fn config_with_project(config: &Config, project: Option<&Path>) -> Result<Config> {
    let Some(project) = project else {
        return Ok(config.clone());
    };
    let project = absolute_path(project)?;
    let mut effective = config.clone();
    for (id, relative) in [
        ("project-claude", ".claude/skills"),
        ("project-agents", ".agents/skills"),
    ] {
        let path = project.join(relative);
        if path.is_dir() {
            if effective.targets.contains_key(id) {
                bail!("configured target id `{id}` conflicts with project discovery");
            }
            effective.targets.insert(
                id.to_string(),
                TargetConfig {
                    path,
                    enabled: true,
                },
            );
        }
    }
    validate_config(&effective)?;
    Ok(effective)
}

fn print_groups(result: &ScanResult) {
    for status in [
        SkillStatus::IdenticalDuplicate,
        SkillStatus::Divergent,
        SkillStatus::Unique,
        SkillStatus::Managed,
        SkillStatus::Broken,
    ] {
        let matching: Vec<_> = result
            .groups
            .values()
            .filter(|g| g.status() == status)
            .collect();
        if matching.is_empty() {
            continue;
        }
        let heading = match status {
            SkillStatus::IdenticalDuplicate => "IDENTICAL DUPLICATES",
            SkillStatus::Divergent => "CONFLICTS",
            SkillStatus::Unique => "UNIQUE",
            SkillStatus::Managed => "MANAGED",
            SkillStatus::Broken => "BROKEN",
        };
        println!("\n{}", theme::heading(heading));
        for group in matching {
            let targets = group
                .installations
                .iter()
                .map(|i| i.target.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            println!(
                "{} {}",
                theme::skill(&format!("{:<20}", group.name)),
                theme::agent(&targets)
            );
            let detailed =
                status == SkillStatus::Divergent || VERBOSITY.load(Ordering::Relaxed) > 1;
            if detailed {
                if let Some(c) = &group.canonical {
                    println!(
                        "  {} {}",
                        theme::dim("canonical   "),
                        theme::fingerprint(short_hash(&c.fingerprint))
                    );
                }
                let fallback = if status == SkillStatus::Divergent {
                    "broken"
                } else {
                    "unreadable"
                };
                for installation in &group.installations {
                    println!(
                        "  {} {}",
                        theme::agent_padded(&installation.target, 12),
                        theme::fingerprint(
                            installation
                                .fingerprint
                                .as_deref()
                                .map(short_hash)
                                .unwrap_or(fallback)
                        )
                    );
                }
            }
        }
    }
}

fn short_hash(hash: &str) -> &str {
    &hash[..hash.len().min(7)]
}

fn target_label(id: &str) -> String {
    match id {
        "claude" => "Claude".into(),
        "codex" => "Codex".into(),
        "gemini" => "Gemini".into(),
        "agents" => "Agents".into(),
        "opencode" => "Opencode".into(),
        "hermes" => "Hermes".into(),
        "cursor" => "Cursor".into(),
        _ => {
            let mut chars = id.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        }
    }
}

fn target_rank(id: &str) -> u8 {
    match id {
        "claude" => 0,
        "codex" => 1,
        "gemini" => 2,
        "agents" => 3,
        "opencode" => 4,
        "hermes" => 5,
        "cursor" => 6,
        _ => 7,
    }
}

fn display_path(path: &Path) -> String {
    theme::display_path(path)
}

fn result_exit(result: &ScanResult) -> u8 {
    if result
        .groups
        .values()
        .any(|g| g.status() == SkillStatus::Divergent)
    {
        EXIT_CONFLICTS
    } else if !result.missing_targets.is_empty()
        || result.groups.values().any(|group| {
            group.canonical.is_some()
                && result.target_counts.keys().any(|target| {
                    !group.installations.iter().any(|installation| {
                        &installation.target == target
                            && installation.kind == InstallationKind::ManagedSymlink
                    })
                })
        })
        || result
            .groups
            .values()
            .any(|g| g.status() != SkillStatus::Managed)
    {
        EXIT_ISSUES
    } else {
        EXIT_OK
    }
}

fn status_command(config: &Config, include_git: bool) -> Result<u8> {
    let result = scan(config)?;
    let canonical = result
        .groups
        .values()
        .filter(|g| g.canonical.is_some())
        .count();
    let installations: usize = result.target_counts.values().sum();
    let managed = result
        .groups
        .values()
        .flat_map(|g| &g.installations)
        .filter(|i| i.kind == InstallationKind::ManagedSymlink)
        .count();
    let coverage: BTreeMap<String, usize> = config
        .targets
        .iter()
        .filter(|(_, target)| target.enabled)
        .map(|(id, _)| {
            let count = result
                .groups
                .values()
                .filter(|group| {
                    group.canonical.is_some()
                        && group.installations.iter().any(|installation| {
                            installation.target == *id
                                && installation.kind == InstallationKind::ManagedSymlink
                        })
                })
                .count();
            (id.clone(), count)
        })
        .collect();
    let recovery = recovery_artifacts(config)?;
    let code = result_exit(&result).max(if recovery.is_empty() {
        EXIT_OK
    } else {
        EXIT_ISSUES
    });
    let git = include_git.then(|| git_health(&config.root)).transpose()?;
    if JSON_OUTPUT.load(Ordering::Relaxed) {
        let skills: Vec<_> = result
            .groups
            .values()
            .map(|group| {
                serde_json::json!({
                    "name": group.name,
                    "status": group.status(),
                    "canonical": group.canonical,
                    "installations": group.installations,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "root": config.root,
                "canonical_skills": canonical,
                "targets": config.targets.values().filter(|target| target.enabled).count(),
                "installations": installations,
                "healthy_symlinks": managed,
                "coverage": coverage,
                "healthy": code == EXIT_OK,
                "recovery_artifacts": recovery,
                "skills": skills,
                "git": git,
            }))?
        );
        return Ok(code);
    }
    println!("{}", theme::banner("one true copy of every agent skill"));
    println!();
    println!("{}", theme::heading("CANONICAL"));
    println!("  {}", theme::path_text(&display_path(&config.root)));
    println!("  {} skills", theme::count(canonical));
    println!("  {} agents", theme::count(coverage.len()));
    println!();
    println!("{}", theme::heading("COVERAGE"));
    let mut ordered_coverage: Vec<_> = coverage.iter().collect();
    ordered_coverage.sort_by_key(|(id, _)| target_rank(id));
    for (id, count) in ordered_coverage {
        let complete = *count == canonical;
        println!(
            "{} {} {} {}",
            if complete { theme::ok() } else { theme::warn() },
            theme::agent_padded(&target_label(id), 10),
            if complete {
                theme::good_count(format!("{count}/{canonical}"))
            } else {
                theme::warn_count(format!("{count}/{canonical}"))
            },
            theme::gauge(*count, canonical, 12)
        );
    }
    if VERBOSITY.load(Ordering::Relaxed) > 0 {
        println!(
            "  {} installations\n  {} healthy symlinks",
            theme::count(installations),
            theme::good_count(managed)
        );
    }
    if code == EXIT_OK {
        println!("\n{} No skill issues.", theme::ok());
    } else {
        println!("\n{}", theme::heading("ISSUES"));
        for target in &result.missing_targets {
            println!(
                "{} {} target directory is missing",
                theme::warn(),
                theme::agent(target)
            );
        }
        for path in &recovery {
            println!(
                "{} interrupted-migration recovery artifact: {}",
                theme::warn(),
                theme::path(path)
            );
        }
        for group in result
            .groups
            .values()
            .filter(|g| g.status() != SkillStatus::Managed)
        {
            println!(
                "{} {} ({})",
                issue_mark(group.status()),
                theme::skill(&group.name),
                status_label(group.status())
            );
            for installation in &group.installations {
                println!(
                    "  {}: {} ({})",
                    theme::agent(&installation.target),
                    theme::path(&installation.path),
                    theme::dim(kind_label(&installation.kind))
                );
            }
        }
        for (id, count) in coverage.iter().filter(|(_, count)| **count < canonical) {
            println!(
                "{} {} is missing {} canonical skill links",
                theme::warn(),
                theme::agent(&target_label(id)),
                theme::warn_count(canonical - count)
            );
        }
        println!("Run: {}", theme::hint("si sync"));
    }
    if let Some(git) = git {
        print_git_health(&git, &config.root);
    }
    Ok(code)
}

#[derive(Debug, Serialize)]
struct GitHealth {
    repository: bool,
    dirty: bool,
    upstream: Option<String>,
    ahead: Option<u64>,
    behind: Option<u64>,
}

fn git_health(root: &Path) -> Result<GitHealth> {
    let repository = git_output(root, &["rev-parse", "--show-toplevel"])?;
    let repository_root = repository.status.success().then(|| {
        PathBuf::from(
            String::from_utf8_lossy(&repository.stdout)
                .trim()
                .to_string(),
        )
    });
    let root_is_repository = repository_root
        .as_deref()
        .and_then(|path| fs::canonicalize(path).ok())
        .zip(fs::canonicalize(root).ok())
        .is_some_and(|(repository, configured)| repository == configured);
    if !root_is_repository {
        return Ok(GitHealth {
            repository: false,
            dirty: false,
            upstream: None,
            ahead: None,
            behind: None,
        });
    }
    let status = git_output(root, &["status", "--porcelain"])?;
    if !status.status.success() {
        bail!("could not inspect Git status in {}", root.display());
    }
    let upstream_output = git_output(
        root,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    )?;
    let upstream = upstream_output.status.success().then(|| {
        String::from_utf8_lossy(&upstream_output.stdout)
            .trim()
            .to_string()
    });
    let (ahead, behind) = if upstream.is_some() {
        let output = git_output(
            root,
            &["rev-list", "--left-right", "--count", "HEAD...@{u}"],
        )?;
        if output.status.success() {
            let counts = String::from_utf8_lossy(&output.stdout);
            let mut counts = counts.split_whitespace();
            (
                counts.next().map(str::parse).transpose()?,
                counts.next().map(str::parse).transpose()?,
            )
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };
    Ok(GitHealth {
        repository: true,
        dirty: !status.stdout.is_empty(),
        upstream,
        ahead,
        behind,
    })
}

fn git_output(root: &Path, args: &[&str]) -> Result<std::process::Output> {
    std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .with_context(|| format!("run `git {}` in {}", args.join(" "), root.display()))
}

fn print_git_health(git: &GitHealth, root: &Path) {
    println!("\n{}", theme::heading("GIT"));
    if !git.repository {
        println!(
            "{} Canonical root is not a Git repository: {}",
            theme::warn(),
            theme::path(root)
        );
        return;
    }
    println!("{} repository", theme::ok());
    println!(
        "{} working tree",
        if git.dirty {
            format!("{} {}", theme::warn(), theme::warn_count("dirty"))
        } else {
            format!("{} {}", theme::ok(), theme::good_count("clean"))
        }
    );
    match (&git.upstream, git.ahead, git.behind) {
        (Some(upstream), Some(ahead), Some(behind)) => {
            let current = ahead == 0 && behind == 0;
            println!(
                "{} upstream {}; {} ahead, {} behind",
                if current { theme::ok() } else { theme::warn() },
                theme::agent(upstream),
                if ahead == 0 {
                    theme::good_count(ahead)
                } else {
                    theme::warn_count(ahead)
                },
                if behind == 0 {
                    theme::good_count(behind)
                } else {
                    theme::warn_count(behind)
                }
            );
        }
        _ => println!("{} no upstream configured", theme::warn()),
    }
}

fn issue_mark(status: SkillStatus) -> console::StyledObject<&'static str> {
    if status == SkillStatus::Divergent {
        theme::bad()
    } else {
        theme::warn()
    }
}

fn status_label(status: SkillStatus) -> &'static str {
    match status {
        SkillStatus::Unique => "unadopted",
        SkillStatus::IdenticalDuplicate => "duplicate",
        SkillStatus::Divergent => "conflict",
        SkillStatus::Managed => "managed",
        SkillStatus::Broken => "broken",
    }
}

fn kind_label(kind: &InstallationKind) -> &'static str {
    match kind {
        InstallationKind::Physical => "physical copy",
        InstallationKind::ManagedSymlink => "managed link",
        InstallationKind::ForeignSymlink => "foreign link",
        InstallationKind::BrokenSymlink => "broken link",
    }
}

fn default_command() -> Result<u8> {
    let config = match Config::load() {
        Ok(config) => config,
        Err(error) if is_missing_config(&error) => {
            eprintln!(
                "{} skill-issue is not configured. Run {}.",
                theme::warn_err(),
                theme::hint_err("si setup ~/code/skills")
            );
            return Ok(EXIT_CONFIG);
        }
        Err(error) => {
            eprintln!("{} Invalid configuration: {error:#}", theme::bad_err());
            return Ok(EXIT_CONFIG);
        }
    };
    validate_config(&config)?;
    status_command(&config, true)
}

fn adopt_command(
    config: &Config,
    skill: Option<&str>,
    dry_run: bool,
    interactive: bool,
) -> Result<u8> {
    let result = scan(config)?;
    adopt_from_scan(config, &result, skill, dry_run, interactive)
}

fn adopt_from_scan(
    config: &Config,
    result: &ScanResult,
    skill: Option<&str>,
    dry_run: bool,
    interactive: bool,
) -> Result<u8> {
    if let Some(name) = skill {
        if !result.groups.contains_key(name) {
            bail!("skill `{name}` was not found");
        }
    }
    let tty = interactive && io::stdin().is_terminal() && io::stdout().is_terminal();
    let selected: Vec<&SkillGroup> = result
        .groups
        .values()
        .filter(|g| skill.is_none_or(|name| g.name == name))
        .collect();
    if !tty
        && selected
            .iter()
            .any(|g| g.status() == SkillStatus::Divergent)
    {
        bail!("divergent skills require an interactive terminal; no files were changed");
    }
    if !tty && !ASSUME_YES.load(Ordering::Relaxed) && !dry_run {
        bail!("adoption requires an interactive confirmation; use --dry-run to inspect the plan");
    }
    println!(
        "{}\n  {}\n",
        theme::heading("CANONICAL SKILL DIRECTORY"),
        theme::path_text(&display_path(&config.root))
    );
    let mut stats = MigrationStats {
        skills: selected.len(),
        installations: result.target_counts.values().sum(),
        ..MigrationStats::default()
    };
    let mut planned_any = false;
    for group in selected {
        if matches!(group.status(), SkillStatus::Managed | SkillStatus::Broken) {
            continue;
        }
        print_adoption_group(group);
        if tty
            && group.status() != SkillStatus::Divergent
            && !ASSUME_YES.load(Ordering::Relaxed)
            && !Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt("Adopt?")
                .default(true)
                .interact()?
        {
            stats.skipped += 1;
            println!("{} skipped\n", theme::warn());
            continue;
        }
        let mut plans = plans_for_group(config, group, tty)?;
        if plans.is_empty() {
            stats.skipped += 1;
            println!("{} skipped\n", theme::warn());
            continue;
        }
        if plans.len() == 1 {
            add_missing_target_links(config, group, &mut plans[0])?;
        }
        planned_any = true;
        if dry_run || VERBOSITY.load(Ordering::Relaxed) > 0 {
            render_adoption_plans(&plans);
        } else {
            render_adoption_summary(&plans);
        }
        if dry_run {
            stats.links += plans
                .iter()
                .map(|plan| plan.replacements.len())
                .sum::<usize>();
            stats.preserved += plans.iter().map(|plan| plan.preserved.len()).sum::<usize>();
            println!();
            continue;
        }
        if tty
            && group.status() == SkillStatus::Divergent
            && !ASSUME_YES.load(Ordering::Relaxed)
            && !Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt("Use this resolution?")
                .default(false)
                .interact()?
        {
            stats.skipped += 1;
            println!("{} skipped\n", theme::warn());
            continue;
        }
        for plan in &plans {
            execute_adoption(plan)?;
            stats.links += plan.replacements.len();
            stats.preserved += plan.preserved.len();
        }
        stats.adopted += 1;
        println!("{} {} adopted\n", theme::ok(), theme::skill(&group.name));
    }
    if !planned_any {
        if stats.skipped > 0 {
            println!("{}", theme::heading("DONE"));
            println!("{} skills", theme::count(stats.skills));
            println!("{} installations", theme::count(stats.installations));
            println!("{} {} adopted", theme::ok(), theme::good_count(0));
            println!(
                "{} {} skipped",
                theme::warn(),
                theme::warn_count(stats.skipped)
            );
            println!("{} {} symlinks created", theme::ok(), theme::good_count(0));
            println!("{} no data lost", theme::ok());
            println!("Run:\n  {}", theme::hint("si status"));
            return Ok(result_exit(result));
        }
        println!("{}", theme::dim("No adoptable skills found."));
        return Ok(result_exit(result));
    }
    if dry_run {
        println!("{}", theme::dim("Dry run; no files changed."));
        return Ok(result_exit(result));
    }
    let after = scan(config)?;
    println!("{}", theme::heading("DONE"));
    println!("{} skills", theme::count(stats.skills));
    println!("{} installations", theme::count(stats.installations));
    println!(
        "{} {} adopted",
        theme::ok(),
        theme::good_count(stats.adopted)
    );
    if stats.skipped > 0 {
        println!(
            "{} {} skipped",
            theme::warn(),
            theme::warn_count(stats.skipped)
        );
    }
    println!(
        "{} {} symlinks created",
        theme::ok(),
        theme::good_count(stats.links)
    );
    if stats.preserved > 0 {
        println!(
            "{} {} divergent versions preserved",
            theme::ok(),
            theme::good_count(stats.preserved)
        );
    }
    println!("{} no data lost", theme::ok());
    if result_exit(&after) == EXIT_OK {
        println!("{} No skill issues.", theme::ok());
    } else {
        println!("Run:\n  {}", theme::hint("si status"));
    }
    Ok(result_exit(&after))
}

fn add_missing_target_links(
    config: &Config,
    group: &SkillGroup,
    plan: &mut AdoptionPlan,
) -> Result<()> {
    let installed: BTreeSet<&str> = group
        .installations
        .iter()
        .map(|installation| installation.target.as_str())
        .collect();
    for (id, target) in config.targets.iter().filter(|(_, target)| target.enabled) {
        if installed.contains(id.as_str()) {
            continue;
        }
        let destination = target.path.join(&group.name);
        if fs::symlink_metadata(&destination).is_ok() {
            bail!("{} already exists; nothing changed", destination.display());
        }
        plan.replacements.push(Replacement {
            original: destination,
            backup: None,
        });
    }
    Ok(())
}

fn print_adoption_group(group: &SkillGroup) {
    println!("{}", theme::skill(&group.name));
    if let Some(canonical) = &group.canonical {
        println!(
            "  {} {}",
            theme::agent_padded("Canonical", 10),
            theme::fingerprint(short_hash(&canonical.fingerprint))
        );
    }
    for installation in &group.installations {
        println!(
            "  {} {}  {}",
            theme::agent_padded(&target_label(&installation.target), 10),
            theme::fingerprint(
                installation
                    .fingerprint
                    .as_deref()
                    .map(short_hash)
                    .unwrap_or("broken")
            ),
            theme::path_text(&display_path(&installation.path))
        );
    }
    match group.status() {
        SkillStatus::IdenticalDuplicate => {
            println!("{} all copies identical", theme::ok())
        }
        SkillStatus::Unique => println!("{} one physical copy", theme::dot()),
        SkillStatus::Divergent => println!("{} copies differ", theme::warn()),
        _ => {}
    }
}

fn plans_for_group(config: &Config, group: &SkillGroup, tty: bool) -> Result<Vec<AdoptionPlan>> {
    if matches!(group.status(), SkillStatus::Managed | SkillStatus::Broken) {
        return Ok(Vec::new());
    }
    let physical_by_fp = physical_versions(group);
    if physical_by_fp.is_empty() {
        return Ok(Vec::new());
    }
    if let Some(canonical) = &group.canonical {
        let matching = physical_by_fp
            .get(&canonical.fingerprint)
            .cloned()
            .unwrap_or_default();
        if group.status() != SkillStatus::Divergent {
            return Ok(plan_existing(
                &group.name,
                canonical,
                matching,
                &config.ignore,
                config.relative_links,
            )
            .into_iter()
            .collect());
        }
        if !tty {
            bail!("{} has divergent copies", group.name);
        }
        let options = [
            "Use the existing canonical version where copies match",
            "Keep all physical versions under separate names",
            "View diff",
            "Skip",
        ];
        match Select::with_theme(&ColorfulTheme::default())
            .with_prompt(format!("{} has divergent copies", group.name))
            .items(options)
            .default(3)
            .interact()?
        {
            0 => plan_selected_version(
                config,
                group,
                canonical.fingerprint.clone(),
                matching,
                Some(canonical),
            )
            .map(|plan| vec![plan]),
            1 => plans_keep_both(config, group, &physical_by_fp, Some(&canonical.fingerprint)),
            2 => {
                diff_group(group, true, &config.ignore)?;
                plans_for_group(config, group, tty)
            }
            _ => Ok(Vec::new()),
        }
    } else if physical_by_fp.len() == 1 {
        let (fingerprint, paths) = physical_by_fp.into_iter().next().unwrap();
        Ok(vec![plan_new(
            config,
            &group.name,
            &group.name,
            fingerprint,
            paths,
        )?])
    } else {
        if !tty {
            bail!("{} has divergent copies", group.name);
        }
        let versions: Vec<_> = physical_by_fp.iter().collect();
        let mut labels: Vec<String> = versions
            .iter()
            .map(|(fp, paths)| {
                let targets = paths
                    .iter()
                    .filter_map(|path| {
                        group
                            .installations
                            .iter()
                            .find(|i| i.path == *path)
                            .map(|i| target_label(&i.target))
                    })
                    .collect::<Vec<_>>()
                    .join(" / ");
                format!("Use {targets} ({})", short_hash(fp))
            })
            .collect();
        labels.push("Keep all versions under separate names".to_string());
        labels.push("View diff".to_string());
        labels.push("Skip".to_string());
        let choice = Select::with_theme(&ColorfulTheme::default())
            .with_prompt(format!("{} has divergent copies", group.name))
            .items(&labels)
            .default(labels.len() - 1)
            .interact()?;
        if choice < versions.len() {
            let (fingerprint, paths) = versions[choice];
            Ok(vec![plan_selected_version(
                config,
                group,
                (*fingerprint).clone(),
                (*paths).clone(),
                None,
            )?])
        } else if choice == versions.len() {
            plans_keep_both(config, group, &physical_by_fp, None)
        } else if choice == versions.len() + 1 {
            diff_group(group, true, &config.ignore)?;
            plans_for_group(config, group, tty)
        } else {
            Ok(Vec::new())
        }
    }
}

fn physical_versions(group: &SkillGroup) -> BTreeMap<String, Vec<PathBuf>> {
    let mut versions = BTreeMap::<String, Vec<PathBuf>>::new();
    for installation in group
        .installations
        .iter()
        .filter(|i| i.kind == InstallationKind::Physical)
    {
        if let Some(fp) = &installation.fingerprint {
            versions
                .entry(fp.clone())
                .or_default()
                .push(installation.path.clone());
        }
    }
    versions
}

fn plan_selected_version(
    config: &Config,
    group: &SkillGroup,
    fingerprint: String,
    matching: Vec<PathBuf>,
    canonical: Option<&CanonicalSkill>,
) -> Result<AdoptionPlan> {
    let archive_root = migration_archive_root()?.join(&group.name);
    plan_selected_version_at(
        config,
        group,
        fingerprint,
        matching,
        canonical,
        archive_root,
    )
}

fn plan_selected_version_at(
    config: &Config,
    group: &SkillGroup,
    fingerprint: String,
    matching: Vec<PathBuf>,
    canonical: Option<&CanonicalSkill>,
    archive_root: PathBuf,
) -> Result<AdoptionPlan> {
    let mut plan = if let Some(canonical) = canonical {
        plan_existing(
            &group.name,
            canonical,
            matching,
            &config.ignore,
            config.relative_links,
        )
        .unwrap_or_else(|| AdoptionPlan {
            display_name: group.name.clone(),
            canonical: canonical.path.clone(),
            fingerprint: canonical.fingerprint.clone(),
            transfer: CanonicalTransfer::Existing,
            replacements: Vec::new(),
            preserved: Vec::new(),
            ignore: config.ignore.clone(),
            relative_links: config.relative_links,
        })
    } else {
        plan_new(
            config,
            &group.name,
            &group.name,
            fingerprint.clone(),
            matching,
        )?
    };
    for installation in group.installations.iter().filter(|installation| {
        installation.kind == InstallationKind::Physical
            && installation.fingerprint.as_deref() != Some(fingerprint.as_str())
    }) {
        let rejected_fingerprint = installation.fingerprint.clone().ok_or_else(|| {
            anyhow!(
                "cannot preserve unreadable copy at {}",
                installation.path.display()
            )
        })?;
        let destination = archive_root.join(&installation.target);
        plan.preserved.push(PreservedCopy {
            source: installation.path.clone(),
            destination,
            fingerprint: rejected_fingerprint,
            target: installation.target.clone(),
        });
        plan.replacements.push(Replacement {
            original: installation.path.clone(),
            backup: Some(unique_sibling(&installation.path, "backup")),
        });
    }
    Ok(plan)
}

fn migration_archive_root() -> Result<PathBuf> {
    let base = std::env::var_os("SKILL_ISSUE_CACHE")
        .or_else(|| std::env::var_os("SKILLISSUE_CACHE"))
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".cache")))
        .ok_or_else(|| anyhow!("could not determine cache directory for migration archive"))?;
    Ok(base.join("skill-issue/migrations").join(
        chrono::Utc::now()
            .format("%Y-%m-%dT%H%M%S%.3fZ")
            .to_string(),
    ))
}

fn plan_existing(
    name: &str,
    canonical: &CanonicalSkill,
    installations: Vec<PathBuf>,
    ignore: &[String],
    relative_links: bool,
) -> Option<AdoptionPlan> {
    if installations.is_empty() {
        return None;
    }
    Some(AdoptionPlan {
        display_name: name.to_string(),
        canonical: canonical.path.clone(),
        fingerprint: canonical.fingerprint.clone(),
        transfer: CanonicalTransfer::Existing,
        replacements: installations
            .into_iter()
            .map(|original| Replacement {
                backup: Some(unique_sibling(&original, "backup")),
                original,
            })
            .collect(),
        preserved: Vec::new(),
        ignore: ignore.to_vec(),
        relative_links,
    })
}

fn plan_new(
    config: &Config,
    display_name: &str,
    canonical_name: &str,
    fingerprint: String,
    installations: Vec<PathBuf>,
) -> Result<AdoptionPlan> {
    validate_skill_name(canonical_name)?;
    if canonical_skill_names(&config.root, &config.ignore)?
        .iter()
        .any(|name| name.to_lowercase() == canonical_name.to_lowercase())
    {
        bail!("canonical name collides with an existing skill: `{canonical_name}`");
    }
    let canonical = config.root.join(canonical_name);
    if canonical.exists() || fs::symlink_metadata(&canonical).is_ok() {
        bail!(
            "canonical destination already exists: {}",
            canonical.display()
        );
    }
    let source = installations
        .first()
        .cloned()
        .ok_or_else(|| anyhow!("no physical source for {display_name}"))?;
    let transfer = if same_filesystem(&source, &config.root) {
        CanonicalTransfer::Move {
            source: source.clone(),
        }
    } else {
        CanonicalTransfer::Copy {
            source: source.clone(),
            temporary: unique_sibling(&canonical, "tmp"),
        }
    };
    let moves_source = matches!(transfer, CanonicalTransfer::Move { .. });
    let replacements = installations
        .into_iter()
        .map(|original| Replacement {
            backup: (!(moves_source && original == source))
                .then(|| unique_sibling(&original, "backup")),
            original,
        })
        .collect();
    Ok(AdoptionPlan {
        display_name: display_name.to_string(),
        canonical,
        fingerprint,
        transfer,
        replacements,
        preserved: Vec::new(),
        ignore: config.ignore.clone(),
        relative_links: config.relative_links,
    })
}

#[cfg(unix)]
fn same_filesystem(source: &Path, destination_directory: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::metadata(source), fs::metadata(destination_directory)) {
        (Ok(source), Ok(destination)) => source.dev() == destination.dev(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn same_filesystem(_source: &Path, _destination_directory: &Path) -> bool {
    false
}

fn plans_keep_both(
    config: &Config,
    group: &SkillGroup,
    versions: &BTreeMap<String, Vec<PathBuf>>,
    existing_fp: Option<&String>,
) -> Result<Vec<AdoptionPlan>> {
    let mut plans = Vec::new();
    let mut used = BTreeSet::new();
    for (index, (fingerprint, paths)) in versions
        .iter()
        .filter(|(fp, _)| existing_fp != Some(*fp))
        .enumerate()
    {
        let target_hint = group
            .installations
            .iter()
            .find(|i| paths.contains(&i.path))
            .map(|i| i.target.as_str())
            .unwrap_or("variant");
        let default = if existing_fp.is_none() && index == 0 {
            group.name.clone()
        } else {
            format!("{}-{target_hint}", group.name)
        };
        let name: String = Input::with_theme(&ColorfulTheme::default())
            .with_prompt(format!("Canonical name for {}", short_hash(fingerprint)))
            .default(default)
            .interact_text()?;
        validate_skill_name(&name)?;
        let folded = name.to_lowercase();
        if !used.insert(folded) {
            bail!("duplicate canonical name `{name}`");
        }
        plans.push(plan_new(
            config,
            &format!("{} ({target_hint})", group.name),
            &name,
            fingerprint.clone(),
            paths.clone(),
        )?);
    }
    if let (Some(canonical), Some(fp)) = (&group.canonical, existing_fp) {
        if let Some(paths) = versions.get(fp) {
            if let Some(plan) = plan_existing(
                &group.name,
                canonical,
                paths.clone(),
                &config.ignore,
                config.relative_links,
            ) {
                plans.push(plan);
            }
        }
    }
    Ok(plans)
}

fn validate_skill_name(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        bail!("invalid skill name `{name}`");
    }
    Ok(())
}

fn render_adoption_summary(plans: &[AdoptionPlan]) {
    let transfer = |label: &str, source: &Path, destination: &Path| {
        println!(
            "{}\n  {}\n    {} {}",
            theme::heading(label),
            theme::path_text(&display_path(source)),
            theme::arrow(),
            theme::path_text(&display_path(destination))
        );
    };
    for plan in plans {
        if !plan.preserved.is_empty() {
            println!("{}", theme::heading("PRESERVING"));
            for preserved in &plan.preserved {
                println!(
                    "  {}\n    {} {}",
                    theme::path_text(&display_path(&preserved.source)),
                    theme::arrow(),
                    theme::path_text(&display_path(&preserved.destination))
                );
            }
        }
        match &plan.transfer {
            CanonicalTransfer::Existing => {}
            CanonicalTransfer::Move { source } => transfer("MOVING", source, &plan.canonical),
            CanonicalTransfer::Copy { source, .. } => transfer("COPYING", source, &plan.canonical),
        }
        println!("{}", theme::heading("LINKING"));
        for replacement in &plan.replacements {
            println!(
                "  {} {} {}",
                theme::path_text(&display_path(&replacement.original)),
                theme::arrow(),
                theme::path_text(&display_path(&plan.canonical))
            );
        }
    }
}

fn render_adoption_plans(plans: &[AdoptionPlan]) {
    println!("{}", theme::heading("PLAN"));
    let step = |verb: &str, from: &Path, to: &Path| {
        println!(
            "{}\n  {}\n    {} {}",
            theme::action(verb),
            theme::path(from),
            theme::arrow(),
            theme::path(to)
        );
    };
    for plan in plans {
        for preserved in &plan.preserved {
            println!(
                "{}\n  {}\n    {} {}",
                theme::action("PRESERVE"),
                theme::path_text(&display_path(&preserved.source)),
                theme::arrow(),
                theme::path_text(&display_path(&preserved.destination))
            );
        }
        match &plan.transfer {
            CanonicalTransfer::Existing => {}
            CanonicalTransfer::Move { source } => step("MOVE", source, &plan.canonical),
            CanonicalTransfer::Copy { source, temporary } => {
                step("COPY", source, temporary);
                println!("{}  {}", theme::action("VERIFY"), theme::path(temporary));
                step("MOVE", temporary, &plan.canonical);
            }
        }
        println!(
            "{}  {}",
            theme::action("VERIFY"),
            theme::path(&plan.canonical)
        );
        for replacement in &plan.replacements {
            if let Some(backup) = &replacement.backup {
                step("STAGE", &replacement.original, backup);
            }
            step(
                "LINK",
                &replacement.original,
                &managed_link_value(&plan.canonical, &replacement.original, plan.relative_links),
            );
            println!(
                "{}  {}",
                theme::action("VERIFY"),
                theme::path(&replacement.original)
            );
            if let Some(backup) = &replacement.backup {
                println!("{}  {}", theme::action("REMOVE"), theme::path(backup));
            }
        }
    }
    println!(
        "{} locations affected.",
        theme::count(plans.iter().map(|p| p.replacements.len()).sum::<usize>())
    );
}

fn execute_adoption(plan: &AdoptionPlan) -> Result<()> {
    for preserved in &plan.preserved {
        let parent = preserved
            .destination
            .parent()
            .ok_or_else(|| anyhow!("invalid migration archive path"))?;
        fs::create_dir_all(parent)?;
        if preserved.destination.exists() {
            bail!(
                "migration archive already exists: {}",
                preserved.destination.display()
            );
        }
        if let Err(error) = copy_skill(&preserved.source, &preserved.destination).and_then(|_| {
            let actual = fingerprint_with_ignores(&preserved.destination, &plan.ignore)?;
            if actual != preserved.fingerprint {
                bail!("migration archive verification failed");
            }
            let manifest = serde_json::to_vec_pretty(&serde_json::json!({
                "skill": plan.display_name,
                "target": preserved.target,
                "source": preserved.source,
                "archive": preserved.destination,
                "fingerprint": preserved.fingerprint,
                "selected_fingerprint": plan.fingerprint,
                "created_at": chrono::Utc::now().to_rfc3339(),
            }))?;
            fs::write(
                parent.join(format!("{}.manifest.json", preserved.target)),
                manifest,
            )?;
            Ok(())
        }) {
            let _ = fs::remove_dir_all(&preserved.destination);
            return Err(error).context("preserve divergent skill version");
        }
    }
    fs::create_dir_all(
        plan.canonical
            .parent()
            .ok_or_else(|| anyhow!("invalid canonical path"))?,
    )?;
    let created_canonical = !matches!(plan.transfer, CanonicalTransfer::Existing);
    let moved_source = match &plan.transfer {
        CanonicalTransfer::Existing => None,
        CanonicalTransfer::Move { source } => {
            fs::rename(source, &plan.canonical).with_context(|| {
                format!("move {} to {}", source.display(), plan.canonical.display())
            })?;
            Some(source.clone())
        }
        CanonicalTransfer::Copy { source, temporary } => {
            if let Err(error) = copy_skill(source, temporary).and_then(|_| {
                let actual = fingerprint_with_ignores(temporary, &plan.ignore)?;
                if actual != plan.fingerprint {
                    bail!("verification failed while copying {}", source.display());
                }
                fs::rename(temporary, &plan.canonical)?;
                Ok(())
            }) {
                let _ = fs::remove_dir_all(temporary);
                return Err(error).with_context(|| {
                    format!("create canonical skill {}", plan.canonical.display())
                });
            }
            None
        }
    };
    if fingerprint_with_ignores(&plan.canonical, &plan.ignore)? != plan.fingerprint {
        if let Some(source) = &moved_source {
            let _ = fs::rename(&plan.canonical, source);
        } else if created_canonical {
            let _ = fs::remove_dir_all(&plan.canonical);
        }
        bail!(
            "canonical verification failed for {}",
            plan.canonical.display()
        );
    }
    let mut staged = Vec::<(PathBuf, PathBuf)>::new();
    let mut created_links = Vec::<PathBuf>::new();
    let operation = (|| -> Result<()> {
        for replacement in &plan.replacements {
            if let Some(backup) = &replacement.backup {
                fs::rename(&replacement.original, backup)
                    .with_context(|| format!("stage {}", replacement.original.display()))?;
                staged.push((replacement.original.clone(), backup.clone()));
            }
            if let Some(parent) = replacement.original.parent() {
                fs::create_dir_all(parent)?;
            }
            create_managed_symlink(&plan.canonical, &replacement.original, plan.relative_links)?;
            created_links.push(replacement.original.clone());
            verify_link(&replacement.original, &plan.canonical)?;
        }
        Ok(())
    })();
    if let Err(error) = operation {
        for link in created_links.iter().rev() {
            if fs::symlink_metadata(link).is_ok() {
                let _ = fs::remove_file(link);
            }
        }
        for (original, backup) in staged.iter().rev() {
            let _ = fs::rename(backup, original);
        }
        if let Some(source) = &moved_source {
            let _ = fs::rename(&plan.canonical, source);
        } else if created_canonical {
            let _ = fs::remove_dir_all(&plan.canonical);
        }
        return Err(error).context("migration rolled back");
    }
    for (_, backup) in &staged {
        if let Err(error) = fs::remove_dir_all(backup) {
            eprintln!(
                "Warning: could not remove recovery copy {}: {error}",
                backup.display()
            );
        }
    }
    for preserved in &plan.preserved {
        println!(
            "{} preserved {} version at:\n  {}",
            theme::warn(),
            theme::agent(&target_label(&preserved.target)),
            theme::path_text(&display_path(&preserved.destination))
        );
    }
    Ok(())
}

fn unique_sibling(path: &Path, purpose: &str) -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .unwrap_or_else(|| OsStr::new("skill"))
        .to_string_lossy();
    path.with_file_name(format!(
        ".skill-issue-{purpose}-{name}-{}-{sequence}",
        std::process::id()
    ))
}

fn copy_skill(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir(destination)?;
    let mut directories = vec![(source.to_path_buf(), destination.to_path_buf())];
    for entry in WalkDir::new(source).follow_links(false).min_depth(1) {
        let entry = entry?;
        let relative = entry.path().strip_prefix(source)?;
        let output = destination.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir(&output)?;
            directories.push((entry.path().to_path_buf(), output));
        } else if entry.file_type().is_file() {
            fs::copy(entry.path(), &output)?;
            copy_metadata(entry.path(), &output)?;
        } else if entry.file_type().is_symlink() {
            create_symlink(&fs::read_link(entry.path())?, &output)?;
        } else {
            bail!("unsupported special file: {}", entry.path().display());
        }
    }
    for (input, output) in directories.into_iter().rev() {
        copy_metadata(&input, &output)?;
    }
    Ok(())
}

fn copy_metadata(input: &Path, output: &Path) -> Result<()> {
    let metadata = fs::metadata(input)?;
    fs::set_permissions(output, metadata.permissions())?;
    filetime::set_file_times(
        output,
        filetime::FileTime::from_last_access_time(&metadata),
        filetime::FileTime::from_last_modification_time(&metadata),
    )?;
    copy_extended_attributes(input, output)?;
    Ok(())
}

#[cfg(unix)]
fn copy_extended_attributes(input: &Path, output: &Path) -> Result<()> {
    for name in
        xattr::list(input).with_context(|| format!("read attributes from {}", input.display()))?
    {
        if let Some(value) = xattr::get(input, &name)? {
            xattr::set(output, &name, &value)
                .with_context(|| format!("copy attribute {:?} to {}", name, output.display()))?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn copy_extended_attributes(_input: &Path, _output: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn create_symlink(target: &Path, link: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, link)
        .with_context(|| format!("link {} -> {}", link.display(), target.display()))
}

#[cfg(not(unix))]
fn create_symlink(_target: &Path, _link: &Path) -> Result<()> {
    bail!("symlink mutations are supported only on macOS and Linux")
}

fn managed_link_value(target: &Path, link: &Path, relative: bool) -> PathBuf {
    if relative {
        pathdiff::diff_paths(target, link.parent().unwrap_or(Path::new(".")))
            .unwrap_or_else(|| target.to_path_buf())
    } else {
        target.to_path_buf()
    }
}

fn create_managed_symlink(target: &Path, link: &Path, relative: bool) -> Result<()> {
    create_symlink(&managed_link_value(target, link, relative), link)
}

fn verify_link(link: &Path, expected: &Path) -> Result<()> {
    let raw = fs::read_link(link)?;
    let resolved = resolve_link_path(link, &raw);
    if resolved != expected || !resolved.is_dir() {
        bail!("link verification failed: {}", link.display());
    }
    Ok(())
}

fn resolve_link_path(link: &Path, raw_target: &Path) -> PathBuf {
    if raw_target.is_absolute() {
        normalize_lexical(raw_target)
    } else {
        normalize_lexical(&link.parent().unwrap_or(Path::new(".")).join(raw_target))
    }
}

fn doctor_command(config: &Config, fix: bool, dry_run: bool) -> Result<u8> {
    if JSON_OUTPUT.load(Ordering::Relaxed) {
        if fix {
            bail!("--json cannot be combined with --fix");
        }
        let result = scan(config)?;
        let recovery = recovery_artifacts(config)?;
        let mut issues = Vec::new();
        if !config.root.is_dir() {
            issues.push(serde_json::json!({"kind": "missing_canonical_root", "path": config.root}));
        }
        for target in &result.missing_targets {
            issues.push(serde_json::json!({"kind": "missing_target", "target": target}));
        }
        for path in recovery {
            issues.push(serde_json::json!({"kind": "recovery_artifact", "path": path}));
        }
        for group in result.groups.values() {
            for installation in &group.installations {
                if installation.kind != InstallationKind::ManagedSymlink {
                    issues.push(serde_json::json!({
                        "kind": installation.kind,
                        "skill": group.name,
                        "target": installation.target,
                        "path": installation.path,
                    }));
                }
            }
        }
        let code = result_exit(&result).max(if issues.is_empty() {
            EXIT_OK
        } else {
            EXIT_ISSUES
        });
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"healthy": issues.is_empty(), "issues": issues})
            )?
        );
        return Ok(code);
    }
    println!("{}", theme::banner("Checking skill-issue..."));
    println!();
    let root_ok = config.root.is_dir();
    println!(
        "{} Canonical directory {}",
        if root_ok { theme::ok() } else { theme::bad() },
        if root_ok {
            theme::good_count("exists")
        } else {
            theme::bad_count("is missing")
        }
    );
    let result = scan(config)?;
    for (target, target_config) in config.targets.iter().filter(|(_, t)| t.enabled) {
        let healthy = target_config.path.is_dir();
        println!(
            "{} {} target {}",
            if healthy { theme::ok() } else { theme::warn() },
            theme::agent(target),
            if healthy {
                theme::good_count("healthy")
            } else {
                theme::warn_count("missing")
            }
        );
    }
    let recovery = recovery_artifacts(config)?;
    let mut issue_count = usize::from(!root_ok) + result.missing_targets.len() + recovery.len();
    for path in &recovery {
        println!(
            "{} Recovery artifact from an interrupted migration: {}",
            theme::warn(),
            theme::path(path)
        );
    }
    for group in result.groups.values() {
        for installation in &group.installations {
            match installation.kind {
                InstallationKind::ManagedSymlink => {}
                InstallationKind::Physical if group.canonical.is_none() => {
                    issue_count += 1;
                    print_doctor_issue(issue_count, &installation.path, "is not adopted");
                }
                InstallationKind::Physical => {
                    issue_count += 1;
                    print_doctor_issue(
                        issue_count,
                        &installation.path,
                        "is a physical copy beside a canonical skill",
                    );
                }
                InstallationKind::ForeignSymlink => {
                    issue_count += 1;
                    print_doctor_issue(issue_count, &installation.path, "is a foreign symlink");
                }
                InstallationKind::BrokenSymlink => {
                    issue_count += 1;
                    print_doctor_issue(issue_count, &installation.path, "is a broken symlink");
                }
            }
        }
    }
    if issue_count == 0 {
        println!("\n{} No skill issues.", theme::ok());
        return Ok(EXIT_OK);
    }
    println!(
        "\n{} {} issue{} found",
        theme::warn(),
        theme::warn_count(issue_count),
        if issue_count == 1 { "" } else { "s" }
    );
    if !fix {
        println!("Run: {}", theme::hint("si doctor --fix"));
        return Ok(result_exit(&result).max(EXIT_ISSUES));
    }

    let mut adoptions = Vec::new();
    let mut link_repairs = Vec::<(PathBuf, PathBuf, PathBuf)>::new();
    for group in result.groups.values() {
        if let Some(canonical) = &group.canonical {
            let matching = group
                .installations
                .iter()
                .filter(|i| {
                    i.kind == InstallationKind::Physical
                        && i.fingerprint.as_ref() == Some(&canonical.fingerprint)
                })
                .map(|i| i.path.clone())
                .collect();
            if let Some(plan) = plan_existing(
                &group.name,
                canonical,
                matching,
                &config.ignore,
                config.relative_links,
            ) {
                adoptions.push(plan);
            }
            for installation in group
                .installations
                .iter()
                .filter(|i| i.kind == InstallationKind::BrokenSymlink)
            {
                let raw = fs::read_link(&installation.path)?;
                link_repairs.push((installation.path.clone(), canonical.path.clone(), raw));
            }
        }
    }
    if adoptions.is_empty() && link_repairs.is_empty() {
        println!(
            "No issues can be fixed automatically; divergent and foreign content was left untouched."
        );
        return Ok(result_exit(&result).max(EXIT_ISSUES));
    }
    if !adoptions.is_empty() {
        render_adoption_plans(&adoptions);
    } else {
        println!("{}", theme::heading("PLAN"));
    }
    for (link, target, _) in &link_repairs {
        println!(
            "{}\n  {}\n    {} {}",
            theme::action("RECREATE LINK"),
            theme::path(link),
            theme::arrow(),
            theme::path(target)
        );
    }
    if dry_run {
        println!("{}", theme::dim("Dry run; no files changed."));
        return Ok(result_exit(&result));
    }
    require_confirmation("Perform these unambiguous repairs?")?;
    for plan in &adoptions {
        execute_adoption(plan)?;
    }
    for (link, target, old_target) in &link_repairs {
        fs::remove_file(link)?;
        if let Err(error) = create_managed_symlink(target, link, config.relative_links)
            .and_then(|_| verify_link(link, target))
        {
            if fs::symlink_metadata(link).is_ok() {
                let _ = fs::remove_file(link);
            }
            let _ = create_symlink(old_target, link);
            return Err(error).context("repair broken link");
        }
    }
    let after = scan(config)?;
    if result_exit(&after) == EXIT_OK {
        println!("{} No skill issues.", theme::ok());
    }
    Ok(result_exit(&after))
}

fn print_doctor_issue(number: usize, path: &Path, problem: &str) {
    println!(
        "{}. {} {}",
        theme::warn_count(number),
        theme::path(path),
        theme::dim(problem)
    );
}

fn recovery_artifacts(config: &Config) -> Result<Vec<PathBuf>> {
    let mut artifacts = Vec::new();
    let locations = std::iter::once(&config.root).chain(
        config
            .targets
            .values()
            .filter(|target| target.enabled)
            .map(|target| &target.path),
    );
    for location in locations.filter(|path| path.is_dir()) {
        for entry in sorted_children(location)? {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(".skill-issue-") || name.starts_with(".skillissue-") {
                artifacts.push(entry.path());
            }
        }
    }
    Ok(artifacts)
}

fn require_confirmation(prompt: &str) -> Result<()> {
    require_confirmation_with_default(prompt, false)
}

fn require_confirmation_with_default(prompt: &str, default: bool) -> Result<()> {
    if ASSUME_YES.load(Ordering::Relaxed) {
        return Ok(());
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("this mutation requires an interactive confirmation; use --dry-run to inspect it");
    }
    if !Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt)
        .default(default)
        .interact()?
    {
        bail!("cancelled; no files were changed");
    }
    Ok(())
}

#[derive(Clone)]
struct ManifestEntry {
    kind: u8,
    hash: String,
    source: PathBuf,
}

#[derive(Serialize)]
struct DiffChange {
    status: char,
    path: String,
    content: Option<String>,
}

fn directory_manifest(root: &Path, ignore: &[String]) -> Result<BTreeMap<String, ManifestEntry>> {
    let ignores = build_ignore_set(ignore)?;
    let mut manifest = BTreeMap::new();
    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| include_entry(entry, root, &ignores));
    for entry in walker {
        let entry = entry?;
        if entry.path() == root || entry.file_type().is_dir() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        if entry.file_type().is_file() {
            let mut hasher = Hasher::new();
            let mut reader = BufReader::new(File::open(entry.path())?);
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let read = reader.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
            }
            manifest.insert(
                relative,
                ManifestEntry {
                    kind: b'F',
                    hash: hasher.finalize().to_hex().to_string(),
                    source: entry.path().to_path_buf(),
                },
            );
        } else if entry.file_type().is_symlink() {
            manifest.insert(
                relative,
                ManifestEntry {
                    kind: b'L',
                    hash: fs::read_link(entry.path())?.to_string_lossy().to_string(),
                    source: entry.path().to_path_buf(),
                },
            );
        } else {
            bail!("unsupported special file: {}", entry.path().display());
        }
    }
    Ok(manifest)
}

fn diff_command(config: &Config, skill: &str, content: bool) -> Result<u8> {
    let result = scan(config)?;
    let group = result
        .groups
        .get(skill)
        .ok_or_else(|| anyhow!("skill `{skill}` was not found"))?;
    diff_group(group, content, &config.ignore)
}

fn diff_group(group: &SkillGroup, content: bool, ignore: &[String]) -> Result<u8> {
    let mut copies = Vec::<(String, PathBuf, String)>::new();
    if let Some(canonical) = &group.canonical {
        copies.push((
            "canonical".into(),
            canonical.path.clone(),
            canonical.fingerprint.clone(),
        ));
    }
    for installation in &group.installations {
        if let Some(fp) = &installation.fingerprint {
            copies.push((
                installation.target.clone(),
                installation.path.clone(),
                fp.clone(),
            ));
        }
    }
    if copies.len() < 2 {
        bail!("`{}` has only one readable copy", group.name);
    }
    let first = 0;
    let second = copies
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, c)| c.2 != copies[first].2)
        .map(|(i, _)| i)
        .unwrap_or(1);
    let left = &copies[first];
    let right = &copies[second];
    let left_manifest = directory_manifest(&left.1, ignore)?;
    let right_manifest = directory_manifest(&right.1, ignore)?;
    let names: BTreeSet<_> = left_manifest
        .keys()
        .chain(right_manifest.keys())
        .cloned()
        .collect();
    let mut changes = Vec::new();
    for name in names {
        match (left_manifest.get(&name), right_manifest.get(&name)) {
            (None, Some(_)) => {
                changes.push(DiffChange {
                    status: 'A',
                    path: name,
                    content: None,
                });
            }
            (Some(_), None) => {
                changes.push(DiffChange {
                    status: 'D',
                    path: name,
                    content: None,
                });
            }
            (Some(a), Some(b)) if a.kind != b.kind || a.hash != b.hash => {
                let detail = (content && a.kind == b'F' && b.kind == b'F')
                    .then(|| content_diff(&name, a, b))
                    .transpose()?;
                changes.push(DiffChange {
                    status: 'M',
                    path: name,
                    content: detail,
                });
            }
            _ => {}
        }
    }
    if JSON_OUTPUT.load(Ordering::Relaxed) {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "skill": group.name,
                "left": left.0,
                "right": right.0,
                "changes": changes,
            }))?
        );
        return Ok(if changes.is_empty() {
            EXIT_OK
        } else {
            EXIT_CONFLICTS
        });
    }
    println!(
        "{} {} {}",
        theme::agent(&target_label(&left.0)),
        theme::dim("↔"),
        theme::agent(&target_label(&right.0))
    );
    for change in &changes {
        let mark = match change.status {
            'A' => theme::good_count(change.status),
            'D' => theme::bad_count(change.status),
            _ => theme::warn_count(change.status),
        };
        println!("{mark} {}", theme::skill(&change.path));
        if let Some(content) = &change.content {
            print!("{}", theme::diff(content));
        }
    }
    if changes.is_empty() {
        println!("{} Copies are identical.", theme::ok());
        Ok(EXIT_OK)
    } else {
        Ok(EXIT_CONFLICTS)
    }
}

fn content_diff(name: &str, left: &ManifestEntry, right: &ManifestEntry) -> Result<String> {
    let left_bytes = fs::read(&left.source)?;
    let right_bytes = fs::read(&right.source)?;
    match (
        std::str::from_utf8(&left_bytes),
        std::str::from_utf8(&right_bytes),
    ) {
        (Ok(a), Ok(b)) => Ok(TextDiff::from_lines(a, b)
            .unified_diff()
            .header(&format!("a/{name}"), &format!("b/{name}"))
            .to_string()),
        _ => Ok("  (binary content differs)\n".to_string()),
    }
}

fn link_command(config: &Config, args: LinkArgs, dry_run: bool) -> Result<u8> {
    let all_skills = args.skill.is_none() && args.all;
    let mut target_ids = args.targets;
    target_ids.extend(args.target);
    let inferred_all_targets = target_ids.is_empty() && args.all;
    if inferred_all_targets {
        target_ids = config
            .targets
            .iter()
            .filter(|(_, target)| target.enabled)
            .map(|(id, _)| id.clone())
            .collect();
    }
    target_ids.sort_by_key(|id| target_rank(id));
    target_ids.dedup();
    if target_ids.is_empty() {
        bail!("at least one target is required; use --all for every detected target");
    }
    let skills = if all_skills {
        canonical_skill_names(&config.root, &config.ignore)?
    } else {
        vec![
            args.skill
                .ok_or_else(|| anyhow!("a skill is required unless --all is used"))?,
        ]
    };
    let requested_target_count = target_ids.len();
    let mut links = Vec::new();
    let mut create_targets = BTreeSet::new();
    for target_id in &target_ids {
        let target = enabled_target(config, target_id)?;
        if !target.path.exists() {
            create_targets.insert(target.path.clone());
        }
        for skill in &skills {
            validate_skill_name(skill)?;
            let canonical = config.root.join(skill);
            let canonical_metadata = fs::symlink_metadata(&canonical).with_context(|| {
                format!("canonical skill does not exist: {}", canonical.display())
            })?;
            if !canonical_metadata.is_dir() || canonical_metadata.file_type().is_symlink() {
                bail!(
                    "canonical skill does not exist as a real directory: {}",
                    canonical.display()
                );
            }
            let destination = target.path.join(skill);
            match fs::symlink_metadata(&destination) {
                Ok(metadata)
                    if metadata.file_type().is_symlink()
                        && verify_link(&destination, &canonical).is_ok() =>
                {
                    continue;
                }
                Ok(_) => bail!("{} already exists; nothing changed", destination.display()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    links.push((destination, canonical))
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
    if links.is_empty() {
        println!("{}", theme::dim("All requested links already exist."));
        return Ok(EXIT_OK);
    }
    println!("{}", theme::heading("DETECTED"));
    for target in &target_ids {
        println!("  {} {}", theme::ok(), theme::agent(&target_label(target)));
    }
    if dry_run || VERBOSITY.load(Ordering::Relaxed) > 0 {
        println!("{}", theme::heading("PLAN"));
        for path in &create_targets {
            println!(
                "{}  {}",
                theme::action("CREATE TARGET"),
                theme::path_text(&display_path(path))
            );
        }
        for (link, target) in &links {
            println!(
                "{}\n  {}\n    {} {}",
                theme::action("LINK"),
                theme::path_text(&display_path(link)),
                theme::arrow(),
                theme::path_text(&display_path(&managed_link_value(
                    target,
                    link,
                    config.relative_links
                )))
            );
        }
    }
    if dry_run {
        println!("{}", theme::dim("Dry run; no files changed."));
        return Ok(EXIT_OK);
    }
    let prompt = if inferred_all_targets {
        format!("Link {} skills into all detected agents?", skills.len())
    } else {
        format!(
            "Link {} skills into {} selected agents?",
            skills.len(),
            requested_target_count
        )
    };
    require_confirmation_with_default(&prompt, true)?;
    for path in create_targets {
        fs::create_dir_all(path)?;
    }
    let mut created = Vec::new();
    for (link, target) in &links {
        if let Err(error) = create_managed_symlink(target, link, config.relative_links)
            .and_then(|_| verify_link(link, target))
        {
            for path in &created {
                let _ = fs::remove_file(path);
            }
            return Err(error).context("link operation rolled back");
        }
        created.push(link.clone());
    }
    let created_count = links.len();
    println!(
        "{} {} links created",
        theme::ok(),
        theme::good_count(created_count)
    );
    let after = scan(config)?;
    if result_exit(&after) == EXIT_OK {
        println!("{} No skill issues.", theme::ok());
    }
    Ok(EXIT_OK)
}

fn restore_command(config: &Config, dry_run: bool) -> Result<u8> {
    let targets: Vec<String> = config
        .targets
        .iter()
        .filter(|(_, target)| target.enabled)
        .map(|(id, _)| id.clone())
        .collect();
    if targets.is_empty() {
        bail!("no enabled targets were detected; add one with `si targets add`");
    }
    link_command(
        config,
        LinkArgs {
            skill: None,
            targets: Vec::new(),
            all: true,
            target: targets,
        },
        dry_run,
    )
}

fn sync_command(config: &Config, check: bool, dry_run: bool) -> Result<u8> {
    if check && dry_run {
        bail!("--check is already read-only and cannot be combined with --dry-run");
    }
    if JSON_OUTPUT.load(Ordering::Relaxed) && !check {
        bail!("--json requires `si sync --check`");
    }

    let initial = git_health(&config.root)?;
    if !initial.repository {
        bail!(
            "canonical root is not a Git repository: {}",
            config.root.display()
        );
    }
    if initial.upstream.is_none() {
        return render_sync_check(config, initial, false);
    }
    if dry_run {
        println!("{}", theme::heading("SYNC PLAN"));
        println!(
            "{}    {}",
            theme::action("FETCH"),
            theme::agent(initial.upstream.as_deref().unwrap_or("upstream"))
        );
        println!("{}     fast-forward only", theme::action("PULL"));
        println!(
            "{}   canonical skills into every enabled target",
            theme::action("RELINK")
        );
        println!(
            "{}",
            theme::dim("Dry run; no files changed and no remote refs fetched.")
        );
        return Ok(if sync_is_current(config, &initial)? {
            EXIT_OK
        } else {
            EXIT_ISSUES
        });
    }

    run_git_checked(&config.root, &["fetch", "--quiet"], "fetch upstream")?;
    let fetched = git_health(&config.root)?;
    if check {
        return render_sync_check(config, fetched, true);
    }
    if fetched.dirty {
        bail!("canonical repository has uncommitted changes; commit or stash them before syncing");
    }
    let ahead = fetched.ahead.unwrap_or(0);
    let behind = fetched.behind.unwrap_or(0);
    if ahead > 0 && behind > 0 {
        bail!(
            "canonical repository has diverged ({ahead} ahead, {behind} behind); reconcile it with Git before syncing"
        );
    }

    println!("{}", theme::heading("SYNC PLAN"));
    if behind > 0 {
        println!(
            "{}     {} commit(s), fast-forward only",
            theme::action("PULL"),
            theme::count(behind)
        );
    } else {
        println!(
            "{}     already at the fetched remote revision",
            theme::action("PULL")
        );
    }
    println!(
        "{}   canonical skills into every enabled target",
        theme::action("RELINK")
    );
    require_confirmation_with_default("Synchronize now?", true)?;

    if behind > 0 {
        run_git_checked(
            &config.root,
            &["pull", "--ff-only", "--quiet"],
            "fast-forward canonical repository",
        )?;
        println!("{} Canonical repository updated", theme::ok());
    } else {
        println!("{} Canonical repository already current", theme::ok());
    }

    ASSUME_YES.store(true, Ordering::Relaxed);
    let link_code = restore_command(config, false)?;
    let final_health = git_health(&config.root)?;
    if final_health.ahead.unwrap_or(0) > 0 {
        println!(
            "{} Local canonical commits have not been pushed; run {} in {}",
            theme::warn(),
            theme::hint("git push"),
            theme::path(&config.root)
        );
        return Ok(link_code.max(EXIT_ISSUES));
    }
    println!("{} This computer is in sync.", theme::ok());
    Ok(link_code)
}

fn run_git_checked(root: &Path, args: &[&str], action: &str) -> Result<()> {
    let output = git_output(root, args)?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr);
    bail!("{action} failed: {}", detail.trim());
}

fn sync_is_current(config: &Config, git: &GitHealth) -> Result<bool> {
    Ok(git.repository
        && !git.dirty
        && git.upstream.is_some()
        && git.ahead == Some(0)
        && git.behind == Some(0)
        && result_exit(&scan(config)?) == EXIT_OK)
}

fn render_sync_check(config: &Config, git: GitHealth, fetched: bool) -> Result<u8> {
    let scan = scan(config)?;
    let links_healthy = result_exit(&scan) == EXIT_OK;
    let mut reasons = Vec::new();
    if !git.repository {
        reasons.push("canonical root is not a Git repository".to_string());
    }
    if git.dirty {
        reasons.push("canonical repository has uncommitted changes".to_string());
    }
    if git.upstream.is_none() {
        reasons.push("canonical repository has no upstream".to_string());
    }
    if let Some(ahead) = git.ahead.filter(|count| *count > 0) {
        reasons.push(format!("{ahead} local commit(s) have not been pushed"));
    }
    if let Some(behind) = git.behind.filter(|count| *count > 0) {
        reasons.push(format!("{behind} remote commit(s) have not been pulled"));
    }
    if !links_healthy {
        reasons.push("one or more agent links need repair".to_string());
    }
    let synced = reasons.is_empty();

    if JSON_OUTPUT.load(Ordering::Relaxed) {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "synced": synced,
                "fetched": fetched,
                "git": git,
                "links_healthy": links_healthy,
                "reasons": reasons,
            }))?
        );
    } else {
        println!("{}", theme::banner("sync status"));
        print_git_health(&git, &config.root);
        println!(
            "\n{}",
            if links_healthy {
                format!(
                    "{} agent links {}",
                    theme::ok(),
                    theme::good_count("healthy")
                )
            } else {
                format!(
                    "{} agent links {}",
                    theme::warn(),
                    theme::warn_count("need repair")
                )
            }
        );
        if synced {
            println!("\n{} This computer is in sync.", theme::ok());
        } else {
            println!("\n{}", theme::heading("OUT OF SYNC"));
            for reason in &reasons {
                println!("{} {reason}", theme::warn());
            }
            println!("Run: {}", theme::hint("si sync"));
        }
    }
    Ok(if synced { EXIT_OK } else { EXIT_ISSUES })
}

fn canonical_skill_names(root: &Path, ignore: &[String]) -> Result<Vec<String>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let ignores = build_ignore_set(ignore)?;
    let mut names = Vec::new();
    for entry in sorted_children(root)? {
        let metadata = entry.file_type()?;
        let name = utf8_name(&entry.path())?;
        if metadata.is_dir()
            && !metadata.is_symlink()
            && !ignored_top_level(&name)
            && !ignores.is_match(&name)
        {
            names.push(name);
        }
    }
    Ok(names)
}

fn enabled_target<'a>(config: &'a Config, id: &str) -> Result<&'a TargetConfig> {
    let target = config
        .targets
        .get(id)
        .ok_or_else(|| anyhow!("unknown target `{id}`"))?;
    if !target.enabled {
        bail!("target `{id}` is disabled");
    }
    Ok(target)
}

fn unlink_command(config: &Config, args: UnlinkArgs, dry_run: bool) -> Result<u8> {
    let mut targets = args.targets;
    targets.extend(args.target);
    targets.sort();
    targets.dedup();
    if targets.is_empty() {
        bail!("at least one target is required");
    }
    let mut links = Vec::new();
    for id in &targets {
        let path = enabled_target(config, id)?.path.join(&args.skill);
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("{} does not exist", path.display()))?;
        if !metadata.file_type().is_symlink() {
            bail!("refusing to unlink physical directory {}", path.display());
        }
        let raw = fs::read_link(&path)?;
        let resolved = resolve_link_path(&path, &raw);
        if !is_canonical_skill_path(&config.root, &resolved) {
            bail!("refusing to remove foreign symlink {}", path.display());
        }
        links.push(path);
    }
    println!("{}", theme::heading("PLAN"));
    for path in &links {
        println!("{}  {}", theme::action("UNLINK"), theme::path(path));
    }
    println!(
        "Canonical skill remains: {}",
        theme::path(&config.root.join(&args.skill))
    );
    if dry_run {
        println!("{}", theme::dim("Dry run; no files changed."));
        return Ok(EXIT_OK);
    }
    require_confirmation("Remove these links?")?;
    for path in links {
        fs::remove_file(&path)?;
        println!("{} Removed {}", theme::ok(), theme::path(&path));
    }
    Ok(EXIT_OK)
}

#[derive(Clone, Debug)]
pub(crate) struct SkillTogglePlan {
    pub skill: String,
    pub enable: bool,
    actions: Vec<SkillToggleAction>,
    create_targets: Vec<PathBuf>,
    relative_links: bool,
}

#[derive(Clone, Debug)]
enum SkillToggleAction {
    Link {
        path: PathBuf,
        canonical: PathBuf,
    },
    Unlink {
        path: PathBuf,
        raw_target: PathBuf,
        canonical: PathBuf,
    },
}

impl SkillTogglePlan {
    pub(crate) fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    pub(crate) fn action_count(&self) -> usize {
        self.actions.len()
    }

    pub(crate) fn paths(&self) -> impl Iterator<Item = &Path> {
        self.actions.iter().map(|action| match action {
            SkillToggleAction::Link { path, .. } | SkillToggleAction::Unlink { path, .. } => {
                path.as_path()
            }
        })
    }

    pub(crate) fn apply(&self) -> Result<()> {
        if self.enable {
            self.apply_links()
        } else {
            self.apply_unlinks()
        }
    }

    fn apply_links(&self) -> Result<()> {
        for path in &self.create_targets {
            fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))?;
        }
        let mut created = Vec::new();
        for action in &self.actions {
            let SkillToggleAction::Link { path, canonical } = action else {
                continue;
            };
            if let Err(error) = create_managed_symlink(canonical, path, self.relative_links)
                .and_then(|_| verify_link(path, canonical))
            {
                for created_path in created.iter().rev() {
                    let _ = fs::remove_file(created_path);
                }
                return Err(error).context("enable operation rolled back");
            }
            created.push(path.clone());
        }
        Ok(())
    }

    fn apply_unlinks(&self) -> Result<()> {
        let mut removed = Vec::<(PathBuf, PathBuf)>::new();
        for action in &self.actions {
            let SkillToggleAction::Unlink {
                path,
                raw_target,
                canonical,
            } = action
            else {
                continue;
            };
            if let Err(error) = verify_link(path, canonical).and_then(|_| {
                fs::remove_file(path).with_context(|| format!("remove {}", path.display()))
            }) {
                for (removed_path, removed_target) in removed.iter().rev() {
                    let _ = create_symlink(removed_target, removed_path);
                }
                return Err(error).context("disable operation rolled back");
            }
            removed.push((path.clone(), raw_target.clone()));
        }
        Ok(())
    }
}

pub(crate) fn plan_skill_toggle(
    config: &Config,
    skill: &str,
    enable: bool,
) -> Result<SkillTogglePlan> {
    validate_skill_name(skill)?;
    let canonical = config.root.join(skill);
    let metadata = fs::symlink_metadata(&canonical)
        .with_context(|| format!("canonical skill does not exist: {}", canonical.display()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!(
            "canonical skill does not exist as a real directory: {}",
            canonical.display()
        );
    }
    if enable && !config.targets.values().any(|target| target.enabled) {
        bail!("no enabled targets were detected; add one with `si targets add`");
    }

    let mut actions = Vec::new();
    let mut create_targets = BTreeSet::new();
    for target in config.targets.values().filter(|target| target.enabled) {
        let path = target.path.join(skill);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let raw_target = fs::read_link(&path)?;
                let resolved = resolve_link_path(&path, &raw_target);
                if resolved != canonical {
                    bail!(
                        "refusing to {} foreign symlink {}",
                        if enable { "replace" } else { "remove" },
                        path.display()
                    );
                }
                if !enable {
                    actions.push(SkillToggleAction::Unlink {
                        path,
                        raw_target,
                        canonical: canonical.clone(),
                    });
                }
            }
            Ok(_) => bail!(
                "refusing to {} physical directory {}",
                if enable { "replace" } else { "remove" },
                path.display()
            ),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if enable {
                    if !target.path.exists() {
                        create_targets.insert(target.path.clone());
                    }
                    actions.push(SkillToggleAction::Link {
                        path,
                        canonical: canonical.clone(),
                    });
                }
            }
            Err(error) => return Err(error.into()),
        }
    }

    Ok(SkillTogglePlan {
        skill: skill.to_string(),
        enable,
        actions,
        create_targets: create_targets.into_iter().collect(),
        relative_links: config.relative_links,
    })
}

fn toggle_skill_command(config: &Config, skill: &str, enable: bool, dry_run: bool) -> Result<u8> {
    let plan = plan_skill_toggle(config, skill, enable)?;
    let verb = if enable { "ENABLE" } else { "DISABLE" };
    if plan.is_empty() {
        println!(
            "{} is already {}.",
            theme::skill(skill),
            if enable { "enabled" } else { "disabled" }
        );
        return Ok(EXIT_OK);
    }
    println!("{}", theme::heading("PLAN"));
    for path in plan.paths() {
        println!("{}  {}", theme::action(verb), theme::path(path));
    }
    println!(
        "Canonical skill remains: {}",
        theme::path(&config.root.join(skill))
    );
    if dry_run {
        println!("{}", theme::dim("Dry run; no files changed."));
        return Ok(EXIT_OK);
    }
    require_confirmation(&format!(
        "{} {skill} for all configured agents?",
        if enable { "Enable" } else { "Disable" }
    ))?;
    plan.apply()?;
    println!(
        "{} {} {} for {} agents",
        theme::ok(),
        theme::skill(skill),
        if enable { "enabled" } else { "disabled" },
        theme::good_count(plan.action_count())
    );
    Ok(EXIT_OK)
}

#[derive(Clone, Debug)]
pub(crate) struct SkillDeletePlan {
    pub skill: String,
    canonical: PathBuf,
    links: Vec<PathBuf>,
}

impl SkillDeletePlan {
    pub(crate) fn link_count(&self) -> usize {
        self.links.len()
    }

    pub(crate) fn links(&self) -> impl Iterator<Item = &Path> {
        self.links.iter().map(PathBuf::as_path)
    }

    pub(crate) fn canonical(&self) -> &Path {
        &self.canonical
    }

    pub(crate) fn apply(&self) -> Result<()> {
        for link in &self.links {
            fs::remove_file(link).with_context(|| format!("remove {}", link.display()))?;
        }
        fs::remove_dir_all(&self.canonical)
            .with_context(|| format!("remove {}", self.canonical.display()))?;
        Ok(())
    }
}

pub(crate) fn plan_skill_delete(config: &Config, skill: &str) -> Result<SkillDeletePlan> {
    validate_skill_name(skill)?;
    let canonical = config.root.join(skill);
    let metadata = fs::symlink_metadata(&canonical)
        .with_context(|| format!("canonical skill does not exist: {}", canonical.display()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!(
            "canonical skill does not exist as a real directory: {}",
            canonical.display()
        );
    }
    let mut links = Vec::new();
    for target in config.targets.values().filter(|target| target.enabled) {
        let path = target.path.join(skill);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let raw_target = fs::read_link(&path)?;
                let resolved = resolve_link_path(&path, &raw_target);
                if resolved == canonical {
                    links.push(path);
                }
            }
            Ok(_) | Err(_) => {}
        }
    }
    Ok(SkillDeletePlan {
        skill: skill.to_string(),
        canonical,
        links,
    })
}

fn delete_skill_command(config: &Config, skill: &str, dry_run: bool) -> Result<u8> {
    let plan = plan_skill_delete(config, skill)?;
    println!("{}", theme::heading("PLAN"));
    for link in plan.links() {
        println!("{}  {}", theme::action("UNLINK"), theme::path(link));
    }
    println!(
        "{}  {}",
        theme::action("DELETE"),
        theme::path(plan.canonical())
    );
    if dry_run {
        println!("{}", theme::dim("Dry run; no files changed."));
        return Ok(EXIT_OK);
    }
    require_confirmation(&format!(
        "Permanently delete `{skill}` and its {} link{}? This cannot be undone.",
        plan.link_count(),
        if plan.link_count() == 1 { "" } else { "s" }
    ))?;
    plan.apply()?;
    println!("{} Deleted {}", theme::ok(), theme::skill(skill));
    Ok(EXIT_OK)
}

fn targets_command(
    mut config: Config,
    command: Option<TargetCommand>,
    dry_run: bool,
) -> Result<u8> {
    match command {
        None => {
            if JSON_OUTPUT.load(Ordering::Relaxed) {
                println!("{}", serde_json::to_string_pretty(&config.targets)?);
                return Ok(EXIT_OK);
            }
            println!(
                "{}",
                theme::dim("TARGET       PATH                                      STATUS")
            );
            for (id, target) in &config.targets {
                let status = if !target.enabled {
                    theme::dim("disabled")
                } else if target.path.is_dir() {
                    theme::good_count("✓")
                } else {
                    theme::warn_count("missing")
                };
                println!(
                    "{} {} {status}",
                    theme::agent_padded(id, 12),
                    theme::path_text(&format!("{:<41}", target.path.display()))
                );
            }
        }
        Some(TargetCommand::Add { id, path }) => {
            validate_target_id(&id)?;
            let path = absolute_path(&path)?;
            if config.targets.contains_key(&id) {
                bail!("target `{id}` already exists");
            }
            config.targets.insert(
                id.clone(),
                TargetConfig {
                    path: path.clone(),
                    enabled: true,
                },
            );
            validate_config(&config)?;
            println!(
                "{} {}  {}",
                theme::action("ADD TARGET"),
                theme::agent(&id),
                theme::path(&path)
            );
            if !dry_run {
                config.save()?;
                println!("{} Target added", theme::ok());
            } else {
                println!("{}", theme::dim("Dry run; no files changed."));
            }
        }
        Some(TargetCommand::Remove { id }) => {
            if config.targets.remove(&id).is_none() {
                bail!("unknown target `{id}`");
            }
            println!(
                "{} {}\n{}",
                theme::action("REMOVE TARGET"),
                theme::agent(&id),
                theme::dim("Filesystem data will not be removed.")
            );
            if !dry_run {
                config.save()?;
                println!("{} Target removed from configuration", theme::ok());
            } else {
                println!("{}", theme::dim("Dry run; no files changed."));
            }
        }
    }
    Ok(EXIT_OK)
}

fn validate_target_id(id: &str) -> Result<()> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        || !id.chars().next().unwrap().is_ascii_alphanumeric()
    {
        bail!(
            "target id must start with a lowercase letter or digit and contain only lowercase letters, digits, '-' or '_'"
        );
    }
    Ok(())
}

fn config_command(mut config: Config, command: Option<ConfigCommand>, dry_run: bool) -> Result<u8> {
    match command {
        None => {
            if JSON_OUTPUT.load(Ordering::Relaxed) {
                println!("{}", serde_json::to_string_pretty(&config)?);
            } else {
                print!("{}", toml::to_string_pretty(&config)?);
            }
        }
        Some(ConfigCommand::SetRoot { root }) => {
            let new_root = absolute_path(&root)?;
            if new_root == config.root {
                println!(
                    "Canonical root is already {}",
                    theme::path_text(&new_root.display().to_string())
                );
                return Ok(EXIT_OK);
            }
            let current = scan(&config)?;
            let has_managed_state = current.groups.values().any(|g| {
                g.canonical.is_some()
                    || g.installations
                        .iter()
                        .any(|i| i.kind == InstallationKind::ManagedSymlink)
            });
            if has_managed_state {
                bail!(
                    "refusing to change the root while canonical skills or managed links exist; move them safely before updating configuration"
                );
            }
            let old = config.root.clone();
            config.root = new_root.clone();
            validate_config(&config)?;
            println!(
                "{}\n  {}\n    {} {}",
                theme::action("SET ROOT"),
                theme::path(&old),
                theme::arrow(),
                theme::path(&new_root)
            );
            if !dry_run {
                fs::create_dir_all(&new_root)?;
                config.save()?;
                println!("{} Canonical root updated", theme::ok());
            } else {
                println!("{}", theme::dim("Dry run; no files changed."));
            }
        }
        Some(ConfigCommand::SetRelativeLinks { enabled }) => {
            println!(
                "{}  {}",
                theme::action("SET RELATIVE LINKS"),
                theme::count(enabled)
            );
            config.relative_links = enabled;
            if !dry_run {
                config.save()?;
                println!(
                    "{} Link style updated; existing links are unchanged",
                    theme::ok()
                );
            } else {
                println!("{}", theme::dim("Dry run; no files changed."));
            }
        }
        Some(ConfigCommand::AddIgnore { pattern }) => {
            build_ignore_set(std::slice::from_ref(&pattern))?;
            if config.ignore.contains(&pattern) {
                bail!("ignore pattern already exists: {pattern}");
            }
            println!(
                "{}  {}",
                theme::action("ADD IGNORE"),
                theme::skill(&pattern)
            );
            config.ignore.push(pattern);
            if !dry_run {
                config.save()?;
            } else {
                println!("{}", theme::dim("Dry run; no files changed."));
            }
        }
        Some(ConfigCommand::RemoveIgnore { pattern }) => {
            let before = config.ignore.len();
            config.ignore.retain(|item| item != &pattern);
            if config.ignore.len() == before {
                bail!("ignore pattern not found: {pattern}");
            }
            println!(
                "{}  {}",
                theme::action("REMOVE IGNORE"),
                theme::skill(&pattern)
            );
            if !dry_run {
                config.save()?;
            } else {
                println!("{}", theme::dim("Dry run; no files changed."));
            }
        }
    }
    Ok(EXIT_OK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture() -> (tempfile::TempDir, Config) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("canonical");
        let claude = temp.path().join("claude");
        let codex = temp.path().join("codex");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&claude).unwrap();
        fs::create_dir_all(&codex).unwrap();
        let targets = BTreeMap::from([
            (
                "claude".to_string(),
                TargetConfig {
                    path: claude,
                    enabled: true,
                },
            ),
            (
                "codex".to_string(),
                TargetConfig {
                    path: codex,
                    enabled: true,
                },
            ),
        ]);
        (
            temp,
            Config {
                root,
                targets,
                relative_links: false,
                ignore: Vec::new(),
            },
        )
    }

    fn skill(path: &Path, body: &str) {
        fs::create_dir_all(path.join("references")).unwrap();
        fs::write(path.join("SKILL.md"), body).unwrap();
        fs::write(path.join("references/note.md"), "note").unwrap();
    }

    #[test]
    fn fingerprints_copies_deterministically_and_ignores_noise() {
        let (temp, _) = fixture();
        let one = temp.path().join("one");
        let two = temp.path().join("two");
        skill(&one, "hello");
        skill(&two, "hello");
        fs::create_dir_all(one.join(".git")).unwrap();
        fs::write(one.join(".git/index"), "ignored").unwrap();
        fs::write(one.join(".DS_Store"), "ignored").unwrap();
        assert_eq!(fingerprint(&one).unwrap(), fingerprint(&two).unwrap());
        fs::write(two.join("SKILL.md"), "changed").unwrap();
        assert_ne!(fingerprint(&one).unwrap(), fingerprint(&two).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn fingerprints_internal_symlinks_without_following_them() {
        let (temp, _) = fixture();
        let one = temp.path().join("one");
        let two = temp.path().join("two");
        skill(&one, "hello");
        skill(&two, "hello");
        std::os::unix::fs::symlink("SKILL.md", one.join("alias")).unwrap();
        std::os::unix::fs::symlink("SKILL.md", two.join("alias")).unwrap();
        assert_eq!(fingerprint(&one).unwrap(), fingerprint(&two).unwrap());
        fs::remove_file(two.join("alias")).unwrap();
        std::os::unix::fs::symlink("references/note.md", two.join("alias")).unwrap();
        assert_ne!(fingerprint(&one).unwrap(), fingerprint(&two).unwrap());
    }

    #[test]
    fn custom_ignore_patterns_exclude_matching_content() {
        let (temp, _) = fixture();
        let one = temp.path().join("one");
        let two = temp.path().join("two");
        skill(&one, "same");
        skill(&two, "same");
        fs::write(one.join("generated.log"), "one").unwrap();
        fs::write(two.join("generated.log"), "two").unwrap();
        assert_ne!(fingerprint(&one).unwrap(), fingerprint(&two).unwrap());
        let ignore = vec!["*.log".to_string()];
        assert_eq!(
            fingerprint_with_ignores(&one, &ignore).unwrap(),
            fingerprint_with_ignores(&two, &ignore).unwrap()
        );
    }

    #[test]
    fn old_configs_receive_v02_defaults() {
        let config: Config = toml::from_str("root = '/tmp/skills'\n").unwrap();
        assert!(!config.relative_links);
        assert!(config.ignore.is_empty());
    }

    #[test]
    fn scan_classifies_identical_and_divergent_skills() {
        let (_temp, config) = fixture();
        skill(&config.targets["claude"].path.join("same"), "same");
        skill(&config.targets["codex"].path.join("same"), "same");
        skill(&config.targets["claude"].path.join("different"), "one");
        skill(&config.targets["codex"].path.join("different"), "two");
        let result = scan(&config).unwrap();
        assert_eq!(
            result.groups["same"].status(),
            SkillStatus::IdenticalDuplicate
        );
        assert_eq!(result.groups["different"].status(), SkillStatus::Divergent);
    }

    #[cfg(unix)]
    #[test]
    fn adoption_creates_one_canonical_copy_and_verified_links() {
        let (_temp, config) = fixture();
        let claude = config.targets["claude"].path.join("foo");
        let codex = config.targets["codex"].path.join("foo");
        skill(&claude, "same");
        skill(&codex, "same");
        let expected = fingerprint(&claude).unwrap();
        let plan = plan_new(
            &config,
            "foo",
            "foo",
            expected.clone(),
            vec![claude.clone(), codex.clone()],
        )
        .unwrap();
        execute_adoption(&plan).unwrap();
        let canonical = config.root.join("foo");
        assert!(canonical.is_dir());
        assert_eq!(fingerprint(&canonical).unwrap(), expected);
        assert_eq!(fs::read_link(&claude).unwrap(), canonical);
        assert_eq!(fs::read_link(&codex).unwrap(), canonical);
        assert_eq!(
            scan(&config).unwrap().groups["foo"].status(),
            SkillStatus::Managed
        );
    }

    #[cfg(unix)]
    #[test]
    fn adoption_can_create_relative_managed_links() {
        let (_temp, mut config) = fixture();
        config.relative_links = true;
        let original = config.targets["claude"].path.join("foo");
        skill(&original, "same");
        let plan = plan_new(
            &config,
            "foo",
            "foo",
            fingerprint(&original).unwrap(),
            vec![original.clone()],
        )
        .unwrap();
        execute_adoption(&plan).unwrap();
        let raw = fs::read_link(&original).unwrap();
        assert!(raw.is_relative());
        verify_link(&original, &config.root.join("foo")).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn managed_foreign_and_broken_links_are_distinguished() {
        let (_temp, config) = fixture();
        let canonical = config.root.join("foo");
        skill(&canonical, "body");
        std::os::unix::fs::symlink(&canonical, config.targets["claude"].path.join("foo")).unwrap();
        let foreign_target = config.root.parent().unwrap().join("foreign");
        skill(&foreign_target, "body");
        std::os::unix::fs::symlink(
            &foreign_target,
            config.targets["codex"].path.join("foreign"),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            config.root.join("missing"),
            config.targets["codex"].path.join("missing"),
        )
        .unwrap();
        let result = scan(&config).unwrap();
        assert_eq!(
            result.groups["foo"].installations[0].kind,
            InstallationKind::ManagedSymlink
        );
        assert_eq!(
            result.groups["foreign"].installations[0].kind,
            InstallationKind::ForeignSymlink
        );
        assert_eq!(
            result.groups["missing"].installations[0].kind,
            InstallationKind::BrokenSymlink
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_loops_are_reported_as_broken_without_recursing() {
        let (_temp, config) = fixture();
        let loop_path = config.targets["claude"].path.join("loop");
        std::os::unix::fs::symlink(&loop_path, &loop_path).unwrap();
        let result = scan(&config).unwrap();
        assert_eq!(
            result.groups["loop"].installations[0].kind,
            InstallationKind::BrokenSymlink
        );
    }

    #[test]
    fn unsafe_root_target_topology_is_rejected() {
        let (_temp, mut config) = fixture();
        config.targets.get_mut("claude").unwrap().path = config.root.join("nested");
        assert!(
            validate_config(&config)
                .unwrap_err()
                .to_string()
                .contains("unsafe topology")
        );
    }

    #[test]
    fn scan_rejects_case_insensitive_collisions_across_targets() {
        let (_temp, config) = fixture();
        skill(&config.targets["claude"].path.join("Rust"), "one");
        skill(&config.targets["codex"].path.join("rust"), "two");
        assert!(
            scan(&config)
                .unwrap_err()
                .to_string()
                .contains("case-insensitive")
        );
    }

    #[test]
    fn canonical_git_metadata_is_not_treated_as_a_skill() {
        let (_temp, config) = fixture();
        fs::create_dir_all(config.root.join(".git/objects")).unwrap();
        fs::write(config.root.join(".git/HEAD"), "ref: refs/heads/main").unwrap();
        skill(&config.root.join("real-skill"), "body");
        let result = scan(&config).unwrap();
        assert_eq!(result.groups.len(), 1);
        assert!(result.groups.contains_key("real-skill"));
    }

    #[cfg(unix)]
    #[test]
    fn canonical_skill_symlinks_are_rejected() {
        let (temp, config) = fixture();
        let external = temp.path().join("external");
        skill(&external, "body");
        std::os::unix::fs::symlink(external, config.root.join("linked")).unwrap();
        assert!(
            scan(&config)
                .unwrap_err()
                .to_string()
                .contains("real directory")
        );
    }

    #[cfg(unix)]
    #[test]
    fn adoption_rolls_back_if_a_later_location_fails() {
        let (_temp, config) = fixture();
        let original = config.targets["claude"].path.join("foo");
        skill(&original, "keep me");
        let expected = fingerprint(&original).unwrap();
        let missing = config.targets["codex"].path.join("missing");
        let plan = plan_new(
            &config,
            "foo",
            "foo",
            expected,
            vec![original.clone(), missing],
        )
        .unwrap();
        assert!(execute_adoption(&plan).is_err());
        assert!(original.is_dir());
        assert_eq!(
            fs::read_to_string(original.join("SKILL.md")).unwrap(),
            "keep me"
        );
        assert!(!config.root.join("foo").exists());
    }

    #[cfg(unix)]
    #[test]
    fn selecting_a_divergent_winner_preserves_the_loser_and_manages_every_target() {
        let (temp, config) = fixture();
        let claude = config.targets["claude"].path.join("android");
        let codex = config.targets["codex"].path.join("android");
        skill(&claude, "winner");
        skill(&codex, "preserve me");
        let result = scan(&config).unwrap();
        let group = &result.groups["android"];
        let selected = fingerprint(&claude).unwrap();
        let archive = temp.path().join("archive/android");
        let plan = plan_selected_version_at(
            &config,
            group,
            selected,
            vec![claude.clone()],
            None,
            archive.clone(),
        )
        .unwrap();
        assert_eq!(plan.preserved.len(), 1);
        execute_adoption(&plan).unwrap();
        assert_eq!(
            fs::read_to_string(archive.join("codex/SKILL.md")).unwrap(),
            "preserve me"
        );
        assert!(archive.join("codex.manifest.json").is_file());
        assert_eq!(fs::read_link(&claude).unwrap(), config.root.join("android"));
        assert_eq!(fs::read_link(&codex).unwrap(), config.root.join("android"));
        assert_eq!(
            scan(&config).unwrap().groups["android"].status(),
            SkillStatus::Managed
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_later_skill_failure_does_not_lose_an_earlier_completed_skill() {
        let (_temp, config) = fixture();
        let first = config.targets["claude"].path.join("first");
        let second = config.targets["claude"].path.join("second");
        skill(&first, "first");
        skill(&second, "second");
        let first_plan = plan_new(
            &config,
            "first",
            "first",
            fingerprint(&first).unwrap(),
            vec![first.clone()],
        )
        .unwrap();
        execute_adoption(&first_plan).unwrap();
        let failed_plan = plan_new(
            &config,
            "second",
            "second",
            fingerprint(&second).unwrap(),
            vec![second.clone(), config.targets["codex"].path.join("missing")],
        )
        .unwrap();
        assert!(execute_adoption(&failed_plan).is_err());
        assert!(config.root.join("first").is_dir());
        assert!(first.is_symlink());
        assert!(second.is_dir());
        assert!(!config.root.join("second").exists());
    }

    #[cfg(unix)]
    #[test]
    fn cross_filesystem_copy_path_preserves_permissions_and_timestamps() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        skill(&source, "body");
        let script = source.join("run.sh");
        fs::write(&script, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o751)).unwrap();
        let timestamp = filetime::FileTime::from_unix_time(1_700_000_000, 0);
        filetime::set_file_mtime(&script, timestamp).unwrap();
        copy_skill(&source, &destination).unwrap();
        let copied = fs::metadata(destination.join("run.sh")).unwrap();
        assert_eq!(copied.permissions().mode() & 0o777, 0o751);
        assert_eq!(
            filetime::FileTime::from_last_modification_time(&copied),
            timestamp
        );
    }

    #[cfg(unix)]
    #[test]
    fn enabling_rolls_back_links_when_a_later_target_changes() {
        let (_temp, config) = fixture();
        skill(&config.root.join("foo"), "body");
        let plan = plan_skill_toggle(&config, "foo", true).unwrap();
        fs::remove_dir(&config.targets["codex"].path).unwrap();
        fs::write(&config.targets["codex"].path, "now a file").unwrap();

        assert!(plan.apply().is_err());
        assert!(!config.targets["claude"].path.join("foo").exists());
        assert!(config.root.join("foo").is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn disabling_restores_removed_links_when_a_later_target_changes() {
        let (_temp, config) = fixture();
        let canonical = config.root.join("foo");
        skill(&canonical, "body");
        let claude = config.targets["claude"].path.join("foo");
        let codex = config.targets["codex"].path.join("foo");
        std::os::unix::fs::symlink(&canonical, &claude).unwrap();
        std::os::unix::fs::symlink(&canonical, &codex).unwrap();
        let plan = plan_skill_toggle(&config, "foo", false).unwrap();
        fs::remove_file(&codex).unwrap();
        skill(&codex, "changed concurrently");

        assert!(plan.apply().is_err());
        assert!(claude.is_symlink());
        verify_link(&claude, &canonical).unwrap();
        assert!(codex.is_dir());
        assert!(canonical.is_dir());
    }
}
