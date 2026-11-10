use super::SkillInfo;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
pub const MAX_SOURCE: usize = 64 * 1024;
pub const MAX_TOTAL: usize = 256 * 1024;
pub struct Skill {
    pub info: SkillInfo,
    pub path: PathBuf,
    pub root: PathBuf,
}

pub fn valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}

pub fn read_scoped(path: &Path, root: &Path, max: usize) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "context source escapes its directory".to_string())?;
    let dir = cap_std::fs::Dir::open_ambient_dir(root, cap_std::ambient_authority())
        .map_err(|e| e.to_string())?;
    let file = crate::tools::files::open_regular_file(&dir, relative).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > max {
        return Err("context source exceeds byte limit".into());
    }
    String::from_utf8(bytes).map_err(|_| "context source must be UTF-8".into())
}

fn chain(cwd: &Path) -> Result<Vec<PathBuf>, String> {
    let cwd = cwd.canonicalize().map_err(|e| e.to_string())?;
    let mut chain = Vec::new();
    for path in cwd.ancestors().take(32) {
        chain.push(path.to_path_buf());
        if path.join(".git").exists() {
            break;
        }
    }
    chain.reverse();
    Ok(chain)
}

pub fn instructions(cwd: &Path) -> Result<String, String> {
    let mut out = String::new();
    for directory in chain(cwd)? {
        for name in ["AGENTS.md", "CLAUDE.md"] {
            let path = directory.join(name);
            match fs::symlink_metadata(&path) {
                Ok(_) => {
                    let text = read_scoped(&path, &directory, MAX_SOURCE)?;
                    if out.len() + text.len() > MAX_TOTAL {
                        return Err("instruction chain exceeds byte limit".into());
                    }
                    out.push_str(&format!(
                        "\nInstructions from {}:\n{text}\n",
                        path.display()
                    ));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
        }
    }
    Ok(out)
}

pub fn discover_skills(cwd: &Path) -> Result<Vec<Skill>, String> {
    let mut found = std::collections::BTreeMap::new();
    let directories = chain(cwd)?;
    // A temporary/non-repository workspace must not discover incidental home skills.
    let directories = if directories.first().is_some_and(|p| p.join(".git").exists()) {
        directories
    } else {
        vec![cwd.canonicalize().map_err(|e| e.to_string())?]
    };
    for directory in directories {
        for relative in [".claude/skills", ".agents/skills"] {
            let path = directory.join(relative);
            let entries = match fs::read_dir(&path) {
                Ok(e) => e,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.to_string()),
            };
            let canonical = path.canonicalize().map_err(|e| e.to_string())?;
            if !canonical.starts_with(&directory) {
                return Err("skill directory escapes project".into());
            }
            for entry in entries.take(257) {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name().to_string_lossy().to_string();
                if !valid_skill_name(&name) {
                    continue;
                }
                let skill_path = entry.path().join("SKILL.md");
                if !skill_path.exists() {
                    continue;
                }
                if found.len() >= 256 {
                    return Err("skill catalogue exceeds 256 entries".into());
                }
                let body = read_scoped(&skill_path, &directory, MAX_SOURCE)?;
                let header = body
                    .strip_prefix("---\n")
                    .or_else(|| body.strip_prefix("---\r\n"))
                    .and_then(|b| b.split("\n---").next())
                    .unwrap_or("");
                let mut lines = header.lines();
                let first = lines
                    .find_map(|line| line.strip_prefix("description:"))
                    .unwrap_or("")
                    .trim();
                let description = if first.starts_with(['>', '|']) {
                    lines
                        .take_while(|line| line.starts_with([' ', '\t']) || line.is_empty())
                        .map(str::trim)
                        .collect::<Vec<_>>()
                        .join(" ")
                } else {
                    first.trim_matches(['\'', '"']).to_owned()
                };
                let description = description.chars().take(512).collect();
                found.insert(
                    name.clone(),
                    Skill {
                        info: SkillInfo { name, description },
                        path: skill_path,
                        root: directory.clone(),
                    },
                );
            }
        }
    }
    Ok(found.into_values().collect())
}
#[cfg(test)]
mod security_tests {
    use super::*;
    #[test]
    fn scoped_source_rejects_directories_and_escaped_paths() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), "outside").unwrap();
        assert!(read_scoped(root.path(), root.path(), MAX_SOURCE).is_err());
        assert!(read_scoped(&outside.path().join("secret"), root.path(), MAX_SOURCE).is_err());
        fs::write(root.path().join("good"), "inside").unwrap();
        assert_eq!(
            read_scoped(&root.path().join("good"), root.path(), MAX_SOURCE).unwrap(),
            "inside"
        );
    }
    #[cfg(windows)]
    #[test]
    fn scoped_source_rejects_windows_devices() {
        let root = tempfile::tempdir().unwrap();
        assert!(read_scoped(&root.path().join("NUL"), root.path(), MAX_SOURCE).is_err());
    }
}
