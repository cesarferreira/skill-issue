use crate::{
    Config, EXIT_ISSUES, EXIT_OK, canonical_skill_names, create_managed_symlink,
    fingerprint_with_ignores, managed_link_value, require_confirmation_with_default,
    resolve_link_path, theme, unique_sibling, verify_link,
};
use anyhow::{Context, Result};
use std::{collections::BTreeSet, fs, io, path::PathBuf};

#[derive(Debug)]
struct ApplyPlan {
    create_targets: BTreeSet<PathBuf>,
    create_links: Vec<(PathBuf, PathBuf)>,
    replace_identical: Vec<(PathBuf, PathBuf, PathBuf)>,
    remove_stale_links: Vec<(PathBuf, PathBuf)>,
    conflicts: Vec<(PathBuf, String)>,
}

pub(crate) fn run(config: &Config, dry_run: bool) -> Result<u8> {
    let plan = build_plan(config)?;
    render_plan(&plan, config);
    if !plan.conflicts.is_empty() {
        println!("Run: {}", theme::hint("si setup"));
        return Ok(EXIT_ISSUES);
    }
    if dry_run {
        println!("{}", theme::dim("Dry run; no files changed."));
        return Ok(EXIT_OK);
    }
    if plan.create_links.is_empty()
        && plan.replace_identical.is_empty()
        && plan.remove_stale_links.is_empty()
    {
        println!("{}", theme::dim("Everything is already applied."));
        return Ok(EXIT_OK);
    }
    require_confirmation_with_default("Apply this plan?", true)?;
    execute_plan(&plan, config)?;
    let created_link_count = plan.create_links.len() + plan.replace_identical.len();
    if created_link_count > 0 {
        println!(
            "{} {} link{} created",
            theme::ok(),
            theme::good_count(created_link_count),
            if created_link_count == 1 { "" } else { "s" }
        );
    }
    if !plan.remove_stale_links.is_empty() {
        println!(
            "{} {} stale link{} removed",
            theme::ok(),
            theme::good_count(plan.remove_stale_links.len()),
            if plan.remove_stale_links.len() == 1 {
                ""
            } else {
                "s"
            }
        );
    }
    Ok(EXIT_OK)
}

fn build_plan(config: &Config) -> Result<ApplyPlan> {
    let mut plan = ApplyPlan {
        create_targets: BTreeSet::new(),
        create_links: Vec::new(),
        replace_identical: Vec::new(),
        remove_stale_links: Vec::new(),
        conflicts: Vec::new(),
    };
    let skills = canonical_skill_names(&config.root)?;
    for target in config.targets.values().filter(|target| target.enabled) {
        if !target.path.exists() {
            plan.create_targets.insert(target.path.clone());
        }
        for skill in &skills {
            let canonical = config.root.join(skill);
            let link = target.path.join(skill);
            match fs::symlink_metadata(&link) {
                Ok(metadata)
                    if metadata.file_type().is_symlink()
                        && verify_link(&link, &canonical).is_ok() => {}
                Ok(metadata) if metadata.file_type().is_symlink() => plan.conflicts.push((
                    link,
                    "is a foreign or broken symlink; it was left unchanged".to_string(),
                )),
                Ok(metadata) if metadata.is_dir() => {
                    if fingerprint_with_ignores(&link, &config.ignore)?
                        == fingerprint_with_ignores(&canonical, &config.ignore)?
                    {
                        plan.replace_identical.push((
                            link.clone(),
                            canonical,
                            unique_sibling(&link, "apply-backup"),
                        ));
                    } else {
                        plan.conflicts.push((
                            link,
                            "is a divergent physical directory; it was left unchanged".to_string(),
                        ));
                    }
                }
                Ok(_) => plan.conflicts.push((
                    link,
                    "is a physical file; it was left unchanged".to_string(),
                )),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    plan.create_links.push((link, canonical));
                }
                Err(error) => return Err(error.into()),
            }
        }
        if !target.path.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&target.path)? {
            let entry = entry?;
            let link = entry.path();
            let metadata = fs::symlink_metadata(&link)?;
            if !metadata.file_type().is_symlink() {
                continue;
            }
            let raw_target = fs::read_link(&link)?;
            let resolved = resolve_link_path(&link, &raw_target);
            if resolved.parent() == Some(config.root.as_path()) && !resolved.exists() {
                plan.remove_stale_links.push((link, raw_target));
            }
        }
    }
    Ok(plan)
}

fn render_plan(plan: &ApplyPlan, config: &Config) {
    println!("{}", theme::heading("APPLY PLAN"));
    for target in &plan.create_targets {
        println!(
            "{}  {}",
            theme::action("CREATE TARGET"),
            theme::path(target)
        );
    }
    for (link, canonical) in &plan.create_links {
        println!(
            "{}\n  {}\n    {} {}",
            theme::action("LINK"),
            theme::path(link),
            theme::arrow(),
            theme::path(&managed_link_value(canonical, link, config.relative_links))
        );
    }
    for (link, canonical, _) in &plan.replace_identical {
        println!(
            "{}\n  {}\n    {} {}",
            theme::action("REPLACE WITH LINK"),
            theme::path(link),
            theme::arrow(),
            theme::path(&managed_link_value(canonical, link, config.relative_links))
        );
    }
    for (link, _) in &plan.remove_stale_links {
        println!("{}  {}", theme::action("UNLINK STALE"), theme::path(link));
    }
    for (path, reason) in &plan.conflicts {
        println!("{} {} {reason}", theme::warn(), theme::path(path));
    }
}

fn execute_plan(plan: &ApplyPlan, config: &Config) -> Result<()> {
    let mut created_targets = Vec::new();
    let mut created_links = Vec::new();
    let mut removed_links = Vec::new();
    let mut replaced_links = Vec::new();
    let result = (|| -> Result<()> {
        for target in &plan.create_targets {
            fs::create_dir_all(target)?;
            created_targets.push(target.clone());
        }
        for (link, raw_target) in &plan.remove_stale_links {
            fs::remove_file(link)?;
            removed_links.push((link.clone(), raw_target.clone()));
        }
        for (link, canonical, backup) in &plan.replace_identical {
            fs::rename(link, backup)?;
            if let Err(error) = create_managed_symlink(canonical, link, config.relative_links)
                .and_then(|_| verify_link(link, canonical))
            {
                let _ = fs::rename(backup, link);
                return Err(error);
            }
            replaced_links.push((link.clone(), backup.clone()));
        }
        for (link, canonical) in &plan.create_links {
            create_managed_symlink(canonical, link, config.relative_links)?;
            verify_link(link, canonical)?;
            created_links.push(link.clone());
        }
        Ok(())
    })();
    if let Err(error) = result {
        for link in created_links.iter().rev() {
            let _ = fs::remove_file(link);
        }
        for (link, backup) in replaced_links.iter().rev() {
            let _ = fs::remove_file(link);
            let _ = fs::rename(backup, link);
        }
        for (link, raw_target) in removed_links.iter().rev() {
            let _ = crate::create_symlink(raw_target, link);
        }
        for target in created_targets.iter().rev() {
            let _ = fs::remove_dir(target);
        }
        return Err(error).context("apply operation rolled back");
    }
    for (_, backup) in replaced_links {
        fs::remove_dir_all(backup)?;
    }
    Ok(())
}
