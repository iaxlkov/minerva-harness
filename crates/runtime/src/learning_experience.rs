//! Only the host's completed experiment ledger can attest experience provenance.
use mh_experience::{EvidenceAssessment, EvidenceVerifier};
use minerva_plugin_sdk::{
    experience::{BundleManifest, PublishExperience},
    learning::{AccessContext, LearningError},
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
pub(crate) struct ExperimentEvidence(pub Arc<mh_experiments::Store>);
impl EvidenceVerifier for ExperimentEvidence {
    fn verify(
        &self,
        access: &AccessContext,
        publication: &PublishExperience,
        manifest: &BundleManifest,
    ) -> Result<EvidenceAssessment, LearningError> {
        let fail = || {
            LearningError::new(
                "EVIDENCE_UNAVAILABLE",
                "completed contrasting executions are required",
            )
        };
        let job = self
            .0
            .host_get(access, &publication.scope, &publication.experiment_id)
            .map_err(|_| fail())?;
        if job.state != "completed"
            || manifest.scope != publication.scope
            || publication.evidence_refs.is_empty()
        {
            return Err(fail());
        }
        let scope = serde_json::to_string(&publication.scope).map_err(|_| fail())?;
        for evidence in &publication.evidence_refs {
            let bytes = self
                .0
                .artifact(
                    &access.actor_id,
                    &scope,
                    &publication.experiment_id,
                    &evidence.digest,
                )
                .map_err(|_| fail())?;
            if bytes.len() as u64 != evidence.bytes
                || format!("{:x}", Sha256::digest(&bytes)) != evidence.digest
            {
                return Err(fail());
            }
        }
        let (_, checkpoint) = self
            .0
            .load_checkpoint(&access.actor_id, &scope, &publication.experiment_id)
            .map_err(|_| fail())?
            .ok_or_else(fail)?;
        let journal: serde_json::Value = serde_json::from_str(&checkpoint).map_err(|_| fail())?;
        let records = journal["records"].as_array().ok_or_else(fail)?;
        for stage in ["baseline", "generalProbe", "domainProbe", "final"] {
            if !records
                .iter()
                .any(|r| r["step"]["stage"] == stage && r["outcome"]["kind"] == "evidence")
            {
                return Err(fail());
            }
        }
        // Selection and model preference establish provenance, not objective regression safety.
        Ok(EvidenceAssessment::Provisional)
    }
}
