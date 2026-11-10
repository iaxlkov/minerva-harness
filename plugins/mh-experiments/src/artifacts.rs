use super::*;
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    path::Component,
};

const MAX_BLOB: usize = 8 * 1024 * 1024;
const MAX_SNAPSHOT: usize = 64 * 1024 * 1024;
const MAX_FILES: usize = 1024;
const MAX_JOURNAL: usize = 1024 * 1024;
fn hash(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
fn namespace(actor: &str, scope: &str) -> String {
    hash(&serde_json::to_vec(&(actor, scope)).expect("strings serialize"))
}
fn valid_path(path: &Path) -> bool {
    path.components().all(|c| match c {
        Component::Normal(name) => {
            let name = name.to_string_lossy().to_ascii_lowercase();
            !name.is_empty()
                && name != ".git"
                && name != ".env"
                && !name.starts_with(".env.")
                && name != ".ssh"
                && name != ".aws"
                && name != ".gnupg"
                && name != "credentials"
                && name != "secrets"
                && !name.ends_with(".pem")
                && !name.ends_with(".key")
                && !name.ends_with(".p12")
                && !name.ends_with(".pfx")
        }
        _ => false,
    }) && path.components().next().is_some()
}
fn read_source(root: &cap_std::fs::Dir, path: &Path) -> Result<Vec<u8>> {
    let file = mh_tool_runtime::files::open_regular_file(root, path)?;
    let mut data = Vec::new();
    file.take(MAX_BLOB as u64 + 1).read_to_end(&mut data)?;
    if data.len() > MAX_BLOB {
        return Err(Error::Invalid);
    }
    Ok(data)
}
impl Store {
    /// Host-only: the actor/scope must be authorized before calling this method.
    pub fn put_artifact(
        &self,
        actor: &str,
        scope: &str,
        job: &str,
        bytes: &[u8],
    ) -> Result<String> {
        if !id(actor) || !id(scope) || !id(job) || bytes.len() > MAX_BLOB {
            return Err(Error::Invalid);
        }
        self.get(actor, scope, job)?;
        let digest = hash(bytes);
        let dir = self.artifacts.join(namespace(actor, scope));
        std::fs::create_dir_all(&dir)?;
        let dest = dir.join(&digest);
        let mut file = tempfile::NamedTempFile::new_in(&dir)?;
        file.write_all(bytes)?;
        file.as_file().sync_all()?;
        if let Err(e) = file.persist_noclobber(&dest) {
            if e.error.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(Error::Io(e.error));
            }
            let mut data = Vec::new();
            std::fs::File::open(&dest)?
                .take(MAX_BLOB as u64 + 1)
                .read_to_end(&mut data)?;
            if data.len() > MAX_BLOB || hash(&data) != digest {
                return Err(Error::Corrupt);
            }
        }
        self.db.lock().map_err(|_| Error::Conflict)?.execute("INSERT OR IGNORE INTO artifact_refs SELECT actor,scope,id,?4,?5 FROM jobs WHERE actor=?1 AND scope=?2 AND id=?3",params![actor,scope,job,digest,bytes.len() as i64])?;
        Ok(digest)
    }
    pub fn artifact(&self, actor: &str, scope: &str, job: &str, digest: &str) -> Result<Vec<u8>> {
        if !id(actor) || !id(scope) || !id(job) || !super::digest(digest) {
            return Err(Error::Invalid);
        }
        let size: Option<i64> = self.db.lock().map_err(|_| Error::Conflict)?.query_row("SELECT bytes FROM artifact_refs WHERE actor=?1 AND scope=?2 AND job=?3 AND digest=?4",params![actor,scope,job,digest],|r|r.get(0)).optional()?;
        let size = size.ok_or(Error::NotFound)?;
        let file = std::fs::File::open(self.artifacts.join(namespace(actor, scope)).join(digest))?;
        let mut bytes = Vec::new();
        file.take(MAX_BLOB as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > MAX_BLOB || bytes.len() as i64 != size || hash(&bytes) != digest {
            return Err(Error::Corrupt);
        }
        Ok(bytes)
    }
    /// Snapshot exactly the host-approved tracked and untracked files; never executes a shell.
    pub fn snapshot(
        &self,
        actor: &str,
        scope: &str,
        job: &str,
        root: &Path,
        files: &[PathBuf],
    ) -> Result<String> {
        if !root.is_absolute() || files.len() > MAX_FILES {
            return Err(Error::Invalid);
        }
        self.get(actor, scope, job)?;
        let root = cap_std::fs::Dir::open_ambient_dir(root, cap_std::ambient_authority())?;
        let mut seen = BTreeSet::new();
        let mut total = 0usize;
        let mut manifest = Vec::new();
        for path in files {
            if !valid_path(path) || !seen.insert(path.clone()) {
                return Err(Error::Invalid);
            }
            let path_text = path.to_str().ok_or(Error::Invalid)?;
            let first = read_source(&root, path)?;
            total = total.checked_add(first.len()).ok_or(Error::Invalid)?;
            if total > MAX_SNAPSHOT {
                return Err(Error::Invalid);
            }
            let second = read_source(&root, path)?;
            if first != second {
                return Err(Error::Conflict);
            }
            let digest = self.put_artifact(actor, scope, job, &first)?;
            manifest
                .push(serde_json::json!({"path":path_text,"digest":digest,"bytes":first.len()}));
        }
        manifest.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
        let bytes = serde_json::to_vec(&serde_json::json!({"schemaVersion":1,"files":manifest}))
            .map_err(|_| Error::Invalid)?;
        self.put_artifact(actor, scope, job, &bytes)
    }
    /// Appends a bounded journal checkpoint with optimistic sequence and worker fencing.
    pub fn save_checkpoint(
        &self,
        actor: &str,
        scope: &str,
        job: &str,
        generation: i64,
        expected_seq: i64,
        journal: &str,
    ) -> Result<i64> {
        if journal.len() > MAX_JOURNAL
            || expected_seq < 0
            || !serde_json::from_str::<serde_json::Value>(journal).is_ok_and(|v| v.is_object())
        {
            return Err(Error::Invalid);
        }
        let mut db = self.db.lock().map_err(|_| Error::Conflict)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE actor=?1 AND scope=?2 AND id=?3 AND state='running' AND generation=?4)",params![actor,scope,job,generation],|r|r.get(0))?;
        if !exists {
            return Err(Error::Conflict);
        }
        let seq: i64 = tx.query_row(
            "SELECT COALESCE(MAX(seq),0) FROM checkpoints WHERE actor=?1 AND scope=?2 AND job=?3",
            params![actor, scope, job],
            |r| r.get(0),
        )?;
        if seq != expected_seq {
            return Err(Error::Conflict);
        }
        let next = seq.checked_add(1).ok_or(Error::Invalid)?;
        tx.execute(
            "INSERT INTO checkpoints VALUES (?1,?2,?3,?4,?5,?6)",
            params![actor, scope, job, next, generation, journal],
        )?;
        tx.commit()?;
        Ok(next)
    }
    pub fn load_checkpoint(
        &self,
        actor: &str,
        scope: &str,
        job: &str,
    ) -> Result<Option<(i64, String)>> {
        self.get(actor, scope, job)?;
        self.db.lock().map_err(|_| Error::Conflict)?.query_row("SELECT seq,journal FROM checkpoints WHERE actor=?1 AND scope=?2 AND job=?3 ORDER BY seq DESC LIMIT 1",params![actor,scope,job],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(Into::into)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (tempfile::TempDir, Store, i64) {
        let d = tempfile::tempdir().unwrap();
        let store = Store::open(d.path().join("db")).unwrap();
        let budget = Budget {
            max_calls: 4,
            max_input: 100,
            max_output: 100,
            max_concurrency: 1,
        };
        let hash = "a".repeat(64);
        store
            .create("actor", "workspace", "job", &hash, &budget)
            .unwrap();
        store
            .add_trial("actor", "workspace", "job", "trial", &hash)
            .unwrap();
        let generation = store.start("actor", "workspace", "job").unwrap();
        (d, store, generation)
    }
    #[test]
    fn artifact_scoping_and_corruption() {
        let (d, s, _) = setup();
        let digest = s
            .put_artifact("actor", "workspace", "job", b"data")
            .unwrap();
        assert_eq!(
            s.artifact("actor", "workspace", "job", &digest).unwrap(),
            b"data"
        );
        assert!(matches!(
            s.artifact("other", "workspace", "job", &digest),
            Err(Error::NotFound)
        ));
        let path = d
            .path()
            .join("artifacts")
            .join(namespace("actor", "workspace"))
            .join(&digest);
        std::fs::write(path, b"evil").unwrap();
        assert!(matches!(
            s.artifact("actor", "workspace", "job", &digest),
            Err(Error::Corrupt)
        ));
    }
    #[test]
    fn snapshot_keeps_changed_source_and_rejects_sensitive_paths() {
        let (d, s, _) = setup();
        let root = d.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("file"), b"first").unwrap();
        let first = s
            .snapshot("actor", "workspace", "job", &root, &["file".into()])
            .unwrap();
        std::fs::write(root.join("file"), b"second").unwrap();
        let second = s
            .snapshot("actor", "workspace", "job", &root, &["file".into()])
            .unwrap();
        assert_ne!(first, second);
        let manifest: serde_json::Value =
            serde_json::from_slice(&s.artifact("actor", "workspace", "job", &first).unwrap())
                .unwrap();
        assert_eq!(
            s.artifact(
                "actor",
                "workspace",
                "job",
                manifest["files"][0]["digest"].as_str().unwrap()
            )
            .unwrap(),
            b"first"
        );
        assert!(matches!(
            s.snapshot("actor", "workspace", "job", &root, &["../file".into()]),
            Err(Error::Invalid)
        ));
        assert!(matches!(
            s.snapshot("actor", "workspace", "job", &root, &[".env".into()]),
            Err(Error::Invalid)
        ));
    }
    #[test]
    fn checkpoint_survives_recovery_and_fences_previous_worker() {
        let (d, s, g) = setup();
        let journal = r#"{"pending":true,"records":[]}"#;
        assert_eq!(
            s.save_checkpoint("actor", "workspace", "job", g, 0, journal)
                .unwrap(),
            1
        );
        assert!(matches!(
            s.save_checkpoint("actor", "workspace", "job", g, 0, journal),
            Err(Error::Conflict)
        ));
        s.recover().unwrap();
        drop(s);
        let s = Store::open(d.path().join("db")).unwrap();
        assert_eq!(
            s.load_checkpoint("actor", "workspace", "job").unwrap(),
            Some((1, journal.into()))
        );
        assert!(matches!(
            s.save_checkpoint("actor", "workspace", "job", g, 1, journal),
            Err(Error::Conflict)
        ));
        assert!(matches!(
            s.load_checkpoint("other", "workspace", "job"),
            Err(Error::NotFound)
        ));
    }
    #[cfg(unix)]
    #[test]
    fn snapshot_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;
        let (d, s, _) = setup();
        let root = d.path().join("root");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(d.path().join("secret"), "secret").unwrap();
        symlink(d.path().join("secret"), root.join("link")).unwrap();
        assert!(s
            .snapshot("actor", "workspace", "job", &root, &["link".into()])
            .is_err());
    }
}
