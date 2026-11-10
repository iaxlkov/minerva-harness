use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
mod artifacts;
mod facade;
pub use facade::*;

#[derive(Debug)]
pub enum Error {
    Storage(rusqlite::Error),
    Invalid,
    NotFound,
    Conflict,
    BudgetExceeded,
    EnvironmentUnavailable,
    Io(std::io::Error),
    Corrupt,
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self::Storage(e)
    }
}
type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Budget {
    pub max_calls: i64,
    pub max_input: i64,
    pub max_output: i64,
    pub max_concurrency: i64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Usage {
    pub calls: i64,
    pub input: i64,
    pub output: i64,
}
impl Usage {
    fn valid(&self) -> bool {
        self.calls > 0 && self.input >= 0 && self.output >= 0
    }
}
#[derive(Debug, PartialEq, Eq)]
pub struct Job {
    pub state: String,
    pub generation: i64,
    pub input_digest: String,
}

/// Implementations must enforce their declared isolation; absence is a hard failure.
pub trait HostRunner: Send + Sync {
    fn start(&self, job: &str, trial: &str) -> Result<()>;
    fn cancel_and_join(&self, job: &str) -> Result<()>;
}
pub struct UnsupportedRunner;
impl HostRunner for UnsupportedRunner {
    fn start(&self, _: &str, _: &str) -> Result<()> {
        Err(Error::EnvironmentUnavailable)
    }
    fn cancel_and_join(&self, _: &str) -> Result<()> {
        Err(Error::EnvironmentUnavailable)
    }
}

pub struct Store {
    db: Mutex<Connection>,
    artifacts: PathBuf,
}
fn id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
}
fn digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let artifacts = path.parent().unwrap_or(Path::new(".")).join("artifacts");
        std::fs::create_dir_all(&artifacts)?;
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        let version: i64 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > 3 {
            return Err(Error::Conflict);
        }
        if version == 0 {
            db.execute_batch("BEGIN IMMEDIATE;
                CREATE TABLE jobs(actor TEXT NOT NULL, scope TEXT NOT NULL, id TEXT NOT NULL, input_digest TEXT NOT NULL, state TEXT NOT NULL, generation INTEGER NOT NULL, max_calls INTEGER NOT NULL, max_input INTEGER NOT NULL, max_output INTEGER NOT NULL, max_concurrency INTEGER NOT NULL, PRIMARY KEY(actor,scope,id));
                CREATE TABLE trials(actor TEXT NOT NULL, scope TEXT NOT NULL, job TEXT NOT NULL, id TEXT NOT NULL, input_digest TEXT NOT NULL, PRIMARY KEY(actor,scope,job,id));
                CREATE TABLE reservations(actor TEXT NOT NULL, scope TEXT NOT NULL, job TEXT NOT NULL, trial TEXT NOT NULL, id TEXT NOT NULL, generation INTEGER NOT NULL, calls INTEGER NOT NULL, input INTEGER NOT NULL, output INTEGER NOT NULL, state TEXT NOT NULL, unknown INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(actor,scope,job,id));
                PRAGMA user_version=1; COMMIT;")?;
        }
        if version < 2 {
            db.execute_batch("BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS artifact_refs(actor TEXT NOT NULL, scope TEXT NOT NULL, job TEXT NOT NULL, digest TEXT NOT NULL, bytes INTEGER NOT NULL, PRIMARY KEY(actor,scope,job,digest)); CREATE TABLE IF NOT EXISTS checkpoints(actor TEXT NOT NULL, scope TEXT NOT NULL, job TEXT NOT NULL, seq INTEGER NOT NULL, generation INTEGER NOT NULL, journal TEXT NOT NULL, PRIMARY KEY(actor,scope,job,seq)); PRAGMA user_version=2; COMMIT;")?;
        }
        if version < 3 {
            db.execute_batch("BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS limits(actor TEXT NOT NULL,scope TEXT NOT NULL,job TEXT NOT NULL,created_ms INTEGER NOT NULL,max_wall_ms INTEGER NOT NULL,max_depth INTEGER NOT NULL,max_cost INTEGER,input_rate INTEGER,output_rate INTEGER,PRIMARY KEY(actor,scope,job)); CREATE TABLE IF NOT EXISTS estimates(actor TEXT NOT NULL,scope TEXT NOT NULL,job TEXT NOT NULL,id TEXT NOT NULL,trial TEXT NOT NULL,generation INTEGER NOT NULL,calls INTEGER NOT NULL,input INTEGER NOT NULL,output INTEGER NOT NULL,depth INTEGER NOT NULL,cost INTEGER,PRIMARY KEY(actor,scope,job,id)); PRAGMA user_version=3; COMMIT;")?;
        }
        Ok(Self {
            db: Mutex::new(db),
            artifacts,
        })
    }
    pub fn create(
        &self,
        actor: &str,
        scope: &str,
        job: &str,
        input_digest: &str,
        budget: &Budget,
    ) -> Result<()> {
        if !id(actor)
            || !id(scope)
            || !id(job)
            || !digest(input_digest)
            || budget.max_calls <= 0
            || budget.max_input < 0
            || budget.max_output < 0
            || !(1..=2).contains(&budget.max_concurrency)
        {
            return Err(Error::Invalid);
        }
        self.db.lock().map_err(|_| Error::Conflict)?.execute(
            "INSERT INTO jobs VALUES (?1,?2,?3,?4,'draft',0,?5,?6,?7,?8)",
            params![
                actor,
                scope,
                job,
                input_digest,
                budget.max_calls,
                budget.max_input,
                budget.max_output,
                budget.max_concurrency
            ],
        )?;
        Ok(())
    }
    /// Call only after the host has confirmed that previous workers are gone.
    pub fn recover(&self) -> Result<usize> {
        let mut db = self.db.lock().map_err(|_| Error::Conflict)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("UPDATE reservations SET state='settled',unknown=1 WHERE state='reserved' AND EXISTS (SELECT 1 FROM jobs j WHERE j.actor=reservations.actor AND j.scope=reservations.scope AND j.id=reservations.job AND j.state IN ('running','cancelling'))", [])?;
        let count = tx.execute("UPDATE jobs SET state='interrupted',generation=generation+1 WHERE state IN ('running','cancelling')", [])?;
        tx.commit()?;
        Ok(count)
    }
    /// Resume admits new work only when the immutable input matches.
    pub fn resume(&self, actor: &str, scope: &str, job: &str, input_digest: &str) -> Result<i64> {
        if !digest(input_digest) {
            return Err(Error::Invalid);
        }
        self.db.lock().map_err(|_| Error::Conflict)?.query_row("UPDATE jobs SET state='running',generation=generation+1 WHERE actor=?1 AND scope=?2 AND id=?3 AND state='interrupted' AND input_digest=?4 RETURNING generation",params![actor,scope,job,input_digest],|r|r.get(0)).optional()?.ok_or(Error::Conflict)
    }
    pub fn get(&self, actor: &str, scope: &str, job: &str) -> Result<Job> {
        self.db.lock().map_err(|_| Error::Conflict)?.query_row("SELECT state,generation,input_digest FROM jobs WHERE actor=?1 AND scope=?2 AND id=?3", params![actor,scope,job], |r| Ok(Job { state:r.get(0)?, generation:r.get(1)?, input_digest:r.get(2)? })).optional()?.ok_or(Error::NotFound)
    }
    pub fn add_trial(
        &self,
        actor: &str,
        scope: &str,
        job: &str,
        trial: &str,
        input_digest: &str,
    ) -> Result<()> {
        if !id(trial) || !digest(input_digest) {
            return Err(Error::Invalid);
        }
        let db = self.db.lock().map_err(|_| Error::Conflict)?;
        let changed = db.execute("INSERT INTO trials SELECT actor,scope,id,?4,?5 FROM jobs WHERE actor=?1 AND scope=?2 AND id=?3 AND state='draft' AND input_digest=?5", params![actor,scope,job,trial,input_digest])?;
        if changed != 1 {
            return Err(Error::Conflict);
        }
        Ok(())
    }
    pub fn start(&self, actor: &str, scope: &str, job: &str) -> Result<i64> {
        let db = self.db.lock().map_err(|_| Error::Conflict)?;
        db.query_row("UPDATE jobs SET state='running',generation=generation+1 WHERE actor=?1 AND scope=?2 AND id=?3 AND state='draft' AND EXISTS(SELECT 1 FROM trials WHERE actor=?1 AND scope=?2 AND job=?3) RETURNING generation",params![actor,scope,job],|r|r.get(0)).optional()?.ok_or(Error::Conflict)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn reserve(
        &self,
        actor: &str,
        scope: &str,
        job: &str,
        trial: &str,
        reservation: &str,
        generation: i64,
        estimate: &Usage,
    ) -> Result<()> {
        if !id(reservation) || !estimate.valid() {
            return Err(Error::Invalid);
        }
        let mut db = self.db.lock().map_err(|_| Error::Conflict)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(String,i64,i64,i64,i64)> = tx.query_row("SELECT trial,generation,calls,input,output FROM estimates WHERE actor=?1 AND scope=?2 AND job=?3 AND id=?4", params![actor,scope,job,reservation], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        if let Some(value) = existing {
            return if value
                == (
                    trial.into(),
                    generation,
                    estimate.calls,
                    estimate.input,
                    estimate.output,
                ) {
                Ok(())
            } else {
                Err(Error::Conflict)
            };
        }
        let protected: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM limits WHERE actor=?1 AND scope=?2 AND job=?3)",
            params![actor, scope, job],
            |r| r.get(0),
        )?;
        if protected {
            return Err(Error::Conflict);
        }
        let limits: Option<(i64,i64,i64,i64)> = tx.query_row("SELECT max_calls,max_input,max_output,max_concurrency FROM jobs WHERE actor=?1 AND scope=?2 AND id=?3 AND state='running' AND generation=?4 AND EXISTS(SELECT 1 FROM trials WHERE actor=?1 AND scope=?2 AND job=?3 AND id=?5)",params![actor,scope,job,generation,trial],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let (calls, input, output, concurrency) = limits.ok_or(Error::Conflict)?;
        let used: (i64,i64,i64,i64) = tx.query_row("SELECT COALESCE(SUM(calls),0),COALESCE(SUM(input),0),COALESCE(SUM(output),0),COALESCE(SUM(state='reserved'),0) FROM reservations WHERE actor=?1 AND scope=?2 AND job=?3",params![actor,scope,job],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        if used.0.checked_add(estimate.calls).is_none_or(|n| n > calls)
            || used.1.checked_add(estimate.input).is_none_or(|n| n > input)
            || used
                .2
                .checked_add(estimate.output)
                .is_none_or(|n| n > output)
            || used.3 >= concurrency
        {
            return Err(Error::BudgetExceeded);
        }
        tx.execute(
            "INSERT INTO reservations VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'reserved',0)",
            params![
                actor,
                scope,
                job,
                trial,
                reservation,
                generation,
                estimate.calls,
                estimate.input,
                estimate.output
            ],
        )?;
        tx.execute(
            "INSERT INTO estimates VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,0,NULL)",
            params![
                actor,
                scope,
                job,
                reservation,
                trial,
                generation,
                estimate.calls,
                estimate.input,
                estimate.output
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn settle(
        &self,
        actor: &str,
        scope: &str,
        job: &str,
        reservation: &str,
        generation: i64,
        actual: Option<&Usage>,
    ) -> Result<()> {
        if actual.is_some_and(|u| !u.valid()) {
            return Err(Error::Invalid);
        }
        let db = self.db.lock().map_err(|_| Error::Conflict)?;
        let n = match actual {
            Some(u) => db.execute("UPDATE reservations SET calls=?6,input=?7,output=?8,state='settled' WHERE actor=?1 AND scope=?2 AND job=?3 AND id=?4 AND generation=?5 AND state='reserved' AND EXISTS(SELECT 1 FROM jobs WHERE actor=?1 AND scope=?2 AND id=?3 AND generation=?5)",params![actor,scope,job,reservation,generation,u.calls,u.input,u.output])?,
            None => db.execute("UPDATE reservations SET state='settled',unknown=1 WHERE actor=?1 AND scope=?2 AND job=?3 AND id=?4 AND generation=?5 AND state='reserved' AND EXISTS(SELECT 1 FROM jobs WHERE actor=?1 AND scope=?2 AND id=?3 AND generation=?5)",params![actor,scope,job,reservation,generation])?,
        };
        if n != 1 {
            return Err(Error::Conflict);
        }
        Ok(())
    }
    /// Stops admission and fences workers before the host cancels and joins them.
    pub fn cancel(
        &self,
        actor: &str,
        scope: &str,
        job: &str,
        runner: &dyn HostRunner,
    ) -> Result<()> {
        {
            let db = self.db.lock().map_err(|_| Error::Conflict)?;
            let n=db.execute("UPDATE jobs SET state='cancelling',generation=generation+1 WHERE actor=?1 AND scope=?2 AND id=?3 AND state IN ('draft','running')",params![actor,scope,job])?;
            if n != 1 {
                let state: Option<String> = db
                    .query_row(
                        "SELECT state FROM jobs WHERE actor=?1 AND scope=?2 AND id=?3",
                        params![actor, scope, job],
                        |r| r.get(0),
                    )
                    .optional()?;
                match state.as_deref() {
                    Some("cancelled") => return Ok(()),
                    Some("cancelling") => {}
                    _ => return Err(Error::Conflict),
                }
            }
        }
        runner.cancel_and_join(job)?;
        let mut db = self.db.lock().map_err(|_| Error::Conflict)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("UPDATE reservations SET state='settled',unknown=1 WHERE actor=?1 AND scope=?2 AND job=?3 AND state='reserved'",params![actor,scope,job])?;
        tx.execute("UPDATE jobs SET state='cancelled' WHERE actor=?1 AND scope=?2 AND id=?3 AND state='cancelling'",params![actor,scope,job])?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Joined;
    impl HostRunner for Joined {
        fn start(&self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn cancel_and_join(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }
    fn setup() -> (tempfile::TempDir, Store, i64) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(d.path().join("db")).unwrap();
        let b = Budget {
            max_calls: 2,
            max_input: 10,
            max_output: 10,
            max_concurrency: 1,
        };
        let hash = "a".repeat(64);
        s.create("a", "s", "j", &hash, &b).unwrap();
        s.add_trial("a", "s", "j", "t", &hash).unwrap();
        let g = s.start("a", "s", "j").unwrap();
        (d, s, g)
    }
    #[test]
    fn unknown_usage_remains_charged_and_survives_reopen() {
        let (d, s, g) = setup();
        let u = Usage {
            calls: 1,
            input: 7,
            output: 4,
        };
        s.reserve("a", "s", "j", "t", "r", g, &u).unwrap();
        s.settle("a", "s", "j", "r", g, None).unwrap();
        drop(s);
        let s = Store::open(d.path().join("db")).unwrap();
        assert!(matches!(
            s.reserve("a", "s", "j", "t", "r2", g, &u),
            Err(Error::BudgetExceeded)
        ));
        assert!(matches!(s.get("other", "s", "j"), Err(Error::NotFound)));
    }
    #[test]
    fn cancellation_fences_inflight_and_admission() {
        let (_d, s, g) = setup();
        let u = Usage {
            calls: 1,
            input: 1,
            output: 1,
        };
        s.reserve("a", "s", "j", "t", "r", g, &u).unwrap();
        assert!(matches!(
            s.reserve("a", "s", "j", "t", "r2", g, &u),
            Err(Error::BudgetExceeded)
        ));
        s.cancel("a", "s", "j", &Joined).unwrap();
        assert!(matches!(
            s.settle("a", "s", "j", "r", g, Some(&u)),
            Err(Error::Conflict)
        ));
        assert!(matches!(
            s.reserve("a", "s", "j", "t", "r2", g, &u),
            Err(Error::Conflict)
        ));
        assert_eq!(s.get("a", "s", "j").unwrap().state, "cancelled");
    }
    #[test]
    fn recovery_fences_workers_and_keeps_unknown_estimate() {
        let (_d, s, g) = setup();
        let usage = Usage {
            calls: 1,
            input: 7,
            output: 4,
        };
        s.reserve("a", "s", "j", "t", "r", g, &usage).unwrap();
        assert_eq!(s.recover().unwrap(), 1);
        assert!(matches!(
            s.resume("a", "s", "j", &"b".repeat(64)),
            Err(Error::Conflict)
        ));
        let next = s.resume("a", "s", "j", &"a".repeat(64)).unwrap();
        assert!(matches!(
            s.settle("a", "s", "j", "r", g, Some(&usage)),
            Err(Error::Conflict)
        ));
        assert!(matches!(
            s.reserve("a", "s", "j", "t", "r2", next, &usage),
            Err(Error::BudgetExceeded)
        ));
    }
}
