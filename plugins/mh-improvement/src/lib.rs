use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    Parallel,
    Sequential,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    Baseline,
    GeneralProposal,
    DomainProposal,
    GeneralProbe,
    DomainProbe,
    Compose,
    Final,
    Select,
    Reflect,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub mode: Mode,
    pub k: u8,
    pub budget: u32,
    pub input_digest: String,
    pub library_revision: String,
    pub allowed_components: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub stage: Stage,
    pub round: u8,
    pub library_revision: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ComponentDiff {
    pub component: String,
    pub path: String,
    pub diff: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Proposal {
    pub target_deficiency: String,
    pub scope: Scope,
    pub base_digest: String,
    pub component_diffs: Vec<ComponentDiff>,
    pub hypothesis: String,
    pub evidence_refs: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Scope {
    General,
    Domain,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Compose {
    pub adopted_diffs: Vec<ComponentDiff>,
    pub rejected_diffs: Vec<ComponentDiff>,
    pub reasons: String,
    pub evidence_refs: Vec<String>,
    pub result_digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Reflection {
    pub cue: String,
    pub lesson: String,
    pub evidence_refs: Vec<String>,
    pub limitations: String,
    pub bundle_digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Outcome {
    Evidence { digest: String, content: String },
    Proposal { proposal: Proposal },
    Compose { compose: Compose },
    Reflection { reflection: Reflection },
    Selected { index: usize },
    NoEntry,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub step: Step,
    pub outcome: Outcome,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    request: Request,
    records: Vec<Record>,
    cancelled: bool,
    #[serde(default)]
    pending: bool,
}
pub trait StageExecutor {
    fn execute(&mut self, step: &Step, evidence: &[Record]) -> Result<Outcome, String>;
    fn select(&mut self, candidates: &[String]) -> Result<usize, String>;
}
impl Journal {
    pub fn new(request: Request) -> Result<Self, String> {
        if !(1..=16).contains(&request.k)
            || request.input_digest.is_empty()
            || request.library_revision.is_empty()
        {
            return Err("invalid request".into());
        }
        Ok(Self {
            request,
            records: vec![],
            cancelled: false,
            pending: false,
        })
    }
    pub fn records(&self) -> &[Record] {
        &self.records
    }
    pub fn cancel(&mut self) {
        self.cancelled = true;
    }
    pub fn resume(&mut self, input_digest: &str, library_revision: &str) -> Result<(), String> {
        if self.pending {
            return Err("uncertain stage requires reconciliation".into());
        }
        if self.request.input_digest != input_digest
            || self.request.library_revision != library_revision
        {
            return Err("resume digest mismatch".into());
        }
        self.validate()?;
        self.cancelled = false;
        Ok(())
    }
    fn schedule(&self) -> Vec<Step> {
        let mut v = Vec::new();
        let mut add = |stage, round| {
            v.push(Step {
                stage,
                round,
                library_revision: self.request.library_revision.clone(),
            })
        };
        for i in 0..self.request.k {
            add(Stage::Baseline, i);
        }
        let stages = [
            Stage::GeneralProposal,
            Stage::DomainProposal,
            Stage::GeneralProbe,
            Stage::DomainProbe,
            Stage::Compose,
            Stage::Final,
        ];
        match self.request.mode {
            Mode::Sequential => {
                for i in 0..self.request.k {
                    for s in &stages {
                        add(s.clone(), i);
                    }
                }
            }
            Mode::Parallel => {
                for s in &stages {
                    for i in 0..self.request.k {
                        add(s.clone(), i);
                    }
                }
            }
        }
        if self.request.k > 1 {
            add(Stage::Select, 0);
        }
        add(Stage::Reflect, 0);
        v
    }
    fn validate(&self) -> Result<(), String> {
        Self::new(self.request.clone())?;
        let schedule = self.schedule();
        if self.records.len() > schedule.len() {
            return Err("invalid journal".into());
        }
        for (index, (record, step)) in self.records.iter().zip(schedule).enumerate() {
            if record.step != step
                || !self.valid_outcome(&record.step, &record.outcome, &self.records[..index])
            {
                return Err("invalid journal".into());
            }
        }
        Ok(())
    }
    pub fn run(&mut self, executor: &mut impl StageExecutor) -> Result<(), String> {
        self.run_persisted(executor, |_| Ok(()))
    }
    /// Checkpoint before dispatch and after completion; a lost call is never retried.
    pub fn run_persisted(
        &mut self,
        executor: &mut impl StageExecutor,
        mut checkpoint: impl FnMut(&Journal) -> Result<(), String>,
    ) -> Result<(), String> {
        self.validate()?;
        if self.pending {
            return Err("uncertain stage requires reconciliation".into());
        }
        let schedule = self.schedule();
        while self.records.len() < schedule.len() {
            if self.cancelled {
                return Err("cancelled".into());
            }
            if self.records.len() as u32 >= self.request.budget {
                self.cancelled = true;
                return Err("budget exhausted".into());
            }
            let step = &schedule[self.records.len()];
            self.pending = true;
            checkpoint(self)?;
            let result = if step.stage == Stage::Select {
                let candidates: Vec<String> = self
                    .records
                    .iter()
                    .filter_map(|r| match (&r.step.stage, &r.outcome) {
                        (Stage::Final, Outcome::Evidence { content, .. }) => Some(content.clone()),
                        _ => None,
                    })
                    .collect();
                executor
                    .select(&candidates)
                    .map(|index| Outcome::Selected { index })
            } else {
                executor.execute(step, &self.records)
            };
            let outcome = match result {
                Ok(o) => o,
                Err(e) => {
                    self.cancelled = true;
                    return Err(e);
                }
            };
            if !self.valid_outcome(step, &outcome, &self.records) {
                self.cancelled = true;
                return Err("invalid outcome".into());
            }
            self.records.push(Record {
                step: step.clone(),
                outcome,
            });
            self.pending = false;
            checkpoint(self)?;
        }
        Ok(())
    }
}
impl Journal {
    fn valid_outcome(&self, step: &Step, outcome: &Outcome, prior: &[Record]) -> bool {
        let refs_valid = |refs: &[String]| {
            refs.iter().all(|id| {
                prior
                    .iter()
                    .any(|r| matches!(&r.outcome, Outcome::Evidence { digest, .. } if digest == id))
            })
        };
        let diffs_valid = |diffs: &[ComponentDiff]| {
            diffs.iter().all(|d| {
                self.request.allowed_components.contains(&d.component)
                    && !d.diff.trim().is_empty()
                    && !d.path.is_empty()
                    && !d.path.contains('\\')
                    && !d.path.contains(':')
                    && !d.path.starts_with('/')
                    && d.path
                        .split('/')
                        .all(|part| !part.is_empty() && part != "." && part != "..")
            })
        };
        match (&step.stage, outcome) {
            (Stage::Baseline | Stage::GeneralProbe | Stage::DomainProbe | Stage::Final, Outcome::Evidence { digest, content }) => !digest.trim().is_empty() && !content.trim().is_empty(),
            (Stage::GeneralProposal | Stage::DomainProposal, Outcome::Proposal { proposal: p }) => {
                let expected = if step.stage == Stage::GeneralProposal { Scope::General } else { Scope::Domain };
                let base = if self.request.mode == Mode::Sequential && step.round > 0 {
                    prior.iter().rev().find_map(|r| match &r.outcome { Outcome::Compose { compose } => Some(compose.result_digest.as_str()), _ => None }).unwrap_or("")
                } else { &self.request.input_digest };
                p.scope == expected && p.base_digest == base && !p.target_deficiency.trim().is_empty() && !p.hypothesis.trim().is_empty() && !p.component_diffs.is_empty() && diffs_valid(&p.component_diffs) && refs_valid(&p.evidence_refs)
            },
            (Stage::Compose, Outcome::Compose { compose: c }) => !c.result_digest.trim().is_empty() && !c.reasons.trim().is_empty() && diffs_valid(&c.adopted_diffs) && diffs_valid(&c.rejected_diffs) && refs_valid(&c.evidence_refs) && c.adopted_diffs.iter().chain(&c.rejected_diffs).all(|d| prior.iter().any(|r| matches!(&r.outcome, Outcome::Proposal { proposal } if proposal.component_diffs.contains(d)))),
            (Stage::Reflect, Outcome::Reflection { reflection: r }) => !r.cue.trim().is_empty() && !r.lesson.trim().is_empty() && !r.bundle_digest.trim().is_empty() && refs_valid(&r.evidence_refs),
            (Stage::Reflect, Outcome::NoEntry) => true,
            (Stage::Select, Outcome::Selected { index }) => *index < prior.iter().filter(|r| r.step.stage == Stage::Final).count(),
            _ => false,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Executor {
        invalid: bool,
        index: usize,
        parallel: bool,
    }
    impl StageExecutor for Executor {
        fn execute(&mut self, s: &Step, _: &[Record]) -> Result<Outcome, String> {
            let diff = ComponentDiff {
                component: "prompt".into(),
                path: if self.invalid {
                    "../escape"
                } else {
                    "prompt.txt"
                }
                .into(),
                diff: "change".into(),
            };
            Ok(match s.stage {
                Stage::GeneralProposal | Stage::DomainProposal => Outcome::Proposal {
                    proposal: Proposal {
                        target_deficiency: "missing".into(),
                        scope: if s.stage == Stage::GeneralProposal {
                            Scope::General
                        } else {
                            Scope::Domain
                        },
                        base_digest: if s.round > 0 && !self.parallel {
                            "bundle"
                        } else {
                            "input"
                        }
                        .into(),
                        component_diffs: vec![diff],
                        hypothesis: "better".into(),
                        evidence_refs: vec![],
                    },
                },
                Stage::Compose => Outcome::Compose {
                    compose: Compose {
                        adopted_diffs: vec![diff],
                        rejected_diffs: vec![],
                        reasons: "reason".into(),
                        evidence_refs: vec![],
                        result_digest: "bundle".into(),
                    },
                },
                Stage::Reflect => Outcome::NoEntry,
                _ => Outcome::Evidence {
                    digest: format!("{:?}{}", s.stage, s.round),
                    content: "answer".into(),
                },
            })
        }
        fn select(&mut self, candidates: &[String]) -> Result<usize, String> {
            assert!(candidates.iter().all(|c| c == "answer"));
            Ok(self.index)
        }
    }
    fn journal(k: u8, mode: Mode, budget: u32) -> Journal {
        Journal::new(Request {
            mode,
            k,
            budget,
            input_digest: "input".into(),
            library_revision: "rev".into(),
            allowed_components: vec!["prompt".into()],
        })
        .unwrap()
    }
    #[test]
    fn order_and_no_entry() {
        let mut j = journal(1, Mode::Sequential, 20);
        j.run(&mut Executor {
            invalid: false,
            index: 0,
            parallel: false,
        })
        .unwrap();
        assert_eq!(
            j.records
                .iter()
                .map(|r| r.step.stage.clone())
                .collect::<Vec<_>>(),
            vec![
                Stage::Baseline,
                Stage::GeneralProposal,
                Stage::DomainProposal,
                Stage::GeneralProbe,
                Stage::DomainProbe,
                Stage::Compose,
                Stage::Final,
                Stage::Reflect
            ]
        );
        assert_eq!(j.records.last().unwrap().outcome, Outcome::NoEntry);
    }
    #[test]
    fn counts() {
        for mode in [Mode::Parallel, Mode::Sequential] {
            let mut j = journal(3, mode, 30);
            let mut e = Executor {
                invalid: false,
                index: 0,
                parallel: j.request.mode == Mode::Parallel,
            };
            assert!(j.run(&mut e).is_ok());
            assert_eq!(j.records.len(), 23);
        }
    }
    #[test]
    fn invalid_proposal_stops_before_probe() {
        let mut j = journal(1, Mode::Sequential, 20);
        assert!(j
            .run(&mut Executor {
                invalid: true,
                index: 0,
                parallel: false
            })
            .is_err());
        assert_eq!(j.records.len(), 1);
    }
    #[test]
    fn selector_cannot_synthesize() {
        let mut j = journal(2, Mode::Sequential, 30);
        assert!(j
            .run(&mut Executor {
                invalid: false,
                index: 2,
                parallel: false
            })
            .is_err());
        assert_eq!(j.records.last().unwrap().step.stage, Stage::Final);
    }
    #[test]
    fn uncertain_stage_is_not_replayed_after_restart() {
        let mut journal = Journal::new(Request {
            mode: Mode::Parallel,
            k: 1,
            budget: 20,
            input_digest: "input".into(),
            library_revision: "library".into(),
            allowed_components: vec!["prompt".into()],
        })
        .unwrap();
        let mut executor = Executor {
            invalid: false,
            index: 0,
            parallel: true,
        };
        let mut checkpoint = None;
        let result = journal.run_persisted(&mut executor, |state| {
            checkpoint = Some(state.clone());
            Err("disk failure".into())
        });
        assert!(result.is_err());
        let mut restored = checkpoint.unwrap();
        assert!(restored.resume("input", "library").is_err());
        assert!(restored.run(&mut executor).is_err());
    }
}
