pub mod chat;
pub mod scratch;

use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct PanelId(u64);

impl PanelId {
    pub fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelKind {
    Chat,
    ScratchCanvas,
}

impl PanelKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            Self::ScratchCanvas => "Scratch Canvas",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitAxis {
    Horizontal,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LayoutNode {
    Panel(PanelId),
    Split {
        axis: SplitAxis,
        ratio: f32,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
}

impl LayoutNode {
    #[cfg(test)]
    pub fn contains(&self, panel_id: PanelId) -> bool {
        match self {
            Self::Panel(id) => *id == panel_id,
            Self::Split { first, second, .. } => {
                first.contains(panel_id) || second.contains(panel_id)
            }
        }
    }

    pub fn panel_ids(&self, output: &mut Vec<PanelId>) {
        match self {
            Self::Panel(id) => output.push(*id),
            Self::Split { first, second, .. } => {
                first.panel_ids(output);
                second.panel_ids(output);
            }
        }
    }

    fn split_panel(&mut self, target: PanelId, axis: SplitAxis, new_panel: PanelId) -> bool {
        match self {
            Self::Panel(id) if *id == target => {
                let old = *id;
                *self = Self::Split {
                    axis,
                    ratio: 0.5,
                    first: Box::new(Self::Panel(old)),
                    second: Box::new(Self::Panel(new_panel)),
                };
                true
            }
            Self::Panel(_) => false,
            Self::Split { first, second, .. } => {
                first.split_panel(target, axis, new_panel)
                    || second.split_panel(target, axis, new_panel)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PanelState {
    pub id: PanelId,
    pub kind: PanelKind,
    pub title: String,
}

#[derive(Clone, Debug)]
pub struct PanelWorkspace {
    next_id: u64,
    panels: BTreeMap<PanelId, PanelState>,
    layout: LayoutNode,
    focused: PanelId,
}

impl PanelWorkspace {
    #[cfg(test)]
    pub fn starter() -> Self {
        Self::starter_with_seed(1)
    }

    pub fn starter_with_seed(seed: u64) -> Self {
        let mut this = Self {
            next_id: seed.max(1),
            panels: BTreeMap::new(),
            layout: LayoutNode::Panel(PanelId(0)),
            focused: PanelId(0),
        };
        let chat = this.allocate_panel(PanelKind::Chat);
        this.layout = LayoutNode::Panel(chat);
        this.focused = chat;
        this
    }

    #[cfg(test)]
    pub fn panels(&self) -> &BTreeMap<PanelId, PanelState> {
        &self.panels
    }

    pub fn layout(&self) -> &LayoutNode {
        &self.layout
    }

    pub fn focused(&self) -> PanelId {
        self.focused
    }

    pub fn panel_ids(&self) -> Vec<PanelId> {
        let mut ids = Vec::new();
        self.layout.panel_ids(&mut ids);
        ids
    }

    pub fn panel(&self, id: PanelId) -> Option<&PanelState> {
        self.panels.get(&id)
    }

    pub fn focus(&mut self, id: PanelId) {
        if self.panels.contains_key(&id) {
            self.focused = id;
        }
    }

    pub fn focus_next(&mut self) {
        let mut ids = Vec::new();
        self.layout.panel_ids(&mut ids);
        if ids.is_empty() {
            return;
        }
        let index = ids.iter().position(|id| *id == self.focused).unwrap_or(0);
        self.focused = ids[(index + 1) % ids.len()];
    }

    pub fn split_focused(&mut self, axis: SplitAxis, kind: PanelKind) -> PanelId {
        let panel = self.allocate_panel(kind);
        if !self.layout.split_panel(self.focused, axis, panel) {
            self.layout = LayoutNode::Panel(panel);
        }
        self.focused = panel;
        panel
    }

    fn allocate_panel(&mut self, kind: PanelKind) -> PanelId {
        let id = PanelId(self.next_id);
        self.next_id += 1;
        self.panels.insert(
            id,
            PanelState {
                id,
                kind,
                title: kind.title().to_owned(),
            },
        );
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_layout_contains_only_chat() {
        let workspace = PanelWorkspace::starter();
        let kinds = workspace
            .panels()
            .values()
            .map(|panel| panel.kind)
            .collect::<Vec<_>>();
        assert_eq!(kinds, vec![PanelKind::Chat]);
    }

    #[test]
    fn split_focused_adds_and_focuses_panel() {
        let mut workspace = PanelWorkspace::starter();
        let id = workspace.split_focused(SplitAxis::Horizontal, PanelKind::ScratchCanvas);
        assert_eq!(workspace.focused(), id);
        assert_eq!(
            workspace.panel(id).map(|panel| panel.kind),
            Some(PanelKind::ScratchCanvas)
        );
        assert!(workspace.layout().contains(id));
    }
}
