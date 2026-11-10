use super::*;
use minerva_plugin_sdk::learning::{AccessContext, Scope};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Limits {
    pub max_calls: i64,
    pub max_input: i64,
    pub max_output: i64,
    pub max_concurrency: i64,
    pub max_wall_ms: i64,
    pub max_depth: i64,
    pub max_cost_minor: Option<i64>,
    pub input_rate_per_million: Option<i64>,
    pub output_rate_per_million: Option<i64>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    pub id: String,
    pub state: String,
    pub generation: i64,
    pub input_digest: String,
    pub usage: Vec<UsageView>,
    pub stages: Vec<StageInfo>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageView {
    pub id: String,
    pub trial: String,
    pub state: String,
    pub unknown: bool,
    pub estimate: UsageValues,
    pub charged: UsageValues,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageValues {
    pub calls: i64,
    pub input: i64,
    pub output: i64,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageInfo {
    pub sequence: i64,
    pub generation: i64,
}
fn authority(host: &AccessContext, scope: &Scope) -> Result<String> {
    host.authorize(scope).map_err(|_| Error::NotFound)?;
    serde_json::to_string(scope).map_err(|_| Error::Invalid)
}
fn now() -> Result<i64> {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::Conflict)?
            .as_millis(),
    )
    .map_err(|_| Error::Invalid)
}
fn cost(input: i64, output: i64, ir: i64, orr: i64) -> Result<i64> {
    let n = (input as i128) * (ir as i128) + (output as i128) * (orr as i128);
    i64::try_from((n + 999_999) / 1_000_000).map_err(|_| Error::BudgetExceeded)
}
impl Store {
    pub fn host_create(
        &self,
        host: &AccessContext,
        scope: &Scope,
        job: &str,
        input_digest: &str,
        l: &Limits,
    ) -> Result<()> {
        let scope = authority(host, scope)?;
        if !id(job)
            || !digest(input_digest)
            || l.max_calls <= 0
            || l.max_input < 0
            || l.max_output < 0
            || !(1..=2).contains(&l.max_concurrency)
            || l.max_wall_ms <= 0
            || l.max_depth < 0
            || l.max_cost_minor.is_some_and(|x| x < 0)
            || l.input_rate_per_million.is_some_and(|x| x < 0)
            || l.output_rate_per_million.is_some_and(|x| x < 0)
            || (l.max_cost_minor.is_some()
                && (l.input_rate_per_million.is_none() || l.output_rate_per_million.is_none()))
        {
            return Err(Error::Invalid);
        }
        let mut db = self.db.lock().map_err(|_| Error::Conflict)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO jobs VALUES (?1,?2,?3,?4,'draft',0,?5,?6,?7,?8)",
            params![
                host.actor_id,
                scope,
                job,
                input_digest,
                l.max_calls,
                l.max_input,
                l.max_output,
                l.max_concurrency
            ],
        )?;
        tx.execute(
            "INSERT INTO limits VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                host.actor_id,
                scope,
                job,
                now()?,
                l.max_wall_ms,
                l.max_depth,
                l.max_cost_minor,
                l.input_rate_per_million,
                l.output_rate_per_million
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn host_get(&self, host: &AccessContext, scope: &Scope, job: &str) -> Result<JobView> {
        let scope = authority(host, scope)?;
        let item = self.get(&host.actor_id, &scope, job)?;
        let db = self.db.lock().map_err(|_| Error::Conflict)?;
        let mut stmt=db.prepare("SELECT r.id,r.trial,r.state,r.unknown,COALESCE(e.calls,r.calls),COALESCE(e.input,r.input),COALESCE(e.output,r.output),r.calls,r.input,r.output FROM reservations r LEFT JOIN estimates e ON e.actor=r.actor AND e.scope=r.scope AND e.job=r.job AND e.id=r.id WHERE r.actor=?1 AND r.scope=?2 AND r.job=?3 ORDER BY r.id")?;
        let usage = stmt
            .query_map(params![host.actor_id, scope, job], |r| {
                Ok(UsageView {
                    id: r.get(0)?,
                    trial: r.get(1)?,
                    state: r.get(2)?,
                    unknown: r.get(3)?,
                    estimate: UsageValues {
                        calls: r.get(4)?,
                        input: r.get(5)?,
                        output: r.get(6)?,
                    },
                    charged: UsageValues {
                        calls: r.get(7)?,
                        input: r.get(8)?,
                        output: r.get(9)?,
                    },
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut stmt=db.prepare("SELECT seq,generation FROM checkpoints WHERE actor=?1 AND scope=?2 AND job=?3 ORDER BY seq LIMIT 1024")?;
        let stages = stmt
            .query_map(params![host.actor_id, scope, job], |r| {
                Ok(StageInfo {
                    sequence: r.get(0)?,
                    generation: r.get(1)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(JobView {
            id: job.into(),
            state: item.state,
            generation: item.generation,
            input_digest: item.input_digest,
            usage,
            stages,
        })
    }
    pub fn host_list(&self, host: &AccessContext, scope: &Scope) -> Result<Vec<JobView>> {
        let key = authority(host, scope)?;
        let ids = {
            let db = self.db.lock().map_err(|_| Error::Conflict)?;
            let mut stmt = db
                .prepare("SELECT id FROM jobs WHERE actor=?1 AND scope=?2 ORDER BY id LIMIT 256")?;
            let rows = stmt.query_map(params![host.actor_id, key], |r| r.get::<_, String>(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        ids.iter()
            .map(|id| self.host_get(host, scope, id))
            .collect()
    }
    pub fn host_add_trial(
        &self,
        host: &AccessContext,
        scope: &Scope,
        job: &str,
        trial: &str,
        input: &str,
    ) -> Result<()> {
        self.add_trial(&host.actor_id, &authority(host, scope)?, job, trial, input)
    }
    pub fn host_start(&self, host: &AccessContext, scope: &Scope, job: &str) -> Result<i64> {
        self.start(&host.actor_id, &authority(host, scope)?, job)
    }
    pub fn host_settle(
        &self,
        host: &AccessContext,
        scope: &Scope,
        job: &str,
        reservation: &str,
        generation: i64,
        actual: Option<&Usage>,
    ) -> Result<()> {
        self.settle(
            &host.actor_id,
            &authority(host, scope)?,
            job,
            reservation,
            generation,
            actual,
        )
    }
    pub fn host_finish(
        &self,
        host: &AccessContext,
        scope: &Scope,
        job: &str,
        generation: i64,
        state: &str,
    ) -> Result<()> {
        if !matches!(state, "completed" | "failed" | "budgetExceeded") {
            return Err(Error::Invalid);
        }
        let key = authority(host, scope)?;
        let db = self.db.lock().map_err(|_| Error::Conflict)?;
        let n = db.execute("UPDATE jobs SET state=?5 WHERE actor=?1 AND scope=?2 AND id=?3 AND generation=?4 AND state='running'", params![host.actor_id,key,job,generation,state])?;
        if n != 1 {
            return Err(Error::Conflict);
        }
        Ok(())
    }
    pub fn host_resume(
        &self,
        host: &AccessContext,
        scope: &Scope,
        job: &str,
        input: &str,
    ) -> Result<i64> {
        self.resume(&host.actor_id, &authority(host, scope)?, job, input)
    }
    pub fn host_cancel(
        &self,
        host: &AccessContext,
        scope: &Scope,
        job: &str,
        runner: &dyn HostRunner,
    ) -> Result<()> {
        self.cancel(&host.actor_id, &authority(host, scope)?, job, runner)
    }
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    pub fn host_reserve(
        &self,
        host: &AccessContext,
        scope: &Scope,
        job: &str,
        trial: &str,
        reservation: &str,
        generation: i64,
        depth: i64,
        estimate: &Usage,
    ) -> Result<()> {
        let scope = authority(host, scope)?;
        if !id(reservation) || !estimate.valid() || depth < 0 {
            return Err(Error::Invalid);
        }
        let mut db = self.db.lock().map_err(|_| Error::Conflict)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing:Option<(String,i64,i64,i64,i64,i64)>=tx.query_row("SELECT trial,generation,calls,input,output,depth FROM estimates WHERE actor=?1 AND scope=?2 AND job=?3 AND id=?4",params![host.actor_id,scope,job,reservation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
        if let Some(e) = existing {
            return if e
                == (
                    trial.into(),
                    generation,
                    estimate.calls,
                    estimate.input,
                    estimate.output,
                    depth,
                ) {
                Ok(())
            } else {
                Err(Error::Conflict)
            };
        }
        let limits:Option<(i64,i64,i64,i64,i64,i64,i64,i64,Option<i64>,Option<i64>,Option<i64>)>=tx.query_row("SELECT j.max_calls,j.max_input,j.max_output,j.max_concurrency,l.created_ms,l.max_wall_ms,l.max_depth,j.generation,l.max_cost,l.input_rate,l.output_rate FROM jobs j JOIN limits l ON l.actor=j.actor AND l.scope=j.scope AND l.job=j.id WHERE j.actor=?1 AND j.scope=?2 AND j.id=?3 AND j.state='running' AND j.generation=?4 AND EXISTS(SELECT 1 FROM trials WHERE actor=?1 AND scope=?2 AND job=?3 AND id=?5)",params![host.actor_id,scope,job,generation,trial],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?))).optional()?;
        let (calls, input, output, concurrency, created, wall, max_depth, _, max_cost, ir, orr) =
            limits.ok_or(Error::Conflict)?;
        let elapsed = now()?.checked_sub(created).ok_or(Error::Conflict)?;
        if elapsed < 0 || elapsed >= wall || depth > max_depth {
            return Err(Error::BudgetExceeded);
        }
        let (uc,ui,uo,active):(i64,i64,i64,i64)=tx.query_row("SELECT COALESCE(SUM(calls),0),COALESCE(SUM(input),0),COALESCE(SUM(output),0),COALESCE(SUM(state='reserved'),0) FROM reservations WHERE actor=?1 AND scope=?2 AND job=?3",params![host.actor_id,scope,job],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        let nc = uc
            .checked_add(estimate.calls)
            .ok_or(Error::BudgetExceeded)?;
        let ni = ui
            .checked_add(estimate.input)
            .ok_or(Error::BudgetExceeded)?;
        let no = uo
            .checked_add(estimate.output)
            .ok_or(Error::BudgetExceeded)?;
        if nc > calls || ni > input || no > output || active >= concurrency {
            return Err(Error::BudgetExceeded);
        }
        if let Some(limit) = max_cost {
            if cost(
                ni,
                no,
                ir.ok_or(Error::Invalid)?,
                orr.ok_or(Error::Invalid)?,
            )? > limit
            {
                return Err(Error::BudgetExceeded);
            }
        }
        tx.execute(
            "INSERT INTO reservations VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'reserved',0)",
            params![
                host.actor_id,
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
            "INSERT INTO estimates VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,NULL)",
            params![
                host.actor_id,
                scope,
                job,
                reservation,
                trial,
                generation,
                estimate.calls,
                estimate.input,
                estimate.output,
                depth
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Runner;
    impl HostRunner for Runner {
        fn start(&self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn cancel_and_join(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }
    fn setup() -> (tempfile::TempDir, Store, AccessContext, Scope, Limits) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(d.path().join("db")).unwrap();
        let h = AccessContext {
            actor_id: "a".into(),
            session_id: "s".into(),
            workspace_id: Some("w".into()),
            allow_user_scope: false,
        };
        let scope = Scope::Workspace {
            workspace_id: "w".into(),
        };
        let l = Limits {
            max_calls: 4,
            max_input: 100,
            max_output: 100,
            max_concurrency: 1,
            max_wall_ms: 100000,
            max_depth: 1,
            max_cost_minor: Some(10),
            input_rate_per_million: Some(1_000_000),
            output_rate_per_million: Some(1_000_000),
        };
        (d, s, h, scope, l)
    }
    #[test]
    fn scoped_budget_idempotency_and_views() {
        let (_d, s, h, scope, l) = setup();
        let hash = "a".repeat(64);
        s.host_create(&h, &scope, "j", &hash, &l).unwrap();
        s.host_add_trial(&h, &scope, "j", "t", &hash).unwrap();
        let g = s.host_start(&h, &scope, "j").unwrap();
        let u = Usage {
            calls: 1,
            input: 4,
            output: 4,
        };
        s.host_reserve(&h, &scope, "j", "t", "r", g, 1, &u).unwrap();
        s.host_reserve(&h, &scope, "j", "t", "r", g, 1, &u).unwrap();
        assert!(matches!(
            s.host_reserve(&h, &scope, "j", "t", "r", g, 0, &u),
            Err(Error::Conflict)
        ));
        s.host_settle(&h, &scope, "j", "r", g, None).unwrap();
        assert!(matches!(
            s.host_reserve(&h, &scope, "j", "t", "r2", g, 1, &u),
            Err(Error::BudgetExceeded)
        ));
        assert!(matches!(
            s.host_reserve(&h, &scope, "j", "t", "r3", g, 2, &u),
            Err(Error::BudgetExceeded)
        ));
        let v = s.host_get(&h, &scope, "j").unwrap();
        assert!(v.usage[0].unknown);
        assert_eq!(v.usage[0].estimate.input, 4);
        assert_eq!(s.host_list(&h, &scope).unwrap().len(), 1);
        let mut other = h.clone();
        other.actor_id = "b".into();
        assert!(matches!(
            s.host_get(&other, &scope, "j"),
            Err(Error::NotFound)
        ));
        s.host_cancel(&h, &scope, "j", &Runner).unwrap();
        s.host_cancel(&h, &scope, "j", &Runner).unwrap();
    }
    #[test]
    fn unknown_tariff_and_expired_wall_rejected() {
        let (_d, s, h, scope, mut l) = setup();
        let hash = "a".repeat(64);
        l.input_rate_per_million = None;
        assert!(matches!(
            s.host_create(&h, &scope, "j", &hash, &l),
            Err(Error::Invalid)
        ));
        l.input_rate_per_million = Some(1);
        l.max_wall_ms = 1;
        s.host_create(&h, &scope, "j", &hash, &l).unwrap();
        s.host_add_trial(&h, &scope, "j", "t", &hash).unwrap();
        let g = s.host_start(&h, &scope, "j").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(matches!(
            s.host_reserve(
                &h,
                &scope,
                "j",
                "t",
                "r",
                g,
                0,
                &Usage {
                    calls: 1,
                    input: 0,
                    output: 0
                }
            ),
            Err(Error::BudgetExceeded)
        ));
    }
}
