//! What the left sidebar shows. The saved form keeps the old fields
//! (`left_tool`, `left_agents`) so older files and older GUIs read it, but
//! code reads and writes one value, and Agents has one encoding.
use super::{LayoutPreset, SidebarTool, UiPreferences};

/// Content of the left sidebar. Its visibility (`left_visible`) is
/// separate, so hiding and showing it returns to the same content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeftContent {
    Projects,
    Agents,
    Tool(SidebarTool),
}

impl LeftContent {
    /// The content that hosts this tool. Agents has its own variant.
    pub fn of_tool(tool: SidebarTool) -> Self {
        match tool {
            SidebarTool::Agents => Self::Agents,
            tool => Self::Tool(tool),
        }
    }

    /// Old files store Agents either as `left_tool: "Agents"` or as
    /// `left_agents` with no tool; a tool wins over a stale flag.
    fn decode(tool: Option<SidebarTool>, agents: bool) -> Self {
        match tool {
            Some(tool) => Self::of_tool(tool),
            None if agents => Self::Agents,
            None => Self::Projects,
        }
    }

    /// The one saved form: Agents is `left_agents` with no tool, which
    /// every earlier GUI reads as Agents.
    fn encode(self) -> (Option<SidebarTool>, bool) {
        match self {
            Self::Projects => (None, false),
            Self::Agents | Self::Tool(SidebarTool::Agents) => (None, true),
            Self::Tool(tool) => (Some(tool), false),
        }
    }
}

impl UiPreferences {
    pub fn left_content(&self) -> LeftContent {
        LeftContent::decode(self.left_tool, self.left_agents)
    }

    pub fn set_left_content(&mut self, content: LeftContent) {
        (self.left_tool, self.left_agents) = content.encode();
    }

    /// The left sidebar is open on this tool (Agents included).
    pub fn left_shows(&self, tool: SidebarTool) -> bool {
        self.left_visible && self.left_content() == LeftContent::of_tool(tool)
    }

    /// Rewrite loaded fields in the one encoding, presets included.
    pub(super) fn normalize_left(&mut self) {
        self.set_left_content(self.left_content());
        for preset in self.named_layouts.values_mut() {
            (preset.left_tool, preset.left_agents) = preset.left_content().encode();
        }
    }
}

impl LayoutPreset {
    pub fn left_content(&self) -> LeftContent {
        LeftContent::decode(self.left_tool, self.left_agents)
    }

    pub(super) fn set_left_content(&mut self, content: LeftContent) {
        (self.left_tool, self.left_agents) = content.encode();
    }
}
