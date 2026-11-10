use crate::{context::skill_provider::SkillProjection, learning::Learning};
use minerva_plugin_sdk::{
    context::SkillInfo,
    learning::Scope,
    skills::{Invocation, LoadSkill},
};
use std::sync::Weak;

pub struct LearningSkills(pub Weak<Learning>);
impl SkillProjection for LearningSkills {
    fn list(&self, session: &str) -> Result<Vec<SkillInfo>, String> {
        let learning = self.0.upgrade().ok_or("learning host unavailable")?;
        if !learning.enabled("skills") {
            return Ok(Vec::new());
        }
        let access = learning.access(session)?;
        learning.ensure_skill_roots(&access)?;
        let scope = access
            .workspace_id
            .as_ref()
            .map(|id| Scope::Workspace {
                workspace_id: id.clone(),
            })
            .unwrap_or_else(|| Scope::Session {
                session_id: session.into(),
            });
        let catalog = learning
            .skills
            .list(&access, &scope)
            .map_err(|e| e.to_string())?;
        if catalog.incomplete {
            return Err("skill catalog incomplete".into());
        }
        Ok(catalog
            .items
            .into_iter()
            .filter(|s| s.model_invocable)
            .map(|s| SkillInfo {
                name: s.name,
                description: s.description,
            })
            .collect())
    }
    fn load(&self, session: &str, name: &str, selected: bool) -> Result<String, String> {
        let learning = self.0.upgrade().ok_or("learning host unavailable")?;
        if !learning.enabled("skills") {
            return Err("skills plugin disabled".into());
        }
        let access = learning.access(session)?;
        learning.ensure_skill_roots(&access)?;
        let scope = access
            .workspace_id
            .as_ref()
            .map(|id| Scope::Workspace {
                workspace_id: id.clone(),
            })
            .unwrap_or_else(|| Scope::Session {
                session_id: session.into(),
            });
        learning
            .skills
            .load(
                &access,
                &scope,
                &LoadSkill {
                    name: name.into(),
                    revision: None,
                },
                if selected {
                    Invocation::Selected
                } else {
                    Invocation::Model
                },
            )
            .map(|s| s.body)
            .map_err(|e| e.to_string())
    }
}
