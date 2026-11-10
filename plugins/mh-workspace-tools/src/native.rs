//! Native handlers constrain all filesystem paths to the selected canonical cwd.
use super::*;
use serde_json::Value;
use std::{path::PathBuf, sync::Mutex, time::Duration};
use std::{
    io::{Read, Write},
    path::{Component, Path},
};
const LIMIT: usize = 64 * 1024;
// Serialize read-modify-write edits so concurrent tool calls cannot lose updates.
pub static EDIT_LOCK: Mutex<()> = Mutex::new(());
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn string<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key} must be a string"))
}
fn contained(
    ctx: &ToolContext,
    path: &str,
    create: bool,
) -> Result<(cap_std::fs::Dir, PathBuf), String> {
    let root = ctx
        .cwd
        .as_ref()
        .ok_or("select a workspace directory first")?;
    let dir =
        cap_std::fs::Dir::open_ambient_dir(root, cap_std::ambient_authority()).map_err(error)?;
    let relative = Path::new(path);
    if path.is_empty()
        || relative.is_absolute()
        || path.contains(':')
        || !relative
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err("path must remain inside workspace".into());
    }
    let mut current = PathBuf::new();
    for component in relative.components() {
        current.push(component);
        match dir.symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err("symlink paths are not allowed".into())
            }
            Err(e) if !create || e.kind() != std::io::ErrorKind::NotFound => return Err(error(e)),
            _ => {}
        }
    }
    Ok((dir, relative.to_path_buf()))
}
pub fn atomic_write(dir: &cap_std::fs::Dir, path: &Path, content: &str) -> Result<(), String> {
    // Hold the parent capability across temp creation and rename, including when
    // another process renames a directory while this edit is running.
    let parent = dir
        .open_dir(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )
        .map_err(error)?;
    let name = path.file_name().ok_or("missing filename")?;
    let temporary = format!(
        ".minerva-edit-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(error)?
            .as_nanos()
    );
    let mut file = parent
        .open_with(
            &temporary,
            cap_std::fs::OpenOptions::new().write(true).create_new(true),
        )
        .map_err(error)?;
    struct Cleanup<'a>(&'a cap_std::fs::Dir, String);
    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            let _ = self.0.remove_file(&self.1);
        }
    }
    let _cleanup = Cleanup(&parent, temporary.clone());
    if let Ok(meta) = parent.metadata(name) {
        file.set_permissions(meta.permissions()).map_err(error)?;
    }
    file.write_all(content.as_bytes()).map_err(error)?;
    file.sync_all().map_err(error)?;
    drop(file);
    parent.rename(&temporary, &parent, name).map_err(error)?;
    Ok(())
}
fn read_prefix(dir: &cap_std::fs::Dir, path: &Path) -> Result<(String, bool), String> {
    let file = super::files::open_regular_file(dir, path).map_err(error)?;
    let mut bytes = Vec::new();
    file.take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    let truncated = bytes.len() > LIMIT;
    bytes.truncate(LIMIT);
    Ok((String::from_utf8_lossy(&bytes).into_owned(), truncated))
}
fn schema(properties: Value, required: Vec<&str>) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
pub fn save_attachment(ctx: &ToolContext, path: &str, data: &str) -> Result<Value, String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(error)?;
    let (dir, relative) = contained(ctx, path, true)?;
    let parent = dir
        .open_dir(
            relative
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )
        .map_err(error)?;
    let name = relative.file_name().ok_or("missing filename")?;
    if ctx.cancellation.is_cancelled() {
        return Err("tool cancelled".into());
    }
    // create_new protects existing workspace files and rejects a raced symlink.
    let mut file = parent
        .open_with(
            name,
            cap_std::fs::OpenOptions::new().write(true).create_new(true),
        )
        .map_err(error)?;
    if let Err(e) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = parent.remove_file(name);
        return Err(error(e));
    }
    Ok(json!({"path":path,"bytes":bytes.len()}))
}
pub fn register_native(registry: &ToolRegistry) -> Result<(), String> {
    let defs=[
        ("read_file","Read a UTF-8 workspace file",schema(json!({"path":{"type":"string"}}),vec!["path"]),false),
        ("search_files","Search literal text in workspace files",schema(json!({"path":{"type":"string"},"pattern":{"type":"string","minLength":1}}),vec!["pattern"]),false),
        ("write_file","Atomically write a UTF-8 workspace file",schema(json!({"path":{"type":"string"},"content":{"type":"string","maxLength":1048576}}),vec!["path","content"]),true),
        ("patch_file","Replace one exact unique substring in a workspace file",schema(json!({"path":{"type":"string"},"oldText":{"type":"string","minLength":1},"newText":{"type":"string"}}),vec!["path","oldText","newText"]),true),
        ("shell","Execute a shell command with cwd set to the workspace; this grants arbitrary local command execution",schema(json!({"command":{"type":"string","minLength":1,"maxLength":65536}}),vec!["command"]),true),
        ("ask_user","Ask the user a question and wait for an answer",schema(json!({"prompt":{"type":"string","minLength":1},"options":{"type":"array","items":{"type":"string"},"maxItems":32}}),vec!["prompt"]),false),
    ];
    for (name, description, input_schema, side_effect) in defs {
        registry.register(
            if name == "ask_user" {
                "native.interaction"
            } else {
                "native.workspace"
            },
            ToolDefinition {
                name: name.into(),
                description: description.into(),
                input_schema,
                output_schema: json!({"type":"object"}),
                side_effect,
            },
            move |ctx, args| Box::pin(async move { dispatch(name, ctx, args).await }),
        )?;
    }
    Ok(())
}
async fn dispatch(name: &str, ctx: ToolContext, args: Value) -> Result<Value, String> {
    if name == "ask_user" {
        let options = args["options"]
            .as_array()
            .map(|v| {
                v.iter()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();
        let answer = ctx
            .interactions
            .request(
                &ctx,
                "question",
                "Question",
                string(&args, "prompt")?,
                options,
            )
            .await?;
        return Ok(json!({"answer":answer}));
    }
    if name == "shell" {
        return shell(&ctx, &args).await;
    }
    let requested = args["path"].as_str().unwrap_or(".");
    let (dir, path) = contained(&ctx, requested, name == "write_file")?;
    match name {
        "read_file" => {
            let (content, truncated) = read_prefix(&dir, &path)?;
            Ok(json!({"path":requested,"content":content,"truncated":truncated}))
        }
        "write_file" => {
            let _edit = EDIT_LOCK.lock().map_err(error)?;
            let content = string(&args, "content")?;
            if ctx.cancellation.is_cancelled() {
                return Err("tool cancelled".into());
            }
            atomic_write(&dir, &path, content)?;
            Ok(json!({"path":requested,"bytes":content.len()}))
        }
        "patch_file" => {
            let _edit = EDIT_LOCK.lock().map_err(error)?;
            let old = string(&args, "oldText")?;
            let new = string(&args, "newText")?;
            let mut content = String::new();
            super::files::open_regular_file(&dir, &path)
                .map_err(error)?
                .take(1024 * 1024 + 1)
                .read_to_string(&mut content)
                .map_err(error)?;
            if content.len() > 1024 * 1024 {
                return Err("patch file exceeds 1 MiB".into());
            }
            if content.matches(old).count() != 1 {
                return Err("oldText must match exactly once; file was not changed".into());
            }
            let updated = content.replacen(old, new, 1);
            if updated.len() > 1024 * 1024 {
                return Err("patched file exceeds 1 MiB".into());
            }
            if ctx.cancellation.is_cancelled() {
                return Err("tool cancelled".into());
            }
            atomic_write(&dir, &path, &updated)?;
            Ok(
                json!({"path":requested,"diff":format!("--- {requested}\n+++ {requested}\n-{}\n+{}",old.chars().take(32768).collect::<String>(),new.chars().take(32768).collect::<String>()),"bytes":updated.len()}),
            )
        }
        "search_files" => {
            let pattern = string(&args, "pattern")?;

            let mut pending = vec![path];
            let mut matches = Vec::new();
            let mut scanned = 0;
            let mut truncated = false;
            while let Some(path) = pending.pop() {
                if ctx.cancellation.is_cancelled() {
                    return Err("tool cancelled".into());
                }
                scanned += 1;
                if scanned > 10000 {
                    truncated = true;
                    break;
                }
                let meta = dir.symlink_metadata(&path).map_err(error)?;
                if meta.file_type().is_symlink() {
                    continue;
                }
                if meta.is_dir() {
                    for entry in dir.read_dir(&path).map_err(error)? {
                        let entry = entry.map_err(error)?;
                        if entry.file_name() != ".git" {
                            pending.push(path.join(entry.file_name()));
                        }
                    }
                } else if meta.is_file() {
                    if let Ok((content, cut)) = read_prefix(&dir, &path) {
                        truncated |= cut;
                        for (index, line) in content.lines().enumerate() {
                            if line.contains(pattern) {
                                truncated |= line.chars().nth(500).is_some();
                                matches.push(json!({"path":path.to_string_lossy(),"line":index+1,"text":line.chars().take(500).collect::<String>()}));
                                if matches.len() >= 200 {
                                    truncated = true;
                                    break;
                                }
                            }
                        }
                    }
                }
                if matches.len() >= 200 {
                    break;
                }
            }
            Ok(json!({"matches":matches,"truncated":truncated}))
        }
        _ => Err("unknown native tool".into()),
    }
}
async fn shell(ctx: &ToolContext, args: &Value) -> Result<Value, String> {
    use tokio::io::AsyncReadExt;
    let cwd = ctx
        .cwd
        .as_ref()
        .ok_or("select a workspace directory first")?
        .canonicalize()
        .map_err(error)?;
    let command = string(args, "command")?;
    #[cfg(windows)]
    let mut process = {
        let mut c = tokio::process::Command::new("powershell.exe");
        c.args(["-NoProfile", "-NonInteractive", "-Command", "[Console]::InputEncoding=[Text.UTF8Encoding]::new($false); [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); & ([ScriptBlock]::Create([Console]::In.ReadToEnd()))"]);
        c.creation_flags(0x08000000);
        c
    };
    #[cfg(not(windows))]
    let mut process = {
        let mut c = tokio::process::Command::new("sh");
        c.args(["-c", command]);
        c.process_group(0);
        c
    };
    let mut child = process
        .current_dir(cwd)
        .stdin(if cfg!(windows) {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(error)?;
    let tree = super::process_tree::ProcessTree::attach(&child)?;
    #[cfg(windows)]
    {
        use tokio::io::AsyncWriteExt;
        let mut input = child.stdin.take().ok_or("missing shell stdin")?;
        input.write_all(command.as_bytes()).await.map_err(error)?;
        input.write_all(b"\n").await.map_err(error)?;
        input.shutdown().await.map_err(error)?;
    }
    let mut stdout = child.stdout.take().ok_or("missing stdout")?;
    let mut stderr = child.stderr.take().ok_or("missing stderr")?;
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut ob = [0u8; 4096];
    let mut eb = [0u8; 4096];
    let (mut out_open, mut err_open) = (true, true);
    let mut truncated = false;
    let mut ticker = tokio::time::interval(Duration::from_millis(150));
    let mut last_progress = (0, 0, false);
    let status = loop {
        tokio::select! {
            read=stdout.read(&mut ob),if out_open=>{let n=read.map_err(error)?;out_open=n!=0;append(&mut out,&ob[..n],&mut truncated)},
            read=stderr.read(&mut eb),if err_open=>{let n=read.map_err(error)?;err_open=n!=0;append(&mut err,&eb[..n],&mut truncated)},
            status=child.wait()=>{break status.map_err(error)?},
            _=ctx.cancellation.cancelled()=>{tree.kill();let _=child.kill().await;let _=child.wait().await;return Err("shell cancelled".into())},
            _=ticker.tick()=>{let progress=(out.len(),err.len(),truncated);if progress!=last_progress {if let Some(sink)=&ctx.sink{sink.emit(ctx,"shell",args,"running",Some(&Ok(json!({"stdout":String::from_utf8_lossy(&out),"stderr":String::from_utf8_lossy(&err),"truncated":truncated}))))?;}last_progress=progress;}}
        }
    };
    // Child exit can precede the last pipe read. Descendants holding handles are
    // terminated before bounded drain so calls never retain background processes.
    tree.kill();
    let drain = async {
        let mut rest = Vec::new();
        stdout
            .take((LIMIT + 1) as u64)
            .read_to_end(&mut rest)
            .await
            .map_err(error)?;
        append(&mut out, &rest, &mut truncated);
        rest.clear();
        stderr
            .take((LIMIT + 1) as u64)
            .read_to_end(&mut rest)
            .await
            .map_err(error)?;
        append(&mut err, &rest, &mut truncated);
        Ok::<_, String>(())
    };
    tokio::time::timeout(Duration::from_secs(2), drain)
        .await
        .map_err(|_| "shell pipes did not close".to_string())??;
    Ok(
        json!({"stdout":String::from_utf8_lossy(&out),"stderr":String::from_utf8_lossy(&err),"exitCode":status.code(),"truncated":truncated}),
    )
}
fn append(target: &mut Vec<u8>, bytes: &[u8], truncated: &mut bool) {
    let remaining = LIMIT.saturating_sub(target.len());
    *truncated |= bytes.len() > remaining;
    target.extend_from_slice(&bytes[..bytes.len().min(remaining)]);
}
