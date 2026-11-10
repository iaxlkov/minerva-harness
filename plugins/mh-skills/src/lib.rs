//! Scoped skill discovery and immutable revisions. Roots are host capabilities.
use cap_std::fs::Dir;
use minerva_plugin_sdk::{
    learning::{AccessContext, LearningError, Scope},
    skills::{Invocation, LoadSkill, LoadedSkill, SkillCatalog, SkillInfo},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const MAX_SOURCE: usize = 64 * 1024;
const MAX_SKILLS: usize = 256;
type Result<T> = std::result::Result<T, LearningError>;
fn err(code: &str, message: impl ToString) -> LearningError {
    LearningError::new(code, message.to_string())
}
fn storage(e: impl ToString) -> LearningError {
    err("STORAGE", e)
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn scope_key(host: &AccessContext, scope: &Scope) -> Result<String> {
    serde_json::to_string(&(host.actor_id.as_str(), scope)).map_err(storage)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    Project,
    Custom,
    User,
    Bundled,
}

/// Only the host may construct and register this config. `root` is an explicit read capability.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub id: String,
    pub scope: Scope,
    pub layer: Layer,
    pub order: u32,
    pub root: PathBuf,
}
struct Provider {
    config: ProviderConfig,
    owner: String,
    dir: Dir,
}

/// Adapter to the existing context database; no second selected-skills store is created.
pub trait SelectionStore {
    fn select(&self, session: &str, name: &str) -> std::result::Result<(), String>;
    fn selected(&self, session: &str) -> std::result::Result<Vec<String>, String>;
}

pub struct SkillsService {
    db: Mutex<Connection>,
    providers: Mutex<Vec<Arc<Provider>>>,
}
impl cordis::Service for SkillsService {
    const NAME: &'static str = "minerva.skills";
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
struct Header {
    #[serde(default)]
    description: String,
    #[serde(default)]
    disable_model_invocation: bool,
    #[serde(default = "yes")]
    user_invocable: bool,
}
fn yes() -> bool {
    true
}
fn header(body: &str) -> Result<Header> {
    let rest = body
        .strip_prefix("---\n")
        .or_else(|| body.strip_prefix("---\r\n"));
    let Some(rest) = rest else {
        return Ok(Header {
            user_invocable: true,
            ..Header::default()
        });
    };
    let end = rest
        .lines()
        .scan(0usize, |pos, line| {
            let start = *pos;
            *pos += line.len() + 1;
            Some((start, line))
        })
        .find(|(_, line)| line.trim_end_matches('\r') == "---")
        .map(|(pos, _)| pos)
        .ok_or_else(|| err("INVALID_SKILL", "unclosed front matter"))?;
    serde_yaml::from_str(&rest[..end])
        .map_err(|e| err("INVALID_SKILL", format!("invalid front matter: {e}")))
}
fn read_skill(dir: &Dir, name: &str) -> Result<String> {
    if !valid_name(name) {
        return Err(err("INVALID_INPUT", "invalid skill name"));
    }
    let path = Path::new(name).join("SKILL.md");
    let file = mh_tool_runtime::files::open_regular_file(dir, &path).map_err(|e| {
        err(
            if e.kind() == std::io::ErrorKind::NotFound {
                "NOT_FOUND"
            } else {
                "READ_ERROR"
            },
            e,
        )
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_SOURCE as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| err("READ_ERROR", e))?;
    if bytes.len() > MAX_SOURCE {
        return Err(err("LIMIT", "skill exceeds byte limit"));
    }
    String::from_utf8(bytes).map_err(|_| err("INVALID_SKILL", "skill must be UTF-8"))
}
fn make_info(name: &str, provider: &str, body: &str, path: Option<String>) -> Result<SkillInfo> {
    let h = header(body)?;
    let revision = digest(format!("{provider}\0{name}\0{body}").as_bytes());
    Ok(SkillInfo {
        name: name.into(),
        description: h.description.chars().take(512).collect(),
        provider_id: provider.into(),
        revision,
        instruction_path: path,
        model_invocable: !h.disable_model_invocation,
        user_invocable: h.user_invocable,
    })
}
/// Validate a managed skill without publishing it.
pub fn validate_candidate(name: &str, body: &str) -> Result<SkillInfo> {
    if !valid_name(name) || body.len() > MAX_SOURCE {
        return Err(err("INVALID_INPUT", "invalid skill name or size"));
    }
    make_info(name, "managed", body, None)
}
fn check_invocation(info: &SkillInfo, invocation: Invocation) -> Result<()> {
    if match invocation {
        Invocation::Model => !info.model_invocable,
        Invocation::User => !info.user_invocable,
        Invocation::Selected => false,
    } {
        return Err(err("FORBIDDEN", "skill does not permit this invocation"));
    }
    Ok(())
}
impl SkillsService {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(storage)?;
        }
        let db = Connection::open(path).map_err(storage)?;
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(storage)?;
        let version: i64 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(storage)?;
        if version > 1 {
            return Err(err(
                "SCHEMA_VERSION",
                "skills database is newer than this service",
            ));
        }
        if version == 0 {
            let tables: i64 = db.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'", [], |r| r.get(0)).map_err(storage)?;
            if tables != 0 {
                return Err(err("SCHEMA_VERSION", "unversioned database is not empty"));
            }
        }
        db.execute_batch("PRAGMA journal_mode=WAL; BEGIN IMMEDIATE;
          CREATE TABLE IF NOT EXISTS skill_revisions(scope TEXT NOT NULL,name TEXT NOT NULL,revision TEXT NOT NULL,info TEXT NOT NULL,body TEXT NOT NULL,requires_user INTEGER NOT NULL,PRIMARY KEY(scope,name,revision));
          CREATE TABLE IF NOT EXISTS skill_current(scope TEXT NOT NULL,name TEXT NOT NULL,revision TEXT NOT NULL,PRIMARY KEY(scope,name));
          CREATE TABLE IF NOT EXISTS skill_publication_audit(request_id TEXT PRIMARY KEY, actor TEXT NOT NULL, scope TEXT NOT NULL, operation TEXT NOT NULL, name TEXT NOT NULL, base_revision TEXT, result_revision TEXT NOT NULL, payload_digest TEXT NOT NULL);
          PRAGMA user_version=1; COMMIT;").map_err(storage)?;
        Ok(Self {
            db: Mutex::new(db),
            providers: Mutex::new(Vec::new()),
        })
    }
    pub fn register_provider(&self, host: &AccessContext, config: ProviderConfig) -> Result<()> {
        host.authorize(&config.scope)?;
        if (!valid_name(&config.id) || config.id == "managed") || !config.root.is_absolute() {
            return Err(err("INVALID_INPUT", "provider id or root invalid"));
        }
        if config.layer == Layer::User && config.scope != Scope::User {
            return Err(err("INVALID_INPUT", "user provider requires user scope"));
        }
        let dir = Dir::open_ambient_dir(&config.root, cap_std::ambient_authority())
            .map_err(|e| err("READ_ERROR", e))?;
        let mut providers = self.providers.lock().map_err(storage)?;
        if providers.len() >= 64 {
            return Err(err("LIMIT", "provider limit exceeded"));
        }
        if providers.iter().any(|p| {
            p.config.id == config.id && p.config.scope == config.scope && p.owner == host.actor_id
        }) {
            return Err(err("CONFLICT", "provider already registered"));
        }
        providers.push(Arc::new(Provider {
            config,
            owner: host.actor_id.clone(),
            dir,
        }));
        Ok(())
    }
    pub fn invalidate(&self, host: &AccessContext, scope: &Scope, provider_id: &str) -> Result<()> {
        host.authorize(scope)?;
        let providers = self.providers.lock().map_err(storage)?;
        if !providers.iter().any(|p| {
            &p.config.scope == scope && p.config.id == provider_id && p.owner == host.actor_id
        }) {
            return Err(err("NOT_FOUND", "provider not registered"));
        }
        // Discovery rescans on every call, so no cache needs invalidation.
        Ok(())
    }
    fn save_revision(
        &self,
        scope: &str,
        info: &SkillInfo,
        body: &str,
        requires_user: bool,
    ) -> Result<()> {
        self.db
            .lock()
            .map_err(storage)?
            .execute(
                "INSERT OR IGNORE INTO skill_revisions VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    scope,
                    info.name,
                    info.revision,
                    serde_json::to_string(info).map_err(storage)?,
                    body,
                    requires_user
                ],
            )
            .map_err(storage)?;
        Ok(())
    }
    pub fn list(&self, host: &AccessContext, scope: &Scope) -> Result<SkillCatalog> {
        host.authorize(scope)?;
        let key = scope_key(host, scope)?;
        let mut providers: Vec<_> = self
            .providers
            .lock()
            .map_err(storage)?
            .iter()
            .filter(|p| {
                (&p.config.scope == scope && p.owner == host.actor_id)
                    || (p.config.scope == Scope::User
                        && host.allow_user_scope
                        && p.owner == host.actor_id)
            })
            .cloned()
            .collect();
        providers.sort_by(|a, b| {
            (a.config.layer, a.config.order, &a.config.id).cmp(&(
                b.config.layer,
                b.config.order,
                &b.config.id,
            ))
        });
        let mut found: BTreeMap<String, (u8, SkillInfo)> = BTreeMap::new();
        let mut diagnostics = Vec::new();
        let mut blocked: BTreeMap<String, u8> = BTreeMap::new();
        let mut incomplete = false;
        for provider in providers {
            let entries = match provider.dir.entries() {
                Ok(e) => e,
                Err(e) => {
                    incomplete = true;
                    diagnostics.push(format!("{}: {e}", provider.config.id));
                    continue;
                }
            };
            let mut names = Vec::new();
            let mut scanned = 0;
            for entry in entries.take(MAX_SKILLS + 1) {
                scanned += 1;
                match entry {
                    Ok(entry) => {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if valid_name(&name) {
                            names.push(name);
                        }
                    }
                    Err(e) => {
                        incomplete = true;
                        diagnostics.push(format!("{}: {e}", provider.config.id));
                    }
                }
            }
            if scanned > MAX_SKILLS {
                incomplete = true;
                diagnostics.push(format!("{}: entry limit exceeded", provider.config.id));
                continue;
            }
            names.sort();
            for name in names {
                let rank = match provider.config.layer {
                    Layer::Project => 0,
                    Layer::Custom => 1,
                    Layer::User => 3,
                    Layer::Bundled => 4,
                };
                if found.contains_key(&name) || blocked.contains_key(&name) {
                    diagnostics.push(format!(
                        "collision for {name}: {} ignored",
                        provider.config.id
                    ));
                    continue;
                }
                if found.len() >= MAX_SKILLS {
                    incomplete = true;
                    diagnostics.push("catalog limit exceeded".into());
                    break;
                }
                let body = match read_skill(&provider.dir, &name) {
                    Ok(body) => body,
                    Err(e) if e.code == "NOT_FOUND" => continue,
                    Err(e) => {
                        incomplete = true;
                        blocked.insert(name.clone(), rank);
                        diagnostics.push(format!("{}:{name}: {e}", provider.config.id));
                        continue;
                    }
                };
                let info = match make_info(
                    &name,
                    &provider.config.id,
                    &body,
                    Some(format!("{name}/SKILL.md")),
                ) {
                    Ok(info) => info,
                    Err(e) => {
                        incomplete = true;
                        blocked.insert(name.clone(), rank);
                        diagnostics.push(format!("{}:{name}: {e}", provider.config.id));
                        continue;
                    }
                };
                self.save_revision(&key, &info, &body, provider.config.scope == Scope::User)?;
                found.insert(name, (rank, info));
            }
        }
        // Managed publications follow project/custom and precede user/bundled.
        let managed: Vec<(String, String, String)> = {
            let db = self.db.lock().map_err(storage)?;
            let mut stmt = db.prepare("SELECT r.name,r.info,r.body FROM skill_current c JOIN skill_revisions r ON r.scope=c.scope AND r.name=c.name AND r.revision=c.revision WHERE c.scope=?1 ORDER BY r.name LIMIT 257").map_err(storage)?;
            let rows = stmt
                .query_map([&key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .map_err(storage)?;
            rows.collect::<std::result::Result<_, _>>()
                .map_err(storage)?
        };
        for (name, json, _) in managed {
            if blocked.get(&name).is_some_and(|rank| *rank <= 2) {
                diagnostics.push(format!("collision for {name}: managed ignored"));
                continue;
            }
            if found.len() >= MAX_SKILLS {
                incomplete = true;
                diagnostics.push("catalog limit exceeded".into());
                break;
            }
            let info: SkillInfo = serde_json::from_str(&json).map_err(storage)?;
            if let Some((rank, previous)) = found.get(&name) {
                if *rank <= 2 {
                    diagnostics.push(format!("collision for {name}: managed ignored"));
                    continue;
                }
                diagnostics.push(format!(
                    "collision for {name}: {} ignored",
                    previous.provider_id
                ));
            }
            found.insert(name, (2, info));
        }
        let items: Vec<_> = found.into_values().map(|(_, info)| info).collect();
        let revision = digest(serde_json::to_string(&items).map_err(storage)?.as_bytes());
        Ok(SkillCatalog {
            items,
            incomplete,
            revision,
            diagnostics,
        })
    }
    pub fn load(
        &self,
        host: &AccessContext,
        scope: &Scope,
        request: &LoadSkill,
        invocation: Invocation,
    ) -> Result<LoadedSkill> {
        host.authorize(scope)?;
        if !valid_name(&request.name) {
            return Err(err("INVALID_INPUT", "invalid skill name"));
        }
        let revision = match &request.revision {
            Some(revision) => revision.clone(),
            None => {
                self.list(host, scope)?
                    .items
                    .into_iter()
                    .find(|i| i.name == request.name)
                    .ok_or_else(|| err("NOT_FOUND", "skill not found"))?
                    .revision
            }
        };
        let key = scope_key(host, scope)?;
        let row: Option<(String,String,bool)> = self.db.lock().map_err(storage)?.query_row("SELECT info,body,requires_user FROM skill_revisions WHERE scope=?1 AND name=?2 AND revision=?3", params![key,request.name,revision], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(storage)?;
        let (json, body, requires_user) =
            row.ok_or_else(|| err("NOT_FOUND", "skill revision not found"))?;
        let info: SkillInfo = serde_json::from_str(&json).map_err(storage)?;
        if requires_user && !host.allow_user_scope {
            return Err(err("NOT_FOUND_OR_FORBIDDEN", "scope unavailable"));
        }
        check_invocation(&info, invocation)?;
        Ok(LoadedSkill { info, body })
    }
    pub fn load_and_select(
        &self,
        host: &AccessContext,
        scope: &Scope,
        name: &str,
        selections: &dyn SelectionStore,
    ) -> Result<LoadedSkill> {
        let loaded = self.load(
            host,
            scope,
            &LoadSkill {
                name: name.into(),
                revision: None,
            },
            Invocation::Model,
        )?;
        selections.select(&host.session_id, name).map_err(storage)?;
        Ok(loaded)
    }
    /// CAS publication; `expected=None` requires that no current revision exists.
    pub fn publish(
        &self,
        host: &AccessContext,
        scope: &Scope,
        name: &str,
        body: &str,
        expected: Option<&str>,
    ) -> Result<SkillInfo> {
        self.publish_inner(host, scope, name, body, expected, None)
    }
    pub fn publish_audited(
        &self,
        host: &AccessContext,
        scope: &Scope,
        name: &str,
        body: &str,
        expected: Option<&str>,
        request_id: &str,
    ) -> Result<SkillInfo> {
        if request_id.is_empty() || request_id.len() > 256 {
            return Err(err("INVALID_INPUT", "invalid request id"));
        }
        self.publish_inner(host, scope, name, body, expected, Some(request_id))
    }
    fn publish_inner(
        &self,
        host: &AccessContext,
        scope: &Scope,
        name: &str,
        body: &str,
        expected: Option<&str>,
        request_id: Option<&str>,
    ) -> Result<SkillInfo> {
        host.authorize(scope)?;
        if !valid_name(name) || body.len() > MAX_SOURCE {
            return Err(err("INVALID_INPUT", "invalid skill name or size"));
        }
        let info = make_info(name, "managed", body, None)?;
        let key = scope_key(host, scope)?;
        let mut db = self.db.lock().map_err(storage)?;
        let tx = db.transaction().map_err(storage)?;
        let payload = digest(
            serde_json::to_vec(&(
                host.actor_id.as_str(),
                &key,
                name,
                body,
                expected,
                "publish",
            ))
            .map_err(storage)?
            .as_slice(),
        );
        if let Some(id) = request_id {
            let previous: Option<(String,String)> = tx.query_row("SELECT payload_digest,result_revision FROM skill_publication_audit WHERE request_id=?1", [id], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(storage)?;
            if let Some((old, revision)) = previous {
                if old != payload {
                    return Err(err("CONFLICT", "request id reused"));
                }
                if revision != info.revision {
                    return Err(err("STORAGE", "audit mismatch"));
                }
                return Ok(info);
            }
        }
        let current: Option<String> = tx
            .query_row(
                "SELECT revision FROM skill_current WHERE scope=?1 AND name=?2",
                params![key, name],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)?;
        if current.as_deref() != expected {
            return Err(err("CONFLICT", "stale skill revision"));
        }
        tx.execute(
            "INSERT OR IGNORE INTO skill_revisions VALUES (?1,?2,?3,?4,?5,0)",
            params![
                key,
                name,
                info.revision,
                serde_json::to_string(&info).map_err(storage)?,
                body
            ],
        )
        .map_err(storage)?;
        tx.execute("INSERT INTO skill_current VALUES (?1,?2,?3) ON CONFLICT(scope,name) DO UPDATE SET revision=excluded.revision", params![key,name,info.revision]).map_err(storage)?;
        if let Some(id) = request_id {
            tx.execute(
                "INSERT INTO skill_publication_audit VALUES (?1,?2,?3,'publish',?4,?5,?6,?7)",
                params![
                    id,
                    host.actor_id,
                    key,
                    name,
                    expected,
                    info.revision,
                    payload
                ],
            )
            .map_err(storage)?;
        }
        tx.commit().map_err(storage)?;
        Ok(info)
    }
    pub fn publication_result(
        &self,
        host: &AccessContext,
        scope: &Scope,
        name: &str,
        body: &str,
        expected: Option<&str>,
        request_id: &str,
    ) -> Result<Option<String>> {
        host.authorize(scope)?;
        let key = scope_key(host, scope)?;
        let payload = digest(
            serde_json::to_vec(&(
                host.actor_id.as_str(),
                &key,
                name,
                body,
                expected,
                "publish",
            ))
            .map_err(storage)?
            .as_slice(),
        );
        let db = self.db.lock().map_err(storage)?;
        let row: Option<(String,String)> = db.query_row("SELECT payload_digest,result_revision FROM skill_publication_audit WHERE request_id=?1", [request_id], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(storage)?;
        match row {
            Some((hash, revision)) if hash == payload => Ok(Some(revision)),
            Some(_) => Err(err("CONFLICT", "request id reused")),
            None => Ok(None),
        }
    }
    pub fn rollback(
        &self,
        host: &AccessContext,
        scope: &Scope,
        name: &str,
        target: &str,
        expected: &str,
    ) -> Result<()> {
        self.rollback_inner(host, scope, name, target, expected, None)
    }
    pub fn rollback_audited(
        &self,
        host: &AccessContext,
        scope: &Scope,
        name: &str,
        target: &str,
        expected: &str,
        request_id: &str,
    ) -> Result<()> {
        if request_id.is_empty() || request_id.len() > 256 {
            return Err(err("INVALID_INPUT", "invalid request id"));
        }
        self.rollback_inner(host, scope, name, target, expected, Some(request_id))
    }
    fn rollback_inner(
        &self,
        host: &AccessContext,
        scope: &Scope,
        name: &str,
        target: &str,
        expected: &str,
        request_id: Option<&str>,
    ) -> Result<()> {
        host.authorize(scope)?;
        let key = scope_key(host, scope)?;
        let mut db = self.db.lock().map_err(storage)?;
        let tx = db.transaction().map_err(storage)?;
        let payload = digest(
            serde_json::to_vec(&(
                host.actor_id.as_str(),
                &key,
                name,
                target,
                expected,
                "rollback",
            ))
            .map_err(storage)?
            .as_slice(),
        );
        if let Some(id) = request_id {
            let previous: Option<String> = tx
                .query_row(
                    "SELECT payload_digest FROM skill_publication_audit WHERE request_id=?1",
                    [id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(storage)?;
            if let Some(old) = previous {
                return if old == payload {
                    Ok(())
                } else {
                    Err(err("CONFLICT", "request id reused"))
                };
            }
        }
        let target_info: Option<String> = tx
            .query_row(
                "SELECT info FROM skill_revisions WHERE scope=?1 AND name=?2 AND revision=?3",
                params![key, name, target],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)?;
        let info: SkillInfo = serde_json::from_str(
            &target_info.ok_or_else(|| err("NOT_FOUND", "revision not found"))?,
        )
        .map_err(storage)?;
        if info.provider_id != "managed" {
            return Err(err("INVALID_INPUT", "target is not managed"));
        }
        let changed = tx
            .execute(
                "UPDATE skill_current SET revision=?1 WHERE scope=?2 AND name=?3 AND revision=?4",
                params![target, key, name, expected],
            )
            .map_err(storage)?;
        if changed != 1 {
            return Err(err("CONFLICT", "stale skill revision"));
        }
        if let Some(id) = request_id {
            tx.execute(
                "INSERT INTO skill_publication_audit VALUES (?1,?2,?3,'rollback',?4,?5,?6,?7)",
                params![id, host.actor_id, key, name, expected, target, payload],
            )
            .map_err(storage)?;
        }
        tx.commit().map_err(storage)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;
    fn host(actor: &str) -> AccessContext {
        AccessContext {
            actor_id: actor.into(),
            session_id: "s".into(),
            workspace_id: Some("w".into()),
            allow_user_scope: true,
        }
    }
    fn scope() -> Scope {
        Scope::Workspace {
            workspace_id: "w".into(),
        }
    }
    fn setup() -> (TempDir, SkillsService) {
        let d = TempDir::new().unwrap();
        let s = SkillsService::open(&d.path().join("skills.db")).unwrap();
        (d, s)
    }
    fn skill(root: &Path, name: &str, text: &str) {
        fs::create_dir_all(root.join(name)).unwrap();
        fs::write(root.join(name).join("SKILL.md"), text).unwrap();
    }
    fn register(
        s: &SkillsService,
        h: &AccessContext,
        scope: Scope,
        root: &Path,
        layer: Layer,
        id: &str,
    ) {
        s.register_provider(
            h,
            ProviderConfig {
                id: id.into(),
                scope,
                layer,
                order: 0,
                root: root.into(),
            },
        )
        .unwrap();
    }
    fn load(
        s: &SkillsService,
        h: &AccessContext,
        name: &str,
        revision: Option<String>,
        invocation: Invocation,
    ) -> Result<LoadedSkill> {
        s.load(
            h,
            &scope(),
            &LoadSkill {
                name: name.into(),
                revision,
            },
            invocation,
        )
    }

    #[test]
    fn precedence_live_edits_and_pins() {
        let (d, s) = setup();
        let h = host("a");
        let project = d.path().join("project");
        let user = d.path().join("user");
        skill(&project, "same", "---\ndescription: project\n---\nfirst");
        skill(&user, "same", "---\ndescription: user\n---\nuser");
        register(&s, &h, Scope::User, &user, Layer::User, "user");
        register(&s, &h, scope(), &project, Layer::Project, "project");
        let catalog = s.list(&h, &scope()).unwrap();
        assert_eq!(catalog.items[0].description, "project");
        assert_eq!(catalog.diagnostics.len(), 1);
        let revision = catalog.items[0].revision.clone();
        skill(&project, "same", "---\ndescription: changed\n---\nsecond");
        assert!(load(&s, &h, "same", None, Invocation::Model)
            .unwrap()
            .body
            .ends_with("second"));
        fs::remove_dir_all(project.join("same")).unwrap();
        assert!(load(&s, &h, "same", Some(revision), Invocation::Model)
            .unwrap()
            .body
            .ends_with("first"));
        assert!(load(&s, &h, "same", None, Invocation::Model)
            .unwrap()
            .body
            .ends_with("user"));
    }
    #[test]
    fn flags_are_strict_and_invalid_higher_priority_cannot_fall_back() {
        let (d, s) = setup();
        let h = host("a");
        let p = d.path().join("p");
        let u = d.path().join("u");
        skill(
            &p,
            "restricted",
            "---\ndisable-model-invocation: true\nuser-invocable: false\n---\nbody",
        );
        skill(
            &p,
            "bad",
            "---\ndisable-model-invocation: 'false'\n---\nbody",
        );
        skill(&u, "bad", "fallback");
        register(&s, &h, scope(), &p, Layer::Project, "p");
        register(&s, &h, Scope::User, &u, Layer::User, "u");
        let catalog = s.list(&h, &scope()).unwrap();
        assert!(catalog.incomplete);
        assert!(catalog.items.iter().all(|i| i.name != "bad"));
        assert_eq!(
            load(&s, &h, "restricted", None, Invocation::Model)
                .unwrap_err()
                .code,
            "FORBIDDEN"
        );
        assert_eq!(
            load(&s, &h, "restricted", None, Invocation::User)
                .unwrap_err()
                .code,
            "FORBIDDEN"
        );
        assert!(load(&s, &h, "restricted", None, Invocation::Selected).is_ok());
    }
    #[test]
    fn managed_cas_rollback_and_actor_isolation() {
        let (d, s) = setup();
        let a = host("a");
        let b = host("b");
        let one = s.publish(&a, &scope(), "skill", "one", None).unwrap();
        let two = s
            .publish(&a, &scope(), "skill", "two", Some(&one.revision))
            .unwrap();
        assert_eq!(
            s.publish(&a, &scope(), "skill", "lost", Some(&one.revision))
                .unwrap_err()
                .code,
            "CONFLICT"
        );
        assert_eq!(
            s.rollback(&a, &scope(), "skill", &one.revision, &one.revision)
                .unwrap_err()
                .code,
            "CONFLICT"
        );
        s.rollback(&a, &scope(), "skill", &one.revision, &two.revision)
            .unwrap();
        assert_eq!(
            load(&s, &a, "skill", None, Invocation::User).unwrap().body,
            "one"
        );
        assert_eq!(
            load(&s, &a, "skill", Some(two.revision), Invocation::User)
                .unwrap()
                .body,
            "two"
        );
        assert!(load(&s, &b, "skill", None, Invocation::User).is_err());
        let p = d.path().join("p");
        skill(&p, "skill", "project");
        register(&s, &a, scope(), &p, Layer::Project, "p");
        assert_eq!(
            load(&s, &a, "skill", None, Invocation::User).unwrap().body,
            "project"
        );
    }
    #[test]
    fn scope_and_user_revocation_are_checked_for_pins() {
        let (d, s) = setup();
        let mut a = host("a");
        let p = d.path().join("user");
        skill(&p, "x", "private");
        register(&s, &a, Scope::User, &p, Layer::User, "user");
        let revision = s.list(&a, &scope()).unwrap().items[0].revision.clone();
        a.allow_user_scope = false;
        assert!(load(&s, &a, "x", Some(revision), Invocation::User).is_err());
        let other = Scope::Workspace {
            workspace_id: "other".into(),
        };
        assert!(s.list(&a, &other).is_err());
        assert!(load(&s, &a, "../x", None, Invocation::User).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn scoped_root_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;
        let (d, s) = setup();
        let a = host("a");
        let p = d.path().join("p");
        let outside = d.path().join("outside");
        fs::create_dir_all(&p).unwrap();
        skill(&outside, "x", "secret");
        symlink(outside.join("x"), p.join("x")).unwrap();
        register(&s, &a, scope(), &p, Layer::Project, "p");
        assert!(s.list(&a, &scope()).unwrap().items.is_empty());
    }
    #[test]
    fn newer_schema_is_rejected() {
        let d = TempDir::new().unwrap();
        let path = d.path().join("future.db");
        let db = Connection::open(&path).unwrap();
        db.pragma_update(None, "user_version", 2).unwrap();
        drop(db);
        assert_eq!(
            SkillsService::open(&path).err().unwrap().code,
            "SCHEMA_VERSION"
        );
    }
}
