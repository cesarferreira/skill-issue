use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub root: PathBuf,
    #[serde(default)]
    pub targets: BTreeMap<String, TargetConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TargetConfig {
    pub path: PathBuf,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

fn enabled_by_default() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallationKind {
    Physical,
    ManagedSymlink,
    ForeignSymlink,
    BrokenSymlink,
}

#[derive(Clone, Debug)]
pub struct Installation {
    pub target: String,
    pub path: PathBuf,
    pub kind: InstallationKind,
    pub fingerprint: Option<String>,
    pub link_target: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct CanonicalSkill {
    pub path: PathBuf,
    pub fingerprint: String,
}

#[derive(Clone, Debug)]
pub struct SkillGroup {
    pub name: String,
    pub canonical: Option<CanonicalSkill>,
    pub installations: Vec<Installation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillStatus {
    Unique,
    IdenticalDuplicate,
    Divergent,
    Managed,
    Broken,
}

#[derive(Debug)]
pub struct ScanResult {
    pub groups: BTreeMap<String, SkillGroup>,
    pub target_counts: BTreeMap<String, usize>,
    pub missing_targets: Vec<String>,
}
