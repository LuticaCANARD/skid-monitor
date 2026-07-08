use crate::components::{
    agents::{self, AddAgentDraft, AgentNotice, AgentOverviewAction},
    event_log,
    layout::PanelLimits,
};
use crate::config;
use crate::state::DashboardState;
use eframe::egui;

pub(crate) struct OverviewState {
    add_agent_open: bool,
    add_agent_draft: AddAgentDraft,
    add_agent_notice: Option<AddAgentNotice>,
}

struct AddAgentNotice {
    message: String,
    is_error: bool,
}

pub(crate) enum OverviewAction {
    Select(String),
    StartAdd,
    CancelAdd,
    SaveAdd {
        endpoint: String,
        node: String,
        service: String,
    },
    Remove(String),
}

impl Default for OverviewState {
    fn default() -> Self {
        Self {
            add_agent_open: false,
            add_agent_draft: AddAgentDraft::default(),
            add_agent_notice: None,
        }
    }
}

impl OverviewState {
    pub(crate) fn select_agent(&mut self) {
        self.add_agent_notice = None;
    }

    pub(crate) fn start_add(&mut self) {
        self.add_agent_open = true;
        self.add_agent_notice = None;
    }

    pub(crate) fn cancel_add(&mut self) {
        self.add_agent_open = false;
        self.add_agent_notice = None;
        self.add_agent_draft.clear();
    }

    pub(crate) fn registered_agent(&mut self) {
        self.add_agent_open = false;
        self.add_agent_draft.clear();
        self.add_agent_notice = Some(AddAgentNotice {
            message: "agent registered".to_string(),
            is_error: false,
        });
    }

    pub(crate) fn rejected_agent(&mut self, error: String) {
        self.add_agent_open = true;
        self.add_agent_notice = Some(AddAgentNotice {
            message: error,
            is_error: true,
        });
    }
}

pub(crate) fn show(
    ui: &mut egui::Ui,
    compact: bool,
    panel_width: f32,
    limits: PanelLimits,
    state: &DashboardState,
    page: &mut OverviewState,
) -> Option<OverviewAction> {
    let notice = page.add_agent_notice.as_ref().map(|notice| AgentNotice {
        message: notice.message.as_str(),
        is_error: notice.is_error,
    });
    let action = agents::show(
        ui,
        compact,
        state.nodes(),
        state.edge_decorations(),
        &mut page.add_agent_draft,
        page.add_agent_open,
        notice,
        panel_width,
        limits.main_height,
    )
    .map(OverviewAction::from);

    ui.add_space(config::SECTION_GAP);
    let events = state.events().iter().collect::<Vec<_>>();
    event_log::show(ui, panel_width, limits.event_log_height, &events);

    action
}

impl From<AgentOverviewAction> for OverviewAction {
    fn from(action: AgentOverviewAction) -> Self {
        match action {
            AgentOverviewAction::Select(key) => Self::Select(key),
            AgentOverviewAction::StartAdd => Self::StartAdd,
            AgentOverviewAction::CancelAdd => Self::CancelAdd,
            AgentOverviewAction::SaveAdd {
                endpoint,
                node,
                service,
            } => Self::SaveAdd {
                endpoint,
                node,
                service,
            },
            AgentOverviewAction::Remove(key) => Self::Remove(key),
        }
    }
}
