use anyhow::{Context, Result, anyhow, bail};
use blake3::Hasher;
use console::{Style, style};
use dialoguer::{Confirm, Input, Select, theme::ColorfulTheme};
use similar::TextDiff;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, BufReader, IsTerminal, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use walkdir::{DirEntry, WalkDir};

mod cli;
mod model;

pub use cli::Cli;
use cli::{Command, ConfigCommand, LinkArgs, TargetCommand};
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

#[derive(Clone, Debug)]
struct AdoptionPlan {
    display_name: String,
    canonical: PathBuf,
    fingerprint: String,
    transfer: CanonicalTransfer,
    replacements: Vec<Replacement>,
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

pub fn run(cli: Cli) -> Result<u8> {
    set_color(cli.no_color);
    VERBOSITY.store(cli.verbose, Ordering::Relaxed);
    match cli.command {
        Some(Command::Init { root }) => init(root, cli.dry_run),
        Some(command) => {
            let config = match Config::load() {
                Ok(config) => config,
                Err(error) if is_missing_config(&error) => {
                    eprintln!("skillissue is not configured. Run `skillissue init`. ");
                    return Ok(EXIT_CONFIG);
                }
                Err(error) => {
                    eprintln!("Invalid configuration: {error:#}");
                    return Ok(EXIT_CONFIG);
                }
            };
            if let Err(error) = validate_config(&config) {
                eprintln!("Invalid configuration: {error:#}");
                return Ok(EXIT_CONFIG);
            }
            match command {
                Command::Scan => scan_command(&config),
                Command::Adopt { skill } => {
                    adopt_command(&config, skill.as_deref(), cli.dry_run, true)
                }
                Command::Status => status_command(&config),
                Command::Doctor { fix } => doctor_command(&config, fix, cli.dry_run),
                Command::Diff { skill, content } => diff_command(&config, &skill, content),
                Command::Link(args) => link_command(&config, args, cli.dry_run),
                Command::Unlink { skill, targets } => {
                    unlink_command(&config, &skill, &targets, cli.dry_run)
                }
                Command::Targets { command } => targets_command(config, command, cli.dry_run),
                Command::Config { command } => config_command(config, command, cli.dry_run),
                Command::Init { .. } => unreachable!(),
            }
        }
        None => default_command(cli.dry_run),
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
        if let Some(path) = std::env::var_os("SKILLISSUE_CONFIG") {
            return Ok(PathBuf::from(path));
        }
        let dir = dirs::config_dir()
            .ok_or_else(|| anyhow!("could not determine the configuration directory"))?;
        Ok(dir.join("skillissue/config.toml"))
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

fn validate_config(config: &Config) -> Result<()> {
    for (id, target) in config.targets.iter().filter(|(_, t)| t.enabled) {
        let root = fs::canonicalize(&config.root).unwrap_or_else(|_| config.root.clone());
        let target_path = fs::canonicalize(&target.path).unwrap_or_else(|_| target.path.clone());
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

fn init(root: Option<PathBuf>, dry_run: bool) -> Result<u8> {
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
    let config = Config {
        root: root.clone(),
        targets,
    };
    validate_config(&config)?;
    let config_path = Config::path()?;
    if config_path.exists() {
        let current = Config::load()?;
        if current.root == config.root {
            println!("skillissue is already configured with {}", root.display());
            return Ok(EXIT_OK);
        }
        bail!(
            "configuration already exists at {}; use `skillissue config set-root`",
            config_path.display()
        );
    }
    println!("{}", Style::new().bold().apply_to("INIT PLAN"));
    println!("CREATE  {}", root.display());
    println!("WRITE   {}", config_path.display());
    if dry_run {
        println!("Dry run; no files changed.");
        return Ok(EXIT_OK);
    }
    fs::create_dir_all(&root)
        .with_context(|| format!("create canonical root {}", root.display()))?;
    config.save()?;
    println!("{} Created {}", style("✓").green(), root.display());
    println!("{} Configuration saved", style("✓").green());
    for id in config.targets.keys() {
        println!("{} Detected {id}", style("✓").green());
    }
    Ok(EXIT_OK)
}

pub fn scan(config: &Config) -> Result<ScanResult> {
    let mut groups = BTreeMap::<String, SkillGroup>::new();
    let mut logical_names = BTreeMap::<String, String>::new();
    if config.root.exists() {
        for entry in sorted_children(&config.root)? {
            let kind = entry.file_type()?;
            let name = utf8_name(&entry.path())?;
            if ignored_top_level(&name) || (!kind.is_dir() && !kind.is_symlink()) {
                continue;
            }
            if kind.is_symlink() {
                bail!(
                    "canonical skill must be a real directory, not a symlink: {}",
                    entry.path().display()
                );
            }
            check_case_collision(&mut logical_names, &name, "configured skill locations")?;
            let fp = fingerprint(&entry.path())?;
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
            if ignored_top_level(&name) {
                continue;
            }
            check_case_collision(&mut logical_names, &name, "configured skill locations")?;
            let installation = inspect_installation(config, target_id, entry.path())?;
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
    name == ".git" || name == ".DS_Store" || name.starts_with(".skillissue-")
}

fn inspect_installation(config: &Config, target: &str, path: PathBuf) -> Result<Installation> {
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
            Some(fingerprint(&resolved)?)
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
        let fingerprint = Some(fingerprint(&path)?);
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
    if !root.is_dir() {
        bail!("not a readable skill directory: {}", root.display());
    }
    let mut records = Vec::<(String, u8, String)>::new();
    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(include_entry);
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

fn include_entry(entry: &DirEntry) -> bool {
    let name = entry.file_name();
    name != OsStr::new(".git") && name != OsStr::new(".DS_Store")
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

fn scan_command(config: &Config) -> Result<u8> {
    let result = scan(config)?;
    println!("Scanning skill locations...");
    for (target, count) in &result.target_counts {
        if VERBOSITY.load(Ordering::Relaxed) > 0 {
            println!(
                "  {target:<12} {count:>3}  {}",
                config.targets[target].path.display()
            );
        } else {
            println!("  {target:<12} {count:>3}");
        }
    }
    for target in &result.missing_targets {
        println!("  {target:<12} {} missing", style("⚠").yellow());
    }
    let installations: usize = result.target_counts.values().sum();
    println!(
        "\n{installations} installations\n{} unique skills",
        result.groups.len()
    );
    print_groups(&result);
    Ok(result_exit(&result))
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
        println!("\n{}", Style::new().bold().apply_to(heading));
        for group in matching {
            let targets = group
                .installations
                .iter()
                .map(|i| i.target.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            println!("{:<20} {}", group.name, targets);
            if status == SkillStatus::Divergent {
                if let Some(c) = &group.canonical {
                    println!("  canonical    {}", short_hash(&c.fingerprint));
                }
                for installation in &group.installations {
                    println!(
                        "  {:<12} {}",
                        installation.target,
                        installation
                            .fingerprint
                            .as_deref()
                            .map(short_hash)
                            .unwrap_or("broken")
                    );
                }
            }
            if VERBOSITY.load(Ordering::Relaxed) > 1 && status != SkillStatus::Divergent {
                if let Some(c) = &group.canonical {
                    println!("  canonical    {}", short_hash(&c.fingerprint));
                }
                for installation in &group.installations {
                    println!(
                        "  {:<12} {}",
                        installation.target,
                        installation
                            .fingerprint
                            .as_deref()
                            .map(short_hash)
                            .unwrap_or("unreadable")
                    );
                }
            }
        }
    }
}

fn short_hash(hash: &str) -> &str {
    &hash[..hash.len().min(7)]
}

fn result_exit(result: &ScanResult) -> u8 {
    if result
        .groups
        .values()
        .any(|g| g.status() == SkillStatus::Divergent)
    {
        EXIT_CONFLICTS
    } else if !result.missing_targets.is_empty()
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

fn status_command(config: &Config) -> Result<u8> {
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
    println!("{}", Style::new().bold().apply_to("Canonical directory"));
    println!("  {}", config.root.display());
    println!(
        "{canonical} skills\n{} targets\n{installations} installations\n{managed} healthy symlinks",
        config.targets.values().filter(|t| t.enabled).count()
    );
    let code = result_exit(&result);
    if code == EXIT_OK {
        println!("{} No skill issues.", style("✓").green());
    } else {
        println!("\n{}", Style::new().bold().apply_to("Issues"));
        for target in &result.missing_targets {
            println!(
                "{} {target} target directory is missing",
                style("⚠").yellow()
            );
        }
        for group in result
            .groups
            .values()
            .filter(|g| g.status() != SkillStatus::Managed)
        {
            println!(
                "{} {} ({:?})",
                issue_mark(group.status()),
                group.name,
                group.status()
            );
            for installation in &group.installations {
                println!(
                    "  {}: {} ({:?})",
                    installation.target,
                    installation.path.display(),
                    installation.kind
                );
            }
        }
        println!("Run: skillissue doctor");
    }
    Ok(code)
}

fn issue_mark(status: SkillStatus) -> console::StyledObject<&'static str> {
    if status == SkillStatus::Divergent {
        style("✗").red()
    } else {
        style("⚠").yellow()
    }
}

fn default_command(dry_run: bool) -> Result<u8> {
    let config = match Config::load() {
        Ok(config) => config,
        Err(error) if is_missing_config(&error) => {
            eprintln!("skillissue is not configured. Run `skillissue init`. ");
            return Ok(EXIT_CONFIG);
        }
        Err(error) => {
            eprintln!("Invalid configuration: {error:#}");
            return Ok(EXIT_CONFIG);
        }
    };
    validate_config(&config)?;
    let result = scan(&config)?;
    if result_exit(&result) == EXIT_OK {
        return status_command(&config);
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return status_command(&config);
    }
    println!("Found skill issues. Preparing a safe migration plan...\n");
    adopt_from_scan(&config, &result, None, dry_run, true)
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
    let mut plans = Vec::new();
    for group in selected {
        plans.extend(plans_for_group(config, group, tty)?);
    }
    if plans.is_empty() {
        println!("No adoptable skills found.");
        return Ok(result_exit(result));
    }
    render_adoption_plans(&plans);
    if dry_run {
        println!("Dry run; no files changed.");
        return Ok(result_exit(result));
    }
    if !tty {
        bail!("adoption requires an interactive confirmation; use --dry-run to inspect the plan");
    }
    if !Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("Proceed?")
        .default(false)
        .interact()?
    {
        println!("Cancelled. No files were changed.");
        return Ok(result_exit(result));
    }
    for plan in &plans {
        execute_adoption(plan)?;
        println!("{} {} adopted", style("✓").green(), plan.display_name);
    }
    let after = scan(config)?;
    if result_exit(&after) == EXIT_OK {
        println!("{} No skill issues.", style("✓").green());
    }
    Ok(result_exit(&after))
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
            return Ok(plan_existing(&group.name, canonical, matching)
                .into_iter()
                .collect());
        }
        if !tty {
            bail!("{} has divergent copies", group.name);
        }
        let options = [
            "Use the existing canonical version where copies match",
            "Keep all physical versions under separate names",
            "View content diff",
            "Skip",
        ];
        match Select::with_theme(&ColorfulTheme::default())
            .with_prompt(format!("{} has divergent copies", group.name))
            .items(options)
            .default(3)
            .interact()?
        {
            0 => Ok(plan_existing(&group.name, canonical, matching)
                .into_iter()
                .collect()),
            1 => plans_keep_both(config, group, &physical_by_fp, Some(&canonical.fingerprint)),
            2 => {
                diff_group(group, true)?;
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
                            .map(|i| i.target.as_str())
                    })
                    .collect::<Vec<_>>()
                    .join(" / ");
                format!("Use {targets} ({})", short_hash(fp))
            })
            .collect();
        labels.push("Keep all versions under separate names".to_string());
        labels.push("View content diff".to_string());
        labels.push("Skip".to_string());
        let choice = Select::with_theme(&ColorfulTheme::default())
            .with_prompt(format!("{} has divergent copies", group.name))
            .items(&labels)
            .default(labels.len() - 1)
            .interact()?;
        if choice < versions.len() {
            let (fingerprint, paths) = versions[choice];
            Ok(vec![plan_new(
                config,
                &group.name,
                &group.name,
                (*fingerprint).clone(),
                (*paths).clone(),
            )?])
        } else if choice == versions.len() {
            plans_keep_both(config, group, &physical_by_fp, None)
        } else if choice == versions.len() + 1 {
            diff_group(group, true)?;
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

fn plan_existing(
    name: &str,
    canonical: &CanonicalSkill,
    installations: Vec<PathBuf>,
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
    if canonical_skill_names(&config.root)?
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
            if let Some(plan) = plan_existing(&group.name, canonical, paths.clone()) {
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

fn render_adoption_plans(plans: &[AdoptionPlan]) {
    println!("{}", Style::new().bold().apply_to("PLAN"));
    for plan in plans {
        match &plan.transfer {
            CanonicalTransfer::Existing => {}
            CanonicalTransfer::Move { source } => {
                println!(
                    "MOVE\n  {}\n    -> {}",
                    source.display(),
                    plan.canonical.display()
                );
            }
            CanonicalTransfer::Copy { source, temporary } => {
                println!(
                    "COPY\n  {}\n    -> {}",
                    source.display(),
                    temporary.display()
                );
                println!("VERIFY  {}", temporary.display());
                println!(
                    "MOVE\n  {}\n    -> {}",
                    temporary.display(),
                    plan.canonical.display()
                );
            }
        }
        for replacement in &plan.replacements {
            if let Some(backup) = &replacement.backup {
                println!(
                    "STAGE\n  {}\n    -> {}",
                    replacement.original.display(),
                    backup.display()
                );
            }
            println!(
                "LINK\n  {}\n    -> {}",
                replacement.original.display(),
                plan.canonical.display()
            );
            println!("VERIFY  {}", replacement.original.display());
            if let Some(backup) = &replacement.backup {
                println!("REMOVE  {}", backup.display());
            }
        }
    }
    println!(
        "{} locations affected.",
        plans.iter().map(|p| p.replacements.len()).sum::<usize>()
    );
}

fn execute_adoption(plan: &AdoptionPlan) -> Result<()> {
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
                let actual = fingerprint(temporary)?;
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
    if fingerprint(&plan.canonical)? != plan.fingerprint {
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
            create_symlink(&plan.canonical, &replacement.original)?;
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
    Ok(())
}

fn unique_sibling(path: &Path, purpose: &str) -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .unwrap_or_else(|| OsStr::new("skill"))
        .to_string_lossy();
    path.with_file_name(format!(
        ".skillissue-{purpose}-{name}-{}-{sequence}",
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
        } else if entry.file_type().is_symlink() {
            create_symlink(&fs::read_link(entry.path())?, &output)?;
        } else {
            bail!("unsupported special file: {}", entry.path().display());
        }
    }
    for (input, output) in directories.into_iter().rev() {
        fs::set_permissions(output, fs::metadata(input)?.permissions())?;
    }
    Ok(())
}

#[cfg(unix)]
fn create_symlink(target: &Path, link: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, link)
        .with_context(|| format!("link {} -> {}", link.display(), target.display()))
}

#[cfg(not(unix))]
fn create_symlink(_target: &Path, _link: &Path) -> Result<()> {
    bail!("symlink mutations are supported only on macOS and Linux in V0.1")
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
    println!("Checking skillissue...");
    let root_ok = config.root.is_dir();
    println!(
        "{} Canonical directory {}",
        if root_ok {
            style("✓").green()
        } else {
            style("✗").red()
        },
        if root_ok { "exists" } else { "is missing" }
    );
    let result = scan(config)?;
    for (target, target_config) in config.targets.iter().filter(|(_, t)| t.enabled) {
        println!(
            "{} {target} target {}",
            if target_config.path.is_dir() {
                style("✓").green()
            } else {
                style("⚠").yellow()
            },
            if target_config.path.is_dir() {
                "healthy"
            } else {
                "missing"
            }
        );
    }
    let recovery = recovery_artifacts(config)?;
    let mut issue_count = usize::from(!root_ok) + result.missing_targets.len() + recovery.len();
    for path in &recovery {
        println!(
            "{} Recovery artifact from an interrupted migration: {}",
            style("⚠").yellow(),
            path.display()
        );
    }
    for group in result.groups.values() {
        for installation in &group.installations {
            match installation.kind {
                InstallationKind::ManagedSymlink => {}
                InstallationKind::Physical if group.canonical.is_none() => {
                    issue_count += 1;
                    println!(
                        "{}. {} is not adopted",
                        issue_count,
                        installation.path.display()
                    );
                }
                InstallationKind::Physical => {
                    issue_count += 1;
                    println!(
                        "{}. {} is a physical copy beside a canonical skill",
                        issue_count,
                        installation.path.display()
                    );
                }
                InstallationKind::ForeignSymlink => {
                    issue_count += 1;
                    println!(
                        "{}. {} is a foreign symlink",
                        issue_count,
                        installation.path.display()
                    );
                }
                InstallationKind::BrokenSymlink => {
                    issue_count += 1;
                    println!(
                        "{}. {} is a broken symlink",
                        issue_count,
                        installation.path.display()
                    );
                }
            }
        }
    }
    if issue_count == 0 {
        println!("{} No skill issues.", style("✓").green());
        return Ok(EXIT_OK);
    }
    println!("{} {issue_count} issues found", style("⚠").yellow());
    if !fix {
        println!("Run: skillissue doctor --fix");
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
            if let Some(plan) = plan_existing(&group.name, canonical, matching) {
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
        println!("{}", Style::new().bold().apply_to("PLAN"));
    }
    for (link, target, _) in &link_repairs {
        println!(
            "RECREATE LINK\n  {}\n    -> {}",
            link.display(),
            target.display()
        );
    }
    if dry_run {
        println!("Dry run; no files changed.");
        return Ok(result_exit(&result));
    }
    require_confirmation("Apply these unambiguous repairs?")?;
    for plan in &adoptions {
        execute_adoption(plan)?;
    }
    for (link, target, old_target) in &link_repairs {
        fs::remove_file(link)?;
        if let Err(error) = create_symlink(target, link).and_then(|_| verify_link(link, target)) {
            if fs::symlink_metadata(link).is_ok() {
                let _ = fs::remove_file(link);
            }
            let _ = create_symlink(old_target, link);
            return Err(error).context("repair broken link");
        }
    }
    let after = scan(config)?;
    if result_exit(&after) == EXIT_OK {
        println!("{} No skill issues.", style("✓").green());
    }
    Ok(result_exit(&after))
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
            if name.to_string_lossy().starts_with(".skillissue-") {
                artifacts.push(entry.path());
            }
        }
    }
    Ok(artifacts)
}

fn require_confirmation(prompt: &str) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("this mutation requires an interactive confirmation; use --dry-run to inspect it");
    }
    if !Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt)
        .default(false)
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

fn directory_manifest(root: &Path) -> Result<BTreeMap<String, ManifestEntry>> {
    let mut manifest = BTreeMap::new();
    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(include_entry);
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
    diff_group(group, content)
}

fn diff_group(group: &SkillGroup, content: bool) -> Result<u8> {
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
    println!("{} ↔ {}", left.0, right.0);
    let left_manifest = directory_manifest(&left.1)?;
    let right_manifest = directory_manifest(&right.1)?;
    let names: BTreeSet<_> = left_manifest
        .keys()
        .chain(right_manifest.keys())
        .cloned()
        .collect();
    let mut changes = 0;
    for name in names {
        match (left_manifest.get(&name), right_manifest.get(&name)) {
            (None, Some(_)) => {
                println!("A {name}");
                changes += 1;
            }
            (Some(_), None) => {
                println!("D {name}");
                changes += 1;
            }
            (Some(a), Some(b)) if a.kind != b.kind || a.hash != b.hash => {
                println!("M {name}");
                changes += 1;
                if content && a.kind == b'F' && b.kind == b'F' {
                    print_content_diff(&name, a, b)?;
                }
            }
            _ => {}
        }
    }
    if changes == 0 {
        println!("{} Copies are identical.", style("✓").green());
        Ok(EXIT_OK)
    } else {
        Ok(EXIT_CONFLICTS)
    }
}

fn print_content_diff(name: &str, left: &ManifestEntry, right: &ManifestEntry) -> Result<()> {
    let left_bytes = fs::read(&left.source)?;
    let right_bytes = fs::read(&right.source)?;
    match (
        std::str::from_utf8(&left_bytes),
        std::str::from_utf8(&right_bytes),
    ) {
        (Ok(a), Ok(b)) => print!(
            "{}",
            TextDiff::from_lines(a, b)
                .unified_diff()
                .header(&format!("a/{name}"), &format!("b/{name}"))
        ),
        _ => println!("  (binary content differs)"),
    }
    Ok(())
}

fn link_command(config: &Config, args: LinkArgs, dry_run: bool) -> Result<u8> {
    let target_ids = if args.all {
        args.target
    } else {
        let mut targets = args.targets;
        targets.extend(args.target);
        targets
    };
    if target_ids.is_empty() {
        bail!("at least one target is required");
    }
    let skills = if args.all {
        canonical_skill_names(&config.root)?
    } else {
        vec![
            args.skill
                .ok_or_else(|| anyhow!("a skill is required unless --all is used"))?,
        ]
    };
    let mut links = Vec::new();
    let mut create_targets = BTreeSet::new();
    for target_id in target_ids {
        let target = enabled_target(config, &target_id)?;
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
        println!("All requested links already exist.");
        return Ok(EXIT_OK);
    }
    println!("{}", Style::new().bold().apply_to("PLAN"));
    for path in &create_targets {
        println!("CREATE TARGET  {}", path.display());
    }
    for (link, target) in &links {
        println!("LINK\n  {}\n    -> {}", link.display(), target.display());
    }
    if dry_run {
        println!("Dry run; no files changed.");
        return Ok(EXIT_OK);
    }
    require_confirmation("Create these links?")?;
    for path in create_targets {
        fs::create_dir_all(path)?;
    }
    let mut created = Vec::new();
    for (link, target) in &links {
        if let Err(error) = create_symlink(target, link).and_then(|_| verify_link(link, target)) {
            for path in &created {
                let _ = fs::remove_file(path);
            }
            return Err(error).context("link operation rolled back");
        }
        created.push(link.clone());
    }
    for (link, _) in links {
        println!("{} {}", style("✓").green(), link.display());
    }
    Ok(EXIT_OK)
}

fn canonical_skill_names(root: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in sorted_children(root)? {
        let metadata = entry.file_type()?;
        let name = utf8_name(&entry.path())?;
        if metadata.is_dir() && !metadata.is_symlink() && !ignored_top_level(&name) {
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

fn unlink_command(config: &Config, skill: &str, targets: &[String], dry_run: bool) -> Result<u8> {
    if targets.is_empty() {
        bail!("at least one target is required");
    }
    let mut links = Vec::new();
    for id in targets {
        let path = enabled_target(config, id)?.path.join(skill);
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
    println!("{}", Style::new().bold().apply_to("PLAN"));
    for path in &links {
        println!("UNLINK  {}", path.display());
    }
    println!(
        "Canonical skill remains: {}",
        config.root.join(skill).display()
    );
    if dry_run {
        println!("Dry run; no files changed.");
        return Ok(EXIT_OK);
    }
    require_confirmation("Remove these links?")?;
    for path in links {
        fs::remove_file(&path)?;
        println!("{} Removed {}", style("✓").green(), path.display());
    }
    Ok(EXIT_OK)
}

fn targets_command(
    mut config: Config,
    command: Option<TargetCommand>,
    dry_run: bool,
) -> Result<u8> {
    match command {
        None => {
            println!("TARGET       PATH                                      STATUS");
            for (id, target) in &config.targets {
                let status = if !target.enabled {
                    "disabled"
                } else if target.path.is_dir() {
                    "✓"
                } else {
                    "missing"
                };
                println!("{id:<12} {:<41} {status}", target.path.display());
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
            println!("ADD TARGET {id}  {}", path.display());
            if !dry_run {
                config.save()?;
                println!("{} Target added", style("✓").green());
            } else {
                println!("Dry run; no files changed.");
            }
        }
        Some(TargetCommand::Remove { id }) => {
            if config.targets.remove(&id).is_none() {
                bail!("unknown target `{id}`");
            }
            println!("REMOVE TARGET {id}\nFilesystem data will not be removed.");
            if !dry_run {
                config.save()?;
                println!("{} Target removed from configuration", style("✓").green());
            } else {
                println!("Dry run; no files changed.");
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
        None => print!("{}", toml::to_string_pretty(&config)?),
        Some(ConfigCommand::SetRoot { root }) => {
            let new_root = absolute_path(&root)?;
            if new_root == config.root {
                println!("Canonical root is already {}", new_root.display());
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
                "SET ROOT\n  {}\n    -> {}",
                old.display(),
                new_root.display()
            );
            if !dry_run {
                fs::create_dir_all(&new_root)?;
                config.save()?;
                println!("{} Canonical root updated", style("✓").green());
            } else {
                println!("Dry run; no files changed.");
            }
        }
    }
    Ok(EXIT_OK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
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
        (temp, Config { root, targets })
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

    #[test]
    fn clap_accepts_the_documented_link_forms() {
        let direct = Cli::try_parse_from(["skillissue", "link", "foo", "claude", "codex"]).unwrap();
        assert!(matches!(
            direct.command,
            Some(Command::Link(LinkArgs { all: false, .. }))
        ));
        let all =
            Cli::try_parse_from(["skillissue", "link", "--all", "--target", "claude"]).unwrap();
        assert!(matches!(
            all.command,
            Some(Command::Link(LinkArgs { all: true, .. }))
        ));
    }
}
