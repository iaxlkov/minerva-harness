use minerva_plugin_sdk::{
    experience::*,
    learning::{valid_digest, AccessContext, LearningError, Scope},
};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, LearningError>;
fn err(code: &str, message: impl std::fmt::Display) -> LearningError {
    LearningError::new(code, message.to_string())
}
fn io(e: impl std::fmt::Display) -> LearningError {
    err("STORAGE_ERROR", e)
}
fn digest(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
fn json<T: serde::Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(io)
}
fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 512
        && !path.contains('\\')
        && !path.contains(':')
        && Path::new(path)
            .components()
            .all(|p| matches!(p, Component::Normal(_)))
        && path != "manifest.json"
}
fn scoped_read(dir: &cap_std::fs::Dir, path: &Path, max: usize) -> Result<Vec<u8>> {
    let file = dir.open(path).map_err(io)?;
    if !file.metadata().map_err(io)?.is_file() {
        return Err(err("CORRUPT_BUNDLE", "not a regular file"));
    }
    let mut data = Vec::new();
    file.take(max as u64 + 1)
        .read_to_end(&mut data)
        .map_err(io)?;
    if data.len() > max {
        return Err(err("CORRUPT_BUNDLE", "file too large"));
    }
    Ok(data)
}
/// The host verifies completed experiment, exact evidence links, contrast and regression gate.
pub trait EvidenceVerifier: Send + Sync {
    fn verify(
        &self,
        access: &AccessContext,
        publication: &PublishExperience,
        manifest: &BundleManifest,
    ) -> Result<EvidenceAssessment>;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvidenceAssessment {
    Provisional,
    Validated,
}
pub struct UnavailableVerifier;
impl EvidenceVerifier for UnavailableVerifier {
    fn verify(
        &self,
        _: &AccessContext,
        _: &PublishExperience,
        _: &BundleManifest,
    ) -> Result<EvidenceAssessment> {
        Err(err(
            "EVIDENCE_UNAVAILABLE",
            "trusted evidence verifier unavailable",
        ))
    }
}
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperienceIndex {
    pub id: String,
    pub domain: String,
    pub cue: String,
    pub bundle_digest: String,
    pub status: ExperienceStatus,
    pub revision: u64,
}
pub struct ExperienceService {
    root: PathBuf,
    db: Mutex<Connection>,
    verifier: Arc<dyn EvidenceVerifier>,
}
impl ExperienceService {
    pub fn open(root: impl AsRef<Path>, verifier: Arc<dyn EvidenceVerifier>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("bundles")).map_err(io)?;
        let db = Connection::open(root.join("experience.sqlite3")).map_err(io)?;
        let version: i64 = db
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(io)?;
        if version > 1 {
            return Err(err("SCHEMA_UNSUPPORTED", "newer experience schema"));
        }
        if version == 0 {
            db.execute_batch("BEGIN IMMEDIATE; CREATE TABLE experience (id TEXT NOT NULL, actor TEXT NOT NULL, scope TEXT NOT NULL, body TEXT NOT NULL, status TEXT NOT NULL, revision INTEGER NOT NULL, PRIMARY KEY(actor,id)); CREATE INDEX experience_scope ON experience(actor,scope,status); PRAGMA user_version=1; COMMIT;").map_err(io)?;
        }
        Ok(Self {
            root,
            db: Mutex::new(db),
            verifier,
        })
    }
    pub fn bundle_digest(manifest: &BundleManifest) -> Result<String> {
        let mut canonical = manifest.clone();
        canonical.digest.clear();
        Ok(digest(&json(&canonical)?))
    }
    fn validate_manifest(
        manifest: &BundleManifest,
        files: &BTreeMap<String, Vec<u8>>,
    ) -> Result<()> {
        if manifest.schema_version != 1
            || !valid_digest(&manifest.digest)
            || Self::bundle_digest(manifest)? != manifest.digest
            || manifest
                .parent_digest
                .as_ref()
                .is_some_and(|d| !valid_digest(d))
            || manifest.components.len() > 128
            || manifest.source_refs.len() > 128
        {
            return Err(err("INVALID_BUNDLE", "invalid manifest"));
        }
        for source in &manifest.source_refs {
            source.validate()?;
        }
        let mut seen = HashSet::new();
        let mut total = 0usize;
        for c in &manifest.components {
            if !safe_path(&c.path)
                || !seen.insert(&c.path)
                || c.adapter_version.is_empty()
                || c.adapter_version.len() > 128
                || !valid_digest(&c.sha256)
            {
                return Err(err("INVALID_BUNDLE", "invalid component"));
            }
            let data = files
                .get(&c.path)
                .ok_or_else(|| err("INVALID_BUNDLE", "missing component"))?;
            total = total
                .checked_add(data.len())
                .ok_or_else(|| err("INVALID_BUNDLE", "size overflow"))?;
            if total > 16 * 1024 * 1024 || data.len() as u64 != c.bytes || digest(data) != c.sha256
            {
                return Err(err("INVALID_BUNDLE", "component digest or size mismatch"));
            }
        }
        if files.len() != manifest.components.len() {
            return Err(err("INVALID_BUNDLE", "unexpected component"));
        }
        Ok(())
    }
    pub fn store_bundle(
        &self,
        access: &AccessContext,
        manifest: &BundleManifest,
        files: &BTreeMap<String, Vec<u8>>,
    ) -> Result<()> {
        access.authorize(&manifest.scope)?;
        Self::validate_manifest(manifest, files)?;
        let root = cap_std::fs::Dir::open_ambient_dir(&self.root, cap_std::ambient_authority())
            .map_err(io)?;
        let base_path = PathBuf::from("bundles").join(digest(access.actor_id.as_bytes()));
        root.create_dir_all(&base_path).map_err(io)?;
        let base = root.open_dir(&base_path).map_err(io)?;
        if base.try_exists(&manifest.digest).map_err(io)? {
            let (existing, data) = self.read_bundle(access, &manifest.scope, &manifest.digest)?;
            if existing != *manifest || data != *files {
                return Err(err("CORRUPT_BUNDLE", "existing bundle differs"));
            }
            return Ok(());
        }
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io)?
            .as_nanos();
        let stage = format!(".stage-{}-{nonce}", std::process::id());
        base.create_dir(&stage).map_err(io)?;
        let result = (|| {
            let staging = base.open_dir(&stage).map_err(io)?;
            for (path, data) in files {
                let target = Path::new(path);
                if let Some(parent) = target.parent() {
                    if !parent.as_os_str().is_empty() {
                        staging.create_dir_all(parent).map_err(io)?;
                    }
                }
                staging.write(target, data).map_err(io)?;
            }
            staging
                .write("manifest.json", json(manifest)?)
                .map_err(io)?;
            drop(staging);
            base.rename(&stage, &base, &manifest.digest).map_err(io)
        })();
        if result.is_err() {
            let _ = base.remove_dir_all(&stage);
        }
        result
    }
    pub fn read_bundle(
        &self,
        access: &AccessContext,
        scope: &Scope,
        id: &str,
    ) -> Result<(BundleManifest, BTreeMap<String, Vec<u8>>)> {
        access.authorize(scope)?;
        if !valid_digest(id) {
            return Err(err("INVALID_INPUT", "invalid bundle digest"));
        }
        let root = cap_std::fs::Dir::open_ambient_dir(&self.root, cap_std::ambient_authority())
            .map_err(io)?;
        let relative = PathBuf::from("bundles")
            .join(digest(access.actor_id.as_bytes()))
            .join(id);
        let dir = root.open_dir(relative).map_err(io)?;
        let raw = scoped_read(&dir, Path::new("manifest.json"), 1024 * 1024)?;
        let manifest: BundleManifest =
            serde_json::from_slice(&raw).map_err(|e| err("CORRUPT_BUNDLE", e))?;
        if &manifest.scope != scope || manifest.digest != id || manifest.components.len() > 128 {
            return Err(err("NOT_FOUND_OR_FORBIDDEN", "bundle unavailable"));
        }
        let mut files = BTreeMap::new();
        let mut total = 0u64;
        for c in &manifest.components {
            if !safe_path(&c.path) || c.bytes > 16 * 1024 * 1024 {
                return Err(err("CORRUPT_BUNDLE", "invalid component"));
            }
            total = total
                .checked_add(c.bytes)
                .ok_or_else(|| err("CORRUPT_BUNDLE", "size overflow"))?;
            if total > 16 * 1024 * 1024 {
                return Err(err("CORRUPT_BUNDLE", "bundle too large"));
            }
            let data = scoped_read(&dir, Path::new(&c.path), c.bytes as usize)?;
            if data.len() as u64 != c.bytes {
                return Err(err("CORRUPT_BUNDLE", "invalid file size"));
            }
            files.insert(c.path.clone(), data);
        }
        Self::validate_manifest(&manifest, &files).map_err(|e| err("CORRUPT_BUNDLE", e))?;
        Ok((manifest, files))
    }
    pub fn publish(&self, access: &AccessContext, p: PublishExperience) -> Result<Experience> {
        access.authorize(&p.scope)?;
        if [
            &p.id,
            &p.experiment_id,
            &p.domain,
            &p.cue,
            &p.lesson,
            &p.adoption_reason,
            &p.limitations,
            &p.confidence,
        ]
        .iter()
        .any(|s| s.trim().is_empty() || s.len() > 8192)
            || p.evidence_refs.is_empty()
            || p.evidence_refs.len() > 128
            || p.evidence_refs
                .iter()
                .any(|e| e.artifact_id.is_empty() || !valid_digest(&e.digest))
        {
            return Err(err("INVALID_INPUT", "invalid experience"));
        }
        let (manifest, _) = self.read_bundle(access, &p.scope, &p.bundle_digest)?;
        let status = match self.verifier.verify(access, &p, &manifest)? {
            EvidenceAssessment::Provisional => ExperienceStatus::Provisional,
            EvidenceAssessment::Validated => ExperienceStatus::Validated,
        };
        let item = Experience {
            publication: p,
            status,
            revision: 1,
        };
        let scope = String::from_utf8(json(&item.publication.scope)?).map_err(io)?;
        let body = String::from_utf8(json(&item)?).map_err(io)?;
        self.db
            .lock()
            .map_err(io)?
            .execute(
                "INSERT INTO experience VALUES (?1,?2,?3,?4,?5,1)",
                params![
                    item.publication.id,
                    access.actor_id,
                    scope,
                    body,
                    format!("{:?}", item.status)
                ],
            )
            .map_err(io)?;
        Ok(item)
    }
    pub fn get(&self, access: &AccessContext, scope: &Scope, id: &str) -> Result<Experience> {
        access.authorize(scope)?;
        let scope = String::from_utf8(json(scope)?).map_err(io)?;
        let body: Option<String> = self
            .db
            .lock()
            .map_err(io)?
            .query_row(
                "SELECT body FROM experience WHERE actor=?1 AND scope=?2 AND id=?3",
                params![access.actor_id, scope, id],
                |r| r.get(0),
            )
            .optional()
            .map_err(io)?;
        serde_json::from_str(
            &body.ok_or_else(|| err("NOT_FOUND_OR_FORBIDDEN", "experience unavailable"))?,
        )
        .map_err(io)
    }
    pub fn search(
        &self,
        access: &AccessContext,
        scope: &Scope,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ExperienceIndex>> {
        access.authorize(scope)?;
        if query.len() > 512 || limit == 0 || limit > 100 {
            return Err(err("INVALID_INPUT", "invalid search"));
        }
        let scope = String::from_utf8(json(scope)?).map_err(io)?;
        let db = self.db.lock().map_err(io)?;
        let mut stmt = db.prepare("SELECT body FROM experience WHERE actor=?1 AND scope=?2 AND status != 'Retired' AND instr(lower(json_extract(body,'$.cue')),lower(?3)) > 0 ORDER BY id LIMIT ?4").map_err(io)?;
        let rows = stmt
            .query_map(params![access.actor_id, scope, query, limit as i64], |r| {
                r.get::<_, String>(0)
            })
            .map_err(io)?;
        let mut result = Vec::new();
        for row in rows {
            let item: Experience = serde_json::from_str(&row.map_err(io)?).map_err(io)?;
            result.push(ExperienceIndex {
                id: item.publication.id,
                domain: item.publication.domain,
                cue: item.publication.cue,
                bundle_digest: item.publication.bundle_digest,
                status: item.status,
                revision: item.revision,
            });
        }
        Ok(result)
    }
    pub fn retire(&self, access: &AccessContext, scope: &Scope, id: &str) -> Result<Experience> {
        let mut item = self.get(access, scope, id)?;
        if item.status == ExperienceStatus::Retired {
            return Ok(item);
        }
        let previous = item.revision;
        item.revision += 1;
        item.status = ExperienceStatus::Retired;
        let scope = String::from_utf8(json(scope)?).map_err(io)?;
        let body = String::from_utf8(json(&item)?).map_err(io)?;
        let changed = self.db.lock().map_err(io)?.execute("UPDATE experience SET body=?1,status='Retired',revision=?2 WHERE actor=?3 AND scope=?4 AND id=?5 AND revision=?6", params![body,item.revision,access.actor_id,scope,id,previous]).map_err(io)?;
        if changed != 1 {
            return Err(err("CONFLICT", "experience changed"));
        }
        Ok(item)
    }
    pub fn export(&self, access: &AccessContext, scope: &Scope, id: &str) -> Result<String> {
        let item = self.get(access, scope, id)?;
        Ok(format!(
            "# Experience\n\n```json\n{}\n```\n",
            serde_json::to_string_pretty(&item).map_err(io)?
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minerva_plugin_sdk::learning::ArtifactRef;
    struct Verified;
    impl EvidenceVerifier for Verified {
        fn verify(
            &self,
            _: &AccessContext,
            p: &PublishExperience,
            _: &BundleManifest,
        ) -> Result<EvidenceAssessment> {
            if p.experiment_id == "completed" {
                Ok(EvidenceAssessment::Validated)
            } else {
                Err(err("INVALID_EVIDENCE", "experiment incomplete"))
            }
        }
    }
    fn access(actor: &str) -> AccessContext {
        AccessContext {
            actor_id: actor.into(),
            session_id: "s".into(),
            workspace_id: Some("w".into()),
            allow_user_scope: false,
        }
    }
    fn bundle() -> (BundleManifest, BTreeMap<String, Vec<u8>>) {
        let mut files = BTreeMap::new();
        files.insert("skills/test.md".into(), b"lesson".to_vec());
        let mut m = BundleManifest {
            schema_version: 1,
            digest: String::new(),
            parent_digest: None,
            components: vec![BundleComponent {
                kind: ComponentKind::Skills,
                path: "skills/test.md".into(),
                sha256: digest(b"lesson"),
                bytes: 6,
                adapter_version: "1".into(),
            }],
            scope: Scope::Workspace {
                workspace_id: "w".into(),
            },
            source_refs: vec![],
        };
        m.digest = ExperienceService::bundle_digest(&m).unwrap();
        (m, files)
    }
    fn publication(m: &BundleManifest) -> PublishExperience {
        PublishExperience {
            id: "x".into(),
            experiment_id: "completed".into(),
            domain: "coding".into(),
            scope: m.scope.clone(),
            cue: "compiler".into(),
            lesson: "check errors".into(),
            evidence_refs: vec![ArtifactRef {
                artifact_id: "e".into(),
                digest: "a".repeat(64),
                media_type: "text/plain".into(),
                bytes: 1,
            }],
            bundle_digest: m.digest.clone(),
            adoption_reason: "better result".into(),
            limitations: "rust only".into(),
            confidence: "measured".into(),
        }
    }
    #[test]
    fn lifecycle_and_isolation() {
        let dir = tempfile::tempdir().unwrap();
        let svc = ExperienceService::open(dir.path(), Arc::new(Verified)).unwrap();
        let (m, files) = bundle();
        let a = access("a");
        svc.store_bundle(&a, &m, &files).unwrap();
        assert!(svc.publish(&a, publication(&m)).is_ok());
        assert_eq!(svc.search(&a, &m.scope, "compiler", 10).unwrap().len(), 1);
        assert!(svc.get(&access("b"), &m.scope, "x").is_err());
        svc.retire(&a, &m.scope, "x").unwrap();
        assert!(svc.search(&a, &m.scope, "", 10).unwrap().is_empty());
        assert!(svc.export(&a, &m.scope, "x").unwrap().contains("retired"));
    }
    #[test]
    fn reject_corruption_and_untrusted_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let svc = ExperienceService::open(dir.path(), Arc::new(UnavailableVerifier)).unwrap();
        let (m, files) = bundle();
        let a = access("a");
        svc.store_bundle(&a, &m, &files).unwrap();
        assert_eq!(
            svc.publish(&a, publication(&m)).unwrap_err().code,
            "EVIDENCE_UNAVAILABLE"
        );
        let path = dir
            .path()
            .join("bundles")
            .join(digest(b"a"))
            .join(&m.digest)
            .join("skills/test.md");
        fs::write(path, b"broken").unwrap();
        assert!(svc.read_bundle(&a, &m.scope, &m.digest).is_err());
        let mut bad = m.clone();
        bad.components[0].path = "../escape".into();
        bad.digest = ExperienceService::bundle_digest(&bad).unwrap();
        assert!(svc.store_bundle(&a, &bad, &files).is_err());
    }
    #[test]
    fn reject_scope_escape_and_incomplete_experiment() {
        let dir = tempfile::tempdir().unwrap();
        let svc = ExperienceService::open(dir.path(), Arc::new(Verified)).unwrap();
        let (m, files) = bundle();
        let a = access("a");
        svc.store_bundle(&a, &m, &files).unwrap();
        let mut other = a.clone();
        other.workspace_id = Some("other".into());
        assert!(svc.read_bundle(&other, &m.scope, &m.digest).is_err());
        let mut p = publication(&m);
        p.experiment_id = "running".into();
        assert_eq!(svc.publish(&a, p).unwrap_err().code, "INVALID_EVIDENCE");
    }
    #[cfg(unix)]
    #[test]
    fn reject_symlink_escape() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let svc = ExperienceService::open(dir.path(), Arc::new(Verified)).unwrap();
        let (m, files) = bundle();
        let a = access("a");
        svc.store_bundle(&a, &m, &files).unwrap();
        let path = dir
            .path()
            .join("bundles")
            .join(digest(b"a"))
            .join(&m.digest)
            .join("skills");
        fs::remove_dir_all(&path).unwrap();
        fs::write(outside.path().join("test.md"), b"lesson").unwrap();
        symlink(outside.path(), path).unwrap();
        assert!(svc.read_bundle(&a, &m.scope, &m.digest).is_err());
    }
}
