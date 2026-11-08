//! System workspace plugin: durable folder registry and session membership.
use crate::{context::ContextService, sessions::SessionService};
use cordis::{Context, Plugin, Service};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    convert::Infallible,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    pub id: String,
    pub title: String,
    pub path: Option<String>,
    pub session_ids: Vec<String>,
    pub missing: bool,
    pub pinned_session_ids: Vec<String>,
    pub archived_session_ids: Vec<String>,
}

pub(crate) fn migrate(db: &Connection) -> Result<(), String> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS projects(id TEXT PRIMARY KEY,title TEXT NOT NULL,path TEXT,path_key TEXT UNIQUE);
        CREATE TABLE IF NOT EXISTS hidden_projects(id TEXT PRIMARY KEY REFERENCES projects(id));
        CREATE TABLE IF NOT EXISTS session_navigation(session TEXT PRIMARY KEY,pinned INTEGER NOT NULL DEFAULT 0,archived INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS project_sessions(session TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects(id));")
        .map_err(|e| e.to_string())
}

fn path_key(path: &str) -> String {
    if cfg!(windows) {
        path.to_lowercase()
    } else {
        path.to_owned()
    }
}

pub struct WorkspaceService {
    context: Arc<ContextService>,
    sessions: Arc<SessionService>,
    local: bool,
    stopped: AtomicBool,
}
impl Service for WorkspaceService {
    const NAME: &'static str = "studio.workspaces";
}
impl WorkspaceService {
    fn open(
        context: Arc<ContextService>,
        sessions: Arc<SessionService>,
        root: &Path,
        local: bool,
    ) -> Result<Self, String> {
        let service = Self {
            context,
            sessions,
            local,
            stopped: AtomicBool::new(false),
        };
        {
            let db = service.context.db()?;
            let exists: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM projects WHERE id='default')",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if !exists {
                let path = if local {
                    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
                    Some(
                        root.canonicalize()
                            .map_err(|e| e.to_string())?
                            .to_str()
                            .ok_or("Workspace path must be UTF-8")?
                            .to_owned(),
                    )
                } else {
                    None
                };
                db.execute(
                    "INSERT INTO projects VALUES('default','Default workspace',?1,?2)",
                    params![path, path.as_deref().map(path_key)],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        service.import_legacy()?;
        Ok(service)
    }
    fn live(&self) -> Result<(), String> {
        if self.stopped.load(Ordering::Acquire) {
            Err("Workspace service is stopped".into())
        } else {
            Ok(())
        }
    }
    // Resume safely after interruption; never overwrite an existing membership or session settings.
    fn import_legacy(&self) -> Result<(), String> {
        let sessions = self.sessions.list()?;
        let mut db = self.context.db()?;
        let tx = db.transaction().map_err(|e| e.to_string())?;
        for session in sessions {
            let assigned: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM project_sessions WHERE session=?1)",
                    [&session.id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if assigned {
                continue;
            }
            let raw: Option<String> = tx
                .query_row(
                    "SELECT settings FROM workspace WHERE session=?1",
                    [&session.id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            let settings: crate::context::WorkspaceSettings = raw
                .map(|s| serde_json::from_str(&s))
                .transpose()
                .map_err(|e| e.to_string())?
                .unwrap_or_default();
            let project = if self.local {
                if let Some(path) = settings.cwd {
                    let canonical = Path::new(&path)
                        .canonicalize()
                        .unwrap_or_else(|_| PathBuf::from(&path));
                    Self::register(
                        &tx,
                        canonical.to_str().ok_or("Workspace path must be UTF-8")?,
                    )?
                } else {
                    "default".into()
                }
            } else {
                "default".into()
            };
            tx.execute(
                "INSERT INTO project_sessions VALUES(?1,?2)",
                params![session.id, project],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }
    fn register(db: &Connection, path: &str) -> Result<String, String> {
        let key = path_key(path);
        let title = Path::new(path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(path);
        db.execute(
            "INSERT OR IGNORE INTO projects VALUES(lower(hex(randomblob(16))),?1,?2,?3)",
            params![title, path, key],
        )
        .map_err(|e| e.to_string())?;
        db.query_row("SELECT id FROM projects WHERE path_key=?1", [key], |r| {
            r.get(0)
        })
        .map_err(|e| e.to_string())
    }
    pub fn list(&self) -> Result<Vec<WorkspaceInfo>, String> {
        self.live()?;
        let sessions = self.sessions.list()?;
        let db = self.context.db()?;
        let mut query = db
            .prepare("SELECT id,title,path FROM projects WHERE id NOT IN (SELECT id FROM hidden_projects) ORDER BY id='default',rowid DESC")
            .map_err(|e| e.to_string())?;
        let rows = query
            .query_map([], |r| {
                Ok(WorkspaceInfo {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    path: r.get(2)?,
                    session_ids: vec![],
                    missing: false,
                    pinned_session_ids: vec![],
                    archived_session_ids: vec![],
                })
            })
            .map_err(|e| e.to_string())?;
        let mut projects = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let mut membership = db
            .prepare("SELECT session,project FROM project_sessions")
            .map_err(|e| e.to_string())?;
        let owners = membership
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<std::collections::HashMap<_, _>, _>>()
            .map_err(|e| e.to_string())?;
        let mut navigation = db.prepare("SELECT session,pinned,archived FROM session_navigation").map_err(|e| e.to_string())?;
        let states = navigation.query_map([], |r| Ok((r.get::<_,String>(0)?,(r.get::<_,bool>(1)?,r.get::<_,bool>(2)?))))
            .map_err(|e| e.to_string())?.collect::<Result<std::collections::HashMap<_,_>,_>>().map_err(|e| e.to_string())?;
        for project in &mut projects {
            project.missing = project
                .path
                .as_ref()
                .is_some_and(|p| !Path::new(p).is_dir());
            for session in &sessions {
                if owners.get(&session.id) == Some(&project.id) {
                    project.session_ids.push(session.id.clone());
                    if let Some((pinned,archived)) = states.get(&session.id) {
                        if *pinned { project.pinned_session_ids.push(session.id.clone()); }
                        if *archived { project.archived_session_ids.push(session.id.clone()); }
                    }
                }
            }
        }
        Ok(projects)
    }
    pub fn create(&self, path: &str) -> Result<WorkspaceInfo, String> {
        self.live()?;
        if !self.local {
            return Err("Local folders are unavailable in this profile".into());
        }
        if !Path::new(path).is_absolute() {
            return Err("Workspace path must be absolute".into());
        }
        let path = Path::new(path).canonicalize().map_err(|e| e.to_string())?;
        if !path.is_dir() {
            return Err("Workspace must be a directory".into());
        }
        let id = Self::register(
            &*self.context.db()?,
            path.to_str().ok_or("Workspace path must be UTF-8")?,
        )?;
        self.context.db()?.execute("DELETE FROM hidden_projects WHERE id=?1", [&id]).map_err(|e| e.to_string())?;
        self.list()?
            .into_iter()
            .find(|p| p.id == id)
            .ok_or("Workspace not found".into())
    }
    pub fn set_session_state(&self, id: &str, pinned: bool, archived: bool) -> Result<(), String> {
        self.live()?;
        self.sessions.get(id)?;
        self.context.db()?.execute("INSERT INTO session_navigation VALUES(?1,?2,?3) ON CONFLICT(session) DO UPDATE SET pinned=excluded.pinned,archived=excluded.archived",params![id,pinned,archived]).map_err(|e|e.to_string())?;
        Ok(())
    }
    pub fn fork_session(&self, source: &str, id: &str) -> Result<crate::sessions::SessionInfo, String> {
        self.live()?;
        if !crate::sessions::valid_id(id) { return Err("Invalid session ID".into()); }
        let mut db=self.context.db()?;
        let tx=db.transaction().map_err(|e|e.to_string())?;
        let project:String=tx.query_row("SELECT project FROM project_sessions WHERE session=?1",[source],|r|r.get(0)).map_err(|_|"Workspace not found")?;
        tx.execute("INSERT INTO project_sessions VALUES(?1,?2)",params![id,project]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO workspace SELECT ?1,settings FROM workspace WHERE session=?2",params![id,source]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO selected_skills SELECT ?1,name FROM selected_skills WHERE session=?2",params![id,source]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO summaries SELECT ?1,prefix,summary FROM summaries WHERE session=?2",params![id,source]).map_err(|e|e.to_string())?;
        tx.execute("INSERT INTO attachment_access SELECT ?1,id FROM attachments WHERE session=?2 UNION SELECT ?1,attachment FROM attachment_access WHERE session=?2",params![id,source]).map_err(|e|e.to_string())?;
        self.sessions.fork(source,id)?;
        tx.commit().map_err(|e|e.to_string())?;
        self.sessions.get(id)
    }
    pub fn rename(&self, id: &str, title: &str) -> Result<(), String> {
        self.live()?;
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 128 || title.chars().any(char::is_control) {
            return Err("Workspace title must contain 1–128 characters without control characters".into());
        }
        let changed = self.context.db()?.execute("UPDATE projects SET title=?1 WHERE id=?2", params![title, id]).map_err(|e| e.to_string())?;
        if changed == 0 { return Err("Workspace not found".into()); }
        Ok(())
    }
    pub fn remove(&self, id: &str) -> Result<(), String> {
        self.live()?;
        if id == "default" { return Err("Default workspace cannot be removed".into()); }
        let changed = self.context.db()?.execute("INSERT OR IGNORE INTO hidden_projects SELECT id FROM projects WHERE id=?1", [id]).map_err(|e| e.to_string())?;
        if changed == 0 { return Err("Workspace not found or already removed".into()); }
        Ok(())
    }
    pub fn create_session(&self, id: &str, project: &str) -> Result<(), String> {
        self.live()?;
        if !crate::sessions::valid_id(id) {
            return Err("Invalid session ID".into());
        }
        {
            let db = self.context.db()?;
            let path: Option<String> = db
                .query_row("SELECT path FROM projects WHERE id=?1", [project], |r| {
                    r.get(0)
                })
                .map_err(|_| "Workspace not found")?;
            if path.as_ref().is_some_and(|p| !Path::new(p).is_dir()) {
                return Err("Workspace directory is unavailable".into());
            }
            db.execute(
                "INSERT INTO project_sessions VALUES(?1,?2)",
                params![id, project],
            )
            .map_err(|e| e.to_string())?;
        }
        if let Err(error) = self.sessions.create(id) {
            self.context
                .db()?
                .execute("DELETE FROM project_sessions WHERE session=?1", [id])
                .map_err(|e| e.to_string())?;
            return Err(error);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_actions_fork_history_and_scope_attachment_access() {
        let dir=tempfile::tempdir().unwrap();
        let context=Arc::new(ContextService::open(&dir.path().join("context.db")).unwrap());
        let sessions=Arc::new(SessionService::open(&dir.path().join("sessions.db")).unwrap());
        let registry=WorkspaceService::open(context.clone(),sessions.clone(),&dir.path().join("root"),true).unwrap();
        registry.create_session("source","default").unwrap();
        registry.create_session("unrelated","default").unwrap();
        let file=context.put_attachment("source","file.txt","text/plain","aGVsbG8=").unwrap();
        sessions.save("source",crate::sessions::SessionMessage{id:"u1".into(),parent_id:None,role:"user".into(),created_at:"2026-11-08".into(),parts:vec![crate::sessions::ContentPart{kind:"file".into(),data:serde_json::to_value(&file).unwrap()}],extensions:Default::default()},None).unwrap();
        sessions.rename("source","Renamed").unwrap();
        assert!(sessions.rename("source"," \n").is_err());
        registry.set_session_state("source",true,true).unwrap();
        let workspace=&registry.list().unwrap()[0];
        assert_eq!(workspace.pinned_session_ids,["source"]);assert_eq!(workspace.archived_session_ids,["source"]);
        let fork=registry.fork_session("source","fork").unwrap();
        assert_eq!(fork.title,"Renamed — ветка");
        assert_eq!(sessions.load("fork").unwrap().len(),1);
        assert_eq!(context.get_attachment("fork",&file.id).unwrap().data,"aGVsbG8=");
        assert!(context.get_attachment("unrelated",&file.id).is_err());
        registry.fork_session("fork","fork-two").unwrap();
        assert!(context.get_attachment("fork-two",&file.id).is_ok());
        assert_eq!(context.workspace_get("fork").unwrap().cwd,context.workspace_get("source").unwrap().cwd);
        assert!(registry.fork_session("source","fork").is_err());
        assert!(registry.fork_session("missing","other").is_err());
        assert_eq!(sessions.load("source").unwrap().len(),1);
        registry.set_session_state("source",false,false).unwrap();
        assert!(registry.list().unwrap()[0].archived_session_ids.is_empty());
        registry.create_session("running","default").unwrap();
        let request=serde_json::from_value(serde_json::json!({"sessionId":"running","requestId":"run","parentId":null,"message":{"id":"user","text":"work"},"model":"test","options":{}})).unwrap();
        sessions.accept_run(&request,"2026-11-08").unwrap();
        assert!(registry.fork_session("running","denied").is_err());
        assert!(sessions.get("denied").is_err());
        assert!(!registry.list().unwrap()[0].session_ids.contains(&"denied".to_owned()));
    }
    #[test]
    fn rename_and_remove_preserve_history_folder_and_membership() {
        let dir = tempfile::tempdir().unwrap();
        let context = Arc::new(ContextService::open(&dir.path().join("context.db")).unwrap());
        let sessions = Arc::new(SessionService::open(&dir.path().join("sessions.db")).unwrap());
        let root = dir.path().join("default");
        let registry = WorkspaceService::open(context.clone(), sessions.clone(), &root, true).unwrap();
        let project = registry.create(dir.path().to_str().unwrap()).unwrap();
        registry.create_session("kept", &project.id).unwrap();
        assert!(registry.rename(&project.id, " \n").is_err());
        assert!(registry.rename("unknown", "Title").is_err());
        registry.rename(&project.id, " Renamed ").unwrap();
        registry.rename("default", "Inbox").unwrap();
        assert!(registry.remove("default").is_err());
        registry.remove(&project.id).unwrap();
        assert_eq!(registry.list().unwrap().len(), 1);
        assert!(sessions.get("kept").is_ok());
        assert_eq!(context.workspace_get("kept").unwrap().cwd, project.path);
        drop(registry);
        let registry = WorkspaceService::open(context, sessions, &root, true).unwrap();
        assert_eq!(registry.list().unwrap().len(), 1);
        let restored = registry.create(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(restored.id, project.id);
        assert_eq!(restored.title, "Renamed");
        assert_eq!(restored.session_ids, ["kept"]);
        assert!(dir.path().is_dir());
    }
    #[test]
    fn registry_migrates_preserves_policy_and_reopens_without_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let context = Arc::new(ContextService::open(&dir.path().join("context.db")).unwrap());
        let sessions = Arc::new(SessionService::open(&dir.path().join("sessions.db")).unwrap());
        sessions.create("old").unwrap();
        let mut settings = crate::context::WorkspaceSettings::default();
        settings.cwd = Some(dir.path().to_str().unwrap().into());
        settings.context_window = 8192;
        context.workspace_save("old", settings).unwrap();
        let root = dir.path().join("default-workspace");
        let registry =
            WorkspaceService::open(context.clone(), sessions.clone(), &root, true).unwrap();
        let project = registry.create(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(project.session_ids, ["old"]);
        assert_eq!(
            registry
                .create(dir.path().join(".").to_str().unwrap())
                .unwrap()
                .id,
            project.id
        );
        registry.create_session("new", &project.id).unwrap();
        registry
            .create_session("default-session", "default")
            .unwrap();
        assert_eq!(context.workspace_get("new").unwrap().cwd, project.path);
        assert_eq!(context.workspace_get("old").unwrap().context_window, 8192);
        assert_ne!(
            context.workspace_get("default-session").unwrap().cwd,
            project.path
        );
        assert!(registry.create_session("bad", "unknown").is_err());
        assert!(sessions.get("bad").is_err());
        assert!(registry.create("relative").is_err());
        assert!(registry
            .create(dir.path().join("context.db").to_str().unwrap())
            .is_err());
        let changed = crate::context::WorkspaceSettings::default();
        assert!(context.workspace_save("new", changed).is_err());
        drop(registry);
        let registry = WorkspaceService::open(context, sessions, &root, true).unwrap();
        assert_eq!(registry.list().unwrap().len(), 2);
        assert_eq!(
            registry
                .list()
                .unwrap()
                .iter()
                .find(|p| p.id == project.id)
                .unwrap()
                .session_ids
                .len(),
            2
        );
        assert!(registry.create_session("new", "default").is_err());
        drop(registry);
        let context = Arc::new(ContextService::open(&dir.path().join("context.db")).unwrap());
        let sessions = Arc::new(SessionService::open(&dir.path().join("sessions.db")).unwrap());
        let registry = WorkspaceService::open(context.clone(), sessions, &root, true).unwrap();
        assert_eq!(registry.list().unwrap().len(), 2);
        assert_eq!(context.workspace_get("new").unwrap().cwd, project.path);
    }
    #[test]
    fn missing_folders_retain_history_and_hosted_profiles_cannot_register_paths() {
        let dir = tempfile::tempdir().unwrap();
        let context = Arc::new(ContextService::open(&dir.path().join("context.db")).unwrap());
        let sessions = Arc::new(SessionService::open(&dir.path().join("sessions.db")).unwrap());
        let root = dir.path().join("default");
        let registry =
            WorkspaceService::open(context.clone(), sessions.clone(), &root, true).unwrap();
        registry.create_session("one", "default").unwrap();
        std::fs::remove_dir(&root).unwrap();
        let projects = registry.list().unwrap();
        assert!(projects[0].missing);
        assert_eq!(projects[0].session_ids, ["one"]);
        assert!(registry.create_session("two", "default").is_err());
        assert!(sessions.get("two").is_err());
        drop(registry);
        let registry = WorkspaceService::open(context, sessions, &root, false).unwrap();
        assert!(registry.create(dir.path().to_str().unwrap()).is_err());
        assert!(!root.exists());
        registry.stopped.store(true, Ordering::Release);
        assert!(registry.list().is_err());
    }
}

pub(crate) struct WorkspacePlugin {
    pub root: PathBuf,
    pub local: bool,
}
impl Plugin for WorkspacePlugin {
    type Config = ();
    type Input = ();
    type PrepareError = Infallible;
    type ApplyError = std::io::Error;
    fn prepare(&self, _: ()) -> Result<(), Infallible> {
        Ok(())
    }
    async fn apply(&self, ctx: Context, _: &()) -> Result<(), Self::ApplyError> {
        let error = |e: String| std::io::Error::other(e);
        let context = ctx
            .try_service::<ContextService>()
            .map_err(|e| error(e.to_string()))?;
        let sessions = ctx
            .try_service::<SessionService>()
            .map_err(|e| error(e.to_string()))?;
        let service = Arc::new(
            WorkspaceService::open(context, sessions, &self.root, self.local).map_err(error)?,
        );
        let owned = service.clone();
        let _cleanup = ctx
            .effect(move || async move {
                owned.stopped.store(true, Ordering::Release);
                Ok::<_, std::io::Error>(())
            })
            .map_err(|e| error(e.to_string()))?;
        let _publication = ctx.provide(service).map_err(|e| error(e.to_string()))?;
        Ok(())
    }
}
