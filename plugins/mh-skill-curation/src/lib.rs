//! Durable, fail-closed skill proposals. Verifiers and publishers are host capabilities.
use minerva_plugin_sdk::learning::{AccessContext, LearningError, Scope, SourceRef};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Mutex};

type Result<T> = std::result::Result<T, LearningError>;
fn error(code: &str, message: impl ToString) -> LearningError {
    LearningError::new(code, message.to_string())
}
fn storage(e: impl ToString) -> LearningError {
    error("STORAGE", e)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub id: String,
    pub scope: Scope,
    pub actor: String,
    pub sources: Vec<SourceRef>,
    pub skill: String,
    pub base_revision: String,
    pub candidate: String,
    pub diff: String,
    pub rationale: String,
    pub applicability: String,
    pub negative_example: String,
    pub validators: Vec<String>,
    pub reviewers: Vec<String>,
    pub state: State,
    pub evidence: Vec<Verdict>,
    pub published_revision: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Draft,
    Evaluated,
    Evaluating,
    Ready,
    PendingPublish,
    Published,
    Rejected,
    Stale,
    Failed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verdict {
    pub id: String,
    pub passed: bool,
    pub evidence: String,
}
/// The host supplies independent, preconfigured validators; model output cannot invoke this capability.
pub trait Verifier {
    fn verify(&self, proposal: &Proposal) -> Result<Vec<Verdict>>;
}
pub trait SkillsPublisher {
    /// Atomically check base and publish; return CONFLICT if base drifted.
    fn publish(&self, actor: &AccessContext, proposal: &Proposal) -> Result<String>;
    fn rollback(&self, actor: &AccessContext, proposal: &Proposal, expected: &str) -> Result<()>;
}
/// Recovery is a trusted host operation after the original publisher has stopped.
/// It must inspect authoritative publication audit, not infer success from model output.
pub trait PublicationRecovery {
    fn resolve(&self, actor: &AccessContext, proposal: &Proposal) -> Result<RecoveryOutcome>;
}
pub enum RecoveryOutcome {
    Published(String),
    Stale,
    Failed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidationCase {
    pub input: String,
    pub contains: Vec<String>,
    pub not_contains: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GateConfig {
    pub executor_model: String,
    pub reviewers: [String; 2],
    pub trigger: ValidationCase,
    pub holdouts: Vec<ValidationCase>,
    pub max_calls: u32,
}
impl GateConfig {
    pub fn validate(&self) -> Result<()> {
        let provider = |model: &str| {
            model
                .strip_prefix("api:")
                .and_then(|s| s.split_once('/'))
                .filter(|(p, m)| !p.is_empty() && !m.is_empty())
                .map(|(p, _)| p.to_owned())
        };
        let executor = provider(&self.executor_model);
        let reviewers = [provider(&self.reviewers[0]), provider(&self.reviewers[1])];
        let cases = std::iter::once(&self.trigger).chain(self.holdouts.iter());
        if executor.is_none()
            || reviewers[0].is_none()
            || reviewers[1].is_none()
            || reviewers[0] == reviewers[1]
            || self.executor_model.is_empty()
            || self.reviewers.iter().any(|m| m.is_empty() || m.len() > 256)
            || self.reviewers[0] == self.reviewers[1]
            || self.holdouts.is_empty()
            || self.holdouts.len() > 20
            || self.max_calls < (2 * (self.holdouts.len() as u32 + 1) + 2)
            || self.max_calls > 100
            || cases.into_iter().any(|c| {
                c.input.trim().is_empty()
                    || c.input.len() > 8192
                    || c.contains.len() + c.not_contains.len() == 0
                    || c.contains.len() + c.not_contains.len() > 32
                    || c.contains
                        .iter()
                        .chain(&c.not_contains)
                        .any(|v| v.is_empty() || v.len() > 1024)
            })
        {
            return Err(error(
                "INVALID_CONFIG",
                "configure bounded trigger, holdouts, distinct reviewers and budget",
            ));
        }
        Ok(())
    }
}
pub struct Curation {
    operation: Mutex<()>,
    db: Mutex<Connection>,
    validators: Vec<String>,
    reviewers: Vec<String>,
}
impl Curation {
    pub fn open(path: &Path, validators: Vec<String>, reviewers: Vec<String>) -> Result<Self> {
        if validators.is_empty()
            || reviewers.len() != 2
            || reviewers[0] == reviewers[1]
            || validators.iter().chain(&reviewers).any(|s| s.is_empty())
        {
            return Err(error(
                "INVALID_CONFIG",
                "validators and two independent reviewer configurations required",
            ));
        }
        let mut ids = validators.clone();
        ids.extend(reviewers.clone());
        ids.sort();
        ids.dedup();
        if ids.len() != validators.len() + reviewers.len() {
            return Err(error("INVALID_CONFIG", "duplicate verifier configuration"));
        }
        let db = Connection::open(path).map_err(storage)?;
        let version: i64 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(storage)?;
        if version > 1 {
            return Err(error("SCHEMA", "unsupported schema version"));
        }
        db.execute_batch("BEGIN; CREATE TABLE IF NOT EXISTS proposals (id TEXT PRIMARY KEY, data TEXT NOT NULL); CREATE TABLE IF NOT EXISTS gate_configs (actor TEXT NOT NULL,scope TEXT NOT NULL,config TEXT NOT NULL, PRIMARY KEY(actor,scope)); CREATE TABLE IF NOT EXISTS proposal_gates (proposal TEXT PRIMARY KEY, config TEXT NOT NULL); PRAGMA user_version=1; COMMIT;").map_err(storage)?;
        Ok(Self {
            operation: Mutex::new(()),
            db: Mutex::new(db),
            validators,
            reviewers,
        })
    }
    pub fn configure_gate(
        &self,
        host: &AccessContext,
        scope: &Scope,
        config: &GateConfig,
    ) -> Result<()> {
        host.authorize(scope)?;
        config.validate()?;
        let scope = serde_json::to_string(scope).map_err(storage)?;
        let config = serde_json::to_string(config).map_err(storage)?;
        self.db.lock().map_err(storage)?.execute("INSERT INTO gate_configs VALUES (?1,?2,?3) ON CONFLICT(actor,scope) DO UPDATE SET config=excluded.config", params![host.actor_id,scope,config]).map_err(storage)?;
        Ok(())
    }
    pub fn gate_for(&self, host: &AccessContext, id: &str) -> Result<GateConfig> {
        self.get(host, id)?;
        let db = self.db.lock().map_err(storage)?;
        let raw: Option<String> = db
            .query_row(
                "SELECT config FROM proposal_gates WHERE proposal=?1",
                [id],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)?;
        let gate: GateConfig = serde_json::from_str(&raw.ok_or_else(|| {
            error(
                "CONFIG_REQUIRED",
                "configure validation cases before proposing",
            )
        })?)
        .map_err(storage)?;
        gate.validate()?;
        Ok(gate)
    }
    fn save(&self, p: &Proposal, old: Option<&Proposal>) -> Result<()> {
        let json = serde_json::to_string(p).map_err(storage)?;
        let db = self.db.lock().map_err(storage)?;
        if let Some(old) = old {
            let previous = serde_json::to_string(old).map_err(storage)?;
            let changed = db
                .execute(
                    "UPDATE proposals SET data=?2 WHERE id=?1 AND data=?3",
                    params![p.id, json, previous],
                )
                .map_err(storage)?;
            if changed != 1 {
                return Err(error("CONFLICT", "proposal changed concurrently"));
            }
        } else {
            db.execute("INSERT INTO proposals VALUES (?1,?2)", params![p.id, json])
                .map_err(storage)?;
        }
        Ok(())
    }
    pub fn propose(&self, host: &AccessContext, mut p: Proposal) -> Result<()> {
        host.authorize(&p.scope)?;
        if p.id.is_empty()
            || p.id.len() > 128
            || p.skill.is_empty()
            || p.base_revision.is_empty()
            || p.candidate.is_empty()
            || p.candidate.len() > 1_000_000
            || p.diff.is_empty()
            || p.rationale.is_empty()
            || p.applicability.is_empty()
            || p.negative_example.is_empty()
            || p.sources.is_empty()
            || p.sources.len() > 100
        {
            return Err(error("INVALID_INPUT", "incomplete proposal"));
        }
        for s in &p.sources {
            s.validate()?;
        }
        p.actor = host.actor_id.clone();
        p.validators = self.validators.clone();
        p.reviewers = self.reviewers.clone();
        p.evidence.clear();
        p.published_revision = None;
        p.state = State::Draft;
        let mut db = self.db.lock().map_err(storage)?;
        let tx = db.transaction().map_err(storage)?;
        let scope = serde_json::to_string(&p.scope).map_err(storage)?;
        let gate: Option<String> = tx
            .query_row(
                "SELECT config FROM gate_configs WHERE actor=?1 AND scope=?2",
                params![host.actor_id, scope],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)?;
        let body = serde_json::to_string(&p).map_err(storage)?;
        tx.execute("INSERT INTO proposals VALUES (?1,?2)", params![p.id, body])
            .map_err(storage)?;
        if let Some(gate) = gate {
            tx.execute(
                "INSERT INTO proposal_gates VALUES (?1,?2)",
                params![p.id, gate],
            )
            .map_err(storage)?;
        }
        tx.commit().map_err(storage)?;
        Ok(())
    }
    pub fn get(&self, host: &AccessContext, id: &str) -> Result<Proposal> {
        let db = self.db.lock().map_err(storage)?;
        let data: Option<String> = db
            .query_row("SELECT data FROM proposals WHERE id=?1", [id], |r| r.get(0))
            .optional()
            .map_err(storage)?;
        let p: Proposal = serde_json::from_str(
            &data.ok_or_else(|| error("NOT_FOUND_OR_FORBIDDEN", "proposal unavailable"))?,
        )
        .map_err(storage)?;
        host.authorize(&p.scope)?;
        if p.actor != host.actor_id {
            return Err(error("NOT_FOUND_OR_FORBIDDEN", "proposal unavailable"));
        }
        Ok(p)
    }
    pub fn list(&self, host: &AccessContext, scope: &Scope) -> Result<Vec<Proposal>> {
        host.authorize(scope)?;
        let db = self.db.lock().map_err(storage)?;
        let mut stmt = db.prepare("SELECT data FROM proposals WHERE json_extract(data,'$.actor')=?1 AND json_extract(data,'$.scope')=json(?2) ORDER BY id LIMIT 256").map_err(storage)?;
        let rows = stmt
            .query_map(
                params![
                    host.actor_id,
                    serde_json::to_string(scope).map_err(storage)?
                ],
                |r| r.get::<_, String>(0),
            )
            .map_err(storage)?;
        rows.map(|r| serde_json::from_str(&r.map_err(storage)?).map_err(storage))
            .collect()
    }
    /// Persist admission before any model call; interrupted evaluations cannot be replayed automatically.
    pub fn begin_evaluation(
        &self,
        host: &AccessContext,
        id: &str,
    ) -> Result<(Proposal, GateConfig)> {
        let _guard = self.operation.lock().map_err(storage)?;
        let mut p = self.get(host, id)?;
        if p.state != State::Draft {
            return Err(error("INVALID_STATE", "expected draft"));
        }
        let gate = self.gate_for(host, id)?;
        let old = p.clone();
        p.state = State::Evaluating;
        self.save(&p, Some(&old))?;
        Ok((p, gate))
    }
    pub fn finish_evaluation(
        &self,
        host: &AccessContext,
        id: &str,
        verdicts: Vec<Verdict>,
    ) -> Result<Proposal> {
        let _guard = self.operation.lock().map_err(storage)?;
        let mut p = self.get(host, id)?;
        if p.state != State::Evaluating {
            return Err(error("INVALID_STATE", "evaluation not active"));
        }
        let old = p.clone();
        let mut required = p.validators.clone();
        required.extend(p.reviewers.clone());
        let valid = verdicts.len() == required.len()
            && required.iter().all(|id| {
                verdicts
                    .iter()
                    .filter(|v| &v.id == id && !v.evidence.trim().is_empty())
                    .count()
                    == 1
            });
        p.state = if valid && verdicts.iter().all(|v| v.passed) {
            State::Ready
        } else {
            State::Rejected
        };
        p.evidence = verdicts;
        self.save(&p, Some(&old))?;
        Ok(p)
    }
    pub fn fail_evaluation(&self, host: &AccessContext, id: &str) -> Result<()> {
        let _guard = self.operation.lock().map_err(storage)?;
        let mut p = self.get(host, id)?;
        if p.state != State::Evaluating {
            return Err(error("INVALID_STATE", "evaluation not active"));
        }
        let old = p.clone();
        p.state = State::Failed;
        self.save(&p, Some(&old))
    }
    pub fn evaluate(
        &self,
        host: &AccessContext,
        id: &str,
        verifier: &dyn Verifier,
    ) -> Result<Proposal> {
        let _guard = self.operation.lock().map_err(storage)?;
        let mut p = self.get(host, id)?;
        if p.state != State::Draft {
            return Err(error("INVALID_STATE", "expected draft"));
        }
        let original = p.clone();
        // A changed host configuration cannot silently redefine a proposal's gate.
        if p.validators != self.validators || p.reviewers != self.reviewers {
            p.state = State::Failed;
            self.save(&p, Some(&original))?;
            return Err(error("CONFIG_CHANGED", "gate configuration changed"));
        }
        let result = verifier.verify(&p);
        p.state = State::Evaluated;
        match result {
            Ok(v) => {
                let mut expected = p.validators.clone();
                expected.extend(p.reviewers.clone());
                let valid = v.len() == expected.len()
                    && expected.iter().all(|id| {
                        v.iter()
                            .filter(|x| &x.id == id && !x.evidence.trim().is_empty())
                            .count()
                            == 1
                    });
                p.state = if valid && v.iter().all(|x| x.passed) {
                    State::Ready
                } else {
                    State::Rejected
                };
                p.evidence = v;
            }
            Err(_) => p.state = State::Failed,
        }
        self.save(&p, Some(&original))?;
        Ok(p)
    }
    pub fn reject(&self, host: &AccessContext, id: &str) -> Result<()> {
        let _guard = self.operation.lock().map_err(storage)?;
        let mut p = self.get(host, id)?;
        if !matches!(p.state, State::Draft | State::Evaluated | State::Ready) {
            return Err(error("INVALID_STATE", "cannot reject"));
        }
        let original = p.clone();
        p.state = State::Rejected;
        self.save(&p, Some(&original))
    }
    pub fn publish(
        &self,
        host: &AccessContext,
        id: &str,
        publisher: &dyn SkillsPublisher,
    ) -> Result<Proposal> {
        let _guard = self.operation.lock().map_err(storage)?;
        let mut p = self.get(host, id)?;
        if p.state != State::Ready {
            return Err(error("INVALID_STATE", "expected ready"));
        }
        // Persist uncertainty before the external effect; never automatically retry after a crash.
        let original = p.clone();
        p.state = State::PendingPublish;
        self.save(&p, Some(&original))?;
        let pending = p.clone();
        match publisher.publish(host, &p) {
            Ok(revision) if !revision.is_empty() => {
                p.published_revision = Some(revision);
                p.state = State::Published
            }
            Ok(_) => {}
            Err(e) if e.code == "CONFLICT" => p.state = State::Stale,
            Err(_) => {}
        }
        self.save(&p, Some(&pending))?;
        Ok(p)
    }
    pub fn recover(
        &self,
        host: &AccessContext,
        id: &str,
        recovery: &dyn PublicationRecovery,
    ) -> Result<Proposal> {
        let _guard = self.operation.lock().map_err(storage)?;
        let mut p = self.get(host, id)?;
        if p.state != State::PendingPublish {
            return Err(error("INVALID_STATE", "expected pending publication"));
        }
        let pending = p.clone();
        match recovery.resolve(host, &p)? {
            RecoveryOutcome::Published(revision) if !revision.is_empty() => {
                p.published_revision = Some(revision);
                p.state = State::Published;
            }
            RecoveryOutcome::Published(_) => {
                return Err(error("INVALID_INPUT", "empty recovered revision"))
            }
            RecoveryOutcome::Stale => p.state = State::Stale,
            RecoveryOutcome::Failed => p.state = State::Failed,
        }
        self.save(&p, Some(&pending))?;
        Ok(p)
    }
    pub fn rollback(
        &self,
        host: &AccessContext,
        id: &str,
        publisher: &dyn SkillsPublisher,
    ) -> Result<()> {
        let _guard = self.operation.lock().map_err(storage)?;
        let p = self.get(host, id)?;
        if p.state != State::Published {
            return Err(error("INVALID_STATE", "expected published"));
        }
        publisher.rollback(
            host,
            &p,
            p.published_revision
                .as_deref()
                .ok_or_else(|| error("INVALID_STATE", "missing revision"))?,
        )
    }
}

/// Adapter to the managed revision store; publication and rollback use its CAS operations.
pub struct ManagedSkillsPublisher<'a>(pub &'a mh_skills::SkillsService);
impl SkillsPublisher for ManagedSkillsPublisher<'_> {
    fn publish(&self, actor: &AccessContext, p: &Proposal) -> Result<String> {
        Ok(self
            .0
            .publish_audited(
                actor,
                &p.scope,
                &p.skill,
                &p.candidate,
                Some(&p.base_revision),
                &format!("curation:publish:{}", p.id),
            )?
            .revision)
    }
    fn rollback(&self, actor: &AccessContext, p: &Proposal, expected: &str) -> Result<()> {
        self.0.rollback_audited(
            actor,
            &p.scope,
            &p.skill,
            &p.base_revision,
            expected,
            &format!("curation:rollback:{}", p.id),
        )
    }
}
impl PublicationRecovery for ManagedSkillsPublisher<'_> {
    fn resolve(&self, actor: &AccessContext, p: &Proposal) -> Result<RecoveryOutcome> {
        match self.0.publication_result(
            actor,
            &p.scope,
            &p.skill,
            &p.candidate,
            Some(&p.base_revision),
            &format!("curation:publish:{}", p.id),
        )? {
            Some(revision) => Ok(RecoveryOutcome::Published(revision)),
            None => Ok(RecoveryOutcome::Failed),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    fn host() -> AccessContext {
        AccessContext {
            actor_id: "actor".into(),
            session_id: "session".into(),
            workspace_id: Some("ws".into()),
            allow_user_scope: false,
        }
    }
    fn scope() -> Scope {
        Scope::Workspace {
            workspace_id: "ws".into(),
        }
    }
    fn proposal(base: String) -> Proposal {
        Proposal {
            id: "p".into(),
            scope: scope(),
            actor: String::new(),
            sources: vec![SourceRef {
                session_id: "session".into(),
                message_id: Some("m".into()),
                event_seq: None,
                run_id: None,
                content_digest: "a".repeat(64),
            }],
            skill: "example".into(),
            base_revision: base,
            candidate: "---\nname: example\ndescription: candidate\n---\nnew".into(),
            diff: "change".into(),
            rationale: "reason".into(),
            applicability: "scope".into(),
            negative_example: "negative".into(),
            validators: vec![],
            reviewers: vec![],
            state: State::Draft,
            evidence: vec![],
            published_revision: None,
        }
    }
    fn service(path: &Path) -> Curation {
        Curation::open(
            path,
            vec!["hard".into()],
            vec!["review1".into(), "review2".into()],
        )
        .unwrap()
    }
    struct Checks(Vec<Verdict>);
    impl Verifier for Checks {
        fn verify(&self, _: &Proposal) -> Result<Vec<Verdict>> {
            Ok(self.0.clone())
        }
    }
    fn checks() -> Checks {
        Checks(
            ["hard", "review1", "review2"]
                .into_iter()
                .map(|id| Verdict {
                    id: id.into(),
                    passed: true,
                    evidence: "verified".into(),
                })
                .collect(),
        )
    }
    #[test]
    fn complete_gate_is_ready_and_persisted() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("c.db");
        let c = service(&path);
        c.propose(&host(), proposal("base".into())).unwrap();
        assert_eq!(
            c.evaluate(&host(), "p", &checks()).unwrap().state,
            State::Ready
        );
        drop(c);
        assert_eq!(
            service(&path).get(&host(), "p").unwrap().state,
            State::Ready
        );
    }
    #[test]
    fn malformed_reviewer_verdict_is_rejected() {
        let dir = tempdir().unwrap();
        let c = service(&dir.path().join("c.db"));
        c.propose(&host(), proposal("base".into())).unwrap();
        let mut invalid = checks();
        invalid.0[2].evidence = " ".into();
        assert_eq!(
            c.evaluate(&host(), "p", &invalid).unwrap().state,
            State::Rejected
        );
    }
    #[test]
    fn stale_base_does_not_overwrite_managed_skill() {
        let dir = tempdir().unwrap();
        let skills = mh_skills::SkillsService::open(&dir.path().join("skills.db")).unwrap();
        let old = skills
            .publish(
                &host(),
                &scope(),
                "example",
                "---\nname: example\ndescription: old\n---\nold",
                None,
            )
            .unwrap();
        let c = service(&dir.path().join("c.db"));
        c.propose(&host(), proposal(old.revision.clone())).unwrap();
        c.evaluate(&host(), "p", &checks()).unwrap();
        let newer = skills
            .publish(
                &host(),
                &scope(),
                "example",
                "---\nname: example\ndescription: newer\n---\nnewer",
                Some(&old.revision),
            )
            .unwrap();
        assert_eq!(
            c.publish(&host(), "p", &ManagedSkillsPublisher(&skills))
                .unwrap()
                .state,
            State::Stale
        );
        assert_eq!(
            skills.list(&host(), &scope()).unwrap().items[0].revision,
            newer.revision
        );
    }
    #[test]
    fn publish_and_rollback_managed_revision() {
        let dir = tempdir().unwrap();
        let skills = mh_skills::SkillsService::open(&dir.path().join("skills.db")).unwrap();
        let old = skills
            .publish(
                &host(),
                &scope(),
                "example",
                "---\nname: example\ndescription: old\n---\nold",
                None,
            )
            .unwrap();
        let c = service(&dir.path().join("c.db"));
        c.propose(&host(), proposal(old.revision.clone())).unwrap();
        c.evaluate(&host(), "p", &checks()).unwrap();
        let published = c
            .publish(&host(), "p", &ManagedSkillsPublisher(&skills))
            .unwrap();
        assert_eq!(published.state, State::Published);
        c.rollback(&host(), "p", &ManagedSkillsPublisher(&skills))
            .unwrap();
        assert_eq!(
            skills.list(&host(), &scope()).unwrap().items[0].revision,
            old.revision
        );
    }
    #[test]
    fn recovery_after_lost_ack_uses_audit() {
        struct LostAck<'a>(&'a mh_skills::SkillsService);
        impl SkillsPublisher for LostAck<'_> {
            fn publish(&self, actor: &AccessContext, p: &Proposal) -> Result<String> {
                ManagedSkillsPublisher(self.0).publish(actor, p)?;
                Err(error("LOST_ACK", "connection lost"))
            }
            fn rollback(&self, _: &AccessContext, _: &Proposal, _: &str) -> Result<()> {
                unreachable!()
            }
        }
        let dir = tempdir().unwrap();
        let skills = mh_skills::SkillsService::open(&dir.path().join("skills.db")).unwrap();
        let old = skills
            .publish(
                &host(),
                &scope(),
                "example",
                "---\nname: example\ndescription: old\n---\nold",
                None,
            )
            .unwrap();
        let path = dir.path().join("curation.db");
        let c = service(&path);
        c.propose(&host(), proposal(old.revision)).unwrap();
        c.evaluate(&host(), "p", &checks()).unwrap();
        assert_eq!(
            c.publish(&host(), "p", &LostAck(&skills)).unwrap().state,
            State::PendingPublish
        );
        drop(c);
        let recovered = service(&path)
            .recover(&host(), "p", &ManagedSkillsPublisher(&skills))
            .unwrap();
        assert_eq!(recovered.state, State::Published);
        assert_eq!(
            skills.list(&host(), &scope()).unwrap().items[0].revision,
            recovered.published_revision.clone().unwrap()
        );
        let mut changed = recovered.clone();
        changed.candidate.push('x');
        assert_eq!(
            ManagedSkillsPublisher(&skills)
                .publish(&host(), &changed)
                .unwrap_err()
                .code,
            "CONFLICT"
        );
    }
    #[test]
    fn gate_is_frozen_before_proposal() {
        let dir = tempdir().unwrap();
        let c = service(&dir.path().join("c.db"));
        let case = ValidationCase {
            input: "case".into(),
            contains: vec!["ok".into()],
            not_contains: vec![],
        };
        let mut gate = GateConfig {
            executor_model: "api:a/model".into(),
            reviewers: ["api:a/r".into(), "api:b/r".into()],
            trigger: case.clone(),
            holdouts: vec![case],
            max_calls: 6,
        };
        c.configure_gate(&host(), &scope(), &gate).unwrap();
        c.propose(&host(), proposal("base".into())).unwrap();
        gate.trigger.input = "changed".into();
        c.configure_gate(&host(), &scope(), &gate).unwrap();
        assert_eq!(c.gate_for(&host(), "p").unwrap().trigger.input, "case");
        let mut other = host();
        other.actor_id = "other".into();
        assert!(c.gate_for(&other, "p").is_err());
    }
}
