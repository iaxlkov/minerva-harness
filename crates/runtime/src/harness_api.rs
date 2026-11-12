//! Shared, typed harness operations for HTTP and desktop transports.
use crate::{context::*, tools::InteractionAnswer, Runtime};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::Value;
use std::sync::Arc;

fn args<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|_| "Invalid harness arguments".into())
}
fn encoded(value: impl serde::Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Session {
    session_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LspSession {
    session_id: String,
    #[serde(default = "default_lsp_server")]
    server_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EditorFile {
    session_id: String,
    #[serde(default)] path: String,
    #[serde(default)] text: Option<String>,
    #[serde(default)] version: Option<String>,
    #[serde(default)] query: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TerminalArgs {
    session_id: String,
    #[serde(default)] id: String,
    #[serde(default)] data: String,
    #[serde(default)] cursor: u64,
    #[serde(default)] cols: u16,
    #[serde(default)] rows: u16,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LspDocument {
    session_id: String,
    path: String,
    #[serde(default)] version: i64,
    #[serde(default)] text: Option<String>,
    #[serde(default)] operation: Option<String>,
    #[serde(default)] line: u32,
    #[serde(default)] character: u32,
}
fn default_lsp_server() -> String { "rust".into() }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Workspace {
    session_id: String,
    settings: WorkspaceSettings,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Upload {
    session_id: String,
    name: String,
    mime_type: String,
    data: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Attachment {
    session_id: String,
    id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Mcp {
    server: crate::mcp::McpServerInput,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Id {
    id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateWorkspace {
    path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameWorkspace {
    id: String,
    title: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionNavigation { id:String,pinned:bool,archived:bool }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ForkSession { source:String,id:String }
impl Runtime {
    pub fn create_session(&self, id: &str, workspace: Option<&str>) -> Result<(), String> {
        self.plugins
            .context()
            .try_service::<crate::workspaces::WorkspaceService>()
            .map_err(|e| e.to_string())?
            .create_session(id, workspace.unwrap_or("default"))
    }

    pub async fn harness_call(
        self: &Arc<Self>,
        method: &str,
        params: Value,
    ) -> Result<Value, String> {
        if self.is_stopped().await {
            return Err("runtime is stopped".into());
        }
        let harness = &self.plugins.harness;
        match method {
            "sessions.rename" => {
                let a:RenameWorkspace=args(params)?;self.sessions.rename(&a.id,&a.title)?;Ok(Value::Null)
            }
            "sessions.state" => {
                let a:SessionNavigation=args(params)?;
                self.plugins.context().try_service::<crate::workspaces::WorkspaceService>().map_err(|e|e.to_string())?.set_session_state(&a.id,a.pinned,a.archived)?;
                Ok(Value::Null)
            }
            "sessions.fork" => {
                let a:ForkSession=args(params)?;
                encoded(self.plugins.context().try_service::<crate::workspaces::WorkspaceService>().map_err(|e|e.to_string())?.fork_session(&a.source,&a.id)?)
            }
            "session.snapshot" => {
                let a: Session = args(params)?;
                encoded(self.sessions.baseline(&a.session_id)?)
            }
            "workspaces.list" => {
                let _: Empty = args(params)?;
                encoded(
                    self.plugins
                        .context()
                        .try_service::<crate::workspaces::WorkspaceService>()
                        .map_err(|e| e.to_string())?
                        .list()?,
                )
            }
            "workspaces.create" => {
                let a: CreateWorkspace = args(params)?;
                encoded(
                    self.plugins
                        .context()
                        .try_service::<crate::workspaces::WorkspaceService>()
                        .map_err(|e| e.to_string())?
                        .create(&a.path)?,
                )
            }
            "workspaces.rename" => {
                let a: RenameWorkspace = args(params)?;
                self.plugins.context().try_service::<crate::workspaces::WorkspaceService>()
                    .map_err(|e| e.to_string())?.rename(&a.id, &a.title)?;
                Ok(Value::Null)
            }
            "workspaces.remove" => {
                let a: Id = args(params)?;
                self.plugins.context().try_service::<crate::workspaces::WorkspaceService>()
                    .map_err(|e| e.to_string())?.remove(&a.id)?;
                if let Ok(lsp) = self.plugins.context().try_service::<crate::lsp::LspService>() {
                    lsp.stop_workspace(&a.id).await;
                }
                Ok(Value::Null)
            }
            "mh-terminal.open" | "mh-terminal.write" | "mh-terminal.read" | "mh-terminal.resize" | "mh-terminal.close" => {
                let a: TerminalArgs = args(params)?;
                self.sessions.get(&a.session_id)?;
                if !harness.local { return Err("Terminal is unavailable in this profile".into()); }
                let terminals = &self.plugins.shell()?.terminals;
                match method {
                    "mh-terminal.open" => {
                        let projects=self.plugins.context().try_service::<crate::workspaces::WorkspaceService>().map_err(|e|e.to_string())?;
                        let project=projects.list()?.into_iter().find(|p|p.session_ids.contains(&a.session_id)).ok_or("Workspace not found")?;
                        let root=project.path.ok_or("Папка рабочего пространства не выбрана")?;
                        encoded(terminals.open(std::path::Path::new(&root),a.cols,a.rows)?)
                    }
                    "mh-terminal.write" => terminals.write(&a.id,&a.data).map(|_| Value::Null),
                    "mh-terminal.read" => terminals.read(&a.id,a.cursor),
                    "mh-terminal.resize" => terminals.resize(&a.id,a.cols,a.rows).map(|_| Value::Null),
                    _ => terminals.close(&a.id).map(|_| Value::Null),
                }
            }
            "mh-editor.list" | "mh-editor.read" | "mh-editor.save" | "mh-editor.search" => {
                let a: EditorFile = args(params)?;
                self.sessions.get(&a.session_id)?;
                if !harness.local { return Err("Editor is unavailable in this profile".into()); }
                let projects=self.plugins.context().try_service::<crate::workspaces::WorkspaceService>().map_err(|e|e.to_string())?;
                let project=projects.list()?.into_iter().find(|p|p.session_ids.contains(&a.session_id)).ok_or("Workspace not found")?;
                let root=std::path::Path::new(project.path.as_ref().ok_or("Workspace directory is unavailable")?);
                let editor=self.plugins.context().try_service::<crate::editor::EditorService>().map_err(|e|e.to_string())?;
                match method {
                    "mh-editor.list" => encoded(editor.list(root,&a.path)?),
                    "mh-editor.read" => editor.read(root,&a.path),
                    "mh-editor.search" => editor.search(root,&a.query),
                    _ => editor.save(root,&a.path,a.version.as_deref().ok_or("Missing file version")?,a.text.as_deref().ok_or("Missing file text")?),
                }
            }
            "mh-lsp.start" | "mh-lsp.status" | "mh-lsp.stop" => {
                let a: LspSession = args(params)?;
                self.sessions.get(&a.session_id)?;
                if !harness.local { return Err("LSP is unavailable in this profile".into()); }
                let projects=self.plugins.context().try_service::<crate::workspaces::WorkspaceService>().map_err(|e|e.to_string())?;
                let project=projects.list()?.into_iter().find(|p|p.session_ids.contains(&a.session_id)).ok_or("Workspace not found")?;
                let root=project.path.as_ref().ok_or("Workspace directory is unavailable")?;
                self.plugins.sync_lsp().await?;
                let service=self.plugins.context().try_service::<crate::lsp::LspService>().map_err(|e|e.to_string())?;
                match method {
                    "mh-lsp.start" => service.start(&project.id,std::path::Path::new(root),&a.server_id).await?,
                    "mh-lsp.stop" => service.stop(&project.id,&a.server_id).await?,
                    _ => (),
                }
                Ok(serde_json::json!({"workspaceId":project.id,"running":service.running(&project.id,&a.server_id).await}))
            }
            "mh-lsp.open" | "mh-lsp.change" | "mh-lsp.close" | "mh-lsp.query" | "mh-lsp.diagnostics" => {
                let a: LspDocument = args(params)?;
                self.sessions.get(&a.session_id)?;
                if !harness.local { return Err("LSP is unavailable in this profile".into()); }
                let projects = self.plugins.context().try_service::<crate::workspaces::WorkspaceService>().map_err(|e| e.to_string())?;
                let project = projects.list()?.into_iter().find(|p| p.session_ids.contains(&a.session_id)).ok_or("Workspace not found")?;
                let root = std::path::Path::new(project.path.as_ref().ok_or("Workspace directory is unavailable")?);
                self.plugins.sync_lsp().await?;
                let service = self.plugins.context().try_service::<crate::lsp::LspService>().map_err(|e| e.to_string())?;
                match method {
                    "mh-lsp.open" => encoded(service.open_document(&project.id, root, &a.session_id, &a.path, a.version, a.text.as_deref().ok_or("Document text required")?).await?),
                    "mh-lsp.change" => { service.update_document(&project.id, root, &a.session_id, &a.path, a.version, Some(a.text.as_deref().ok_or("Document text required")?)).await?; Ok(Value::Null) },
                    "mh-lsp.close" => { service.update_document(&project.id, root, &a.session_id, &a.path, a.version, None).await?; Ok(Value::Null) },
                    "mh-lsp.query" => service.query_document(&project.id, root, &a.session_id, &a.path, a.operation.as_deref().ok_or("LSP operation required")?, a.line, a.character).await,
                    _ => {
                        encoded(service.document_diagnostics(&project.id, root, &a.session_id, &a.path).await?)
                    }
                }
            }
            "workspace.get" => {
                let a: Session = args(params)?;
                self.sessions.get(&a.session_id)?;
                encoded(harness.context.workspace_get(&a.session_id)?)
            }
            "workspace.save" => {
                let a: Workspace = args(params)?;
                self.sessions.get(&a.session_id)?;
                if !harness.local && a.settings.cwd.is_some() {
                    return Err("Local workspaces are unavailable in this profile".into());
                }
                if self.sessions.baseline(&a.session_id)?.active_run.is_some() {
                    return Err(
                        "Wait for the active run before changing its workspace or policy".into(),
                    );
                }
                encoded(harness.context.workspace_save(&a.session_id, a.settings)?)
            }
            "tools.list" => {
                let a: Session = args(params)?;
                self.sessions.get(&a.session_id)?;
                encoded(self.plugins.tools_for(&a.session_id).await?.definitions())
            }
            "interactions.list" => {
                let a: Session = args(params)?;
                self.sessions.get(&a.session_id)?;
                encoded(harness.interactions.list(&a.session_id))
            }
            "interactions.answer" => {
                let a: InteractionAnswer = args(params)?;
                self.sessions.get(&a.session_id)?;
                encoded(harness.interactions.answer(a))
            }
            "context.status" => {
                let a: Session = args(params)?;
                let messages = self.current_branch(&a.session_id)?;
                encoded(harness.context.prepare(&a.session_id, &messages)?.budget)
            }
            "context.compact" => {
                let a: Session = args(params)?;
                if self.sessions.baseline(&a.session_id)?.active_run.is_some() {
                    return Err("Cannot compact an active run".into());
                }
                encoded(self.compact_context(&a.session_id).await?)
            }
            "attachments.put" => {
                let a: Upload = args(params)?;
                self.sessions.get(&a.session_id)?;
                encoded(harness.context.put_attachment(
                    &a.session_id,
                    &a.name,
                    &a.mime_type,
                    &a.data,
                )?)
            }
            "attachments.get" => {
                let a: Attachment = args(params)?;
                self.sessions.get(&a.session_id)?;
                encoded(harness.context.get_attachment(&a.session_id, &a.id)?)
            }
            "mcp.list" => {
                let _: Empty = args(params)?;
                encoded(harness.mcp.list())
            }
            "mcp.save" => {
                let a: Mcp = args(params)?;
                encoded(harness.mcp.save(a.server).await?)
            }
            "mcp.delete" => {
                let a: Id = args(params)?;
                encoded(harness.mcp.delete(&a.id).await?)
            }
            _ => Err("Unknown harness operation".into()),
        }
    }
    pub fn current_branch(
        &self,
        session: &str,
    ) -> Result<Vec<crate::sessions::SessionMessage>, String> {
        self.sessions.get(session)?;
        let messages = self.sessions.load(session)?;
        match messages.last() {
            Some(last) => self.sessions.branch(session, &last.id),
            None => Ok(Vec::new()),
        }
    }
    pub async fn compact_context(&self, session: &str) -> Result<ContextSummary, String> {
        let selected = self
            .sessions
            .get(session)?
            .model
            .ok_or("Select a model before compacting history")?;
        let messages = self.current_branch(session)?;
        self.plugins
            .compact_messages(session, &selected, &messages)
            .await
    }
}
