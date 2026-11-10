use crate::SkillInfo;
/// Host-owned catalog and body projection. Implementations enforce lifecycle and scope.
pub trait SkillProjection: Send + Sync {
    fn list(&self, session: &str) -> Result<Vec<SkillInfo>, String>;
    fn load(&self, session: &str, name: &str, selected: bool) -> Result<String, String>;
}
