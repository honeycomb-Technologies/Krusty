/// Update status
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateStatus {
    Checking,
    UpToDate,
    Available(UpdateInfo),
    Downloading { progress: String },
    Ready { version: String },
    Error(String),
}

/// Information about an available update
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateInfo {
    pub current_version: String,
    pub new_version: String,
    pub release_notes: String,
    pub is_dev_mode: bool,
}
