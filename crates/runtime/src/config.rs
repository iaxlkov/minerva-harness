use std::path::PathBuf;

/// Fixed scope and explicit host-owned paths for one runtime instance.
#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    pub tenant_id: String,
    pub workspace_id: String,
    pub data_dir: PathBuf,
    pub config_dir: PathBuf,
    pub profile: String,
}
