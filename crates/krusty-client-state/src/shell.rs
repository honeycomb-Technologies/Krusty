#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellAction {
    PickAttachment,
    OpenBrowser { url: String },
    OpenTerminal { session_id: Option<String> },
    OpenLocalRuntimeSpike,
}
