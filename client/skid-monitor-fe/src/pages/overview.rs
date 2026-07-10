use crate::components::{
    agents::{self, AddAgentDraft, AgentNotice, AgentOverviewAction, ListenerDraft},
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
    listener_draft: ListenerDraft,
    agent_filter: String,
    pending_remove_key: Option<String>,
    pending_remove_listener: Option<String>,
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
    RequestRemove(String),
    ConfirmRemove(String),
    CancelRemove,
    SaveListener(String),
    RequestRemoveListener(String),
    ConfirmRemoveListener(String),
    CancelRemoveListener,
}

impl Default for OverviewState {
    fn default() -> Self {
        Self {
            add_agent_open: false,
            add_agent_draft: AddAgentDraft::default(),
            add_agent_notice: None,
            listener_draft: ListenerDraft::default(),
            agent_filter: String::new(),
            pending_remove_key: None,
            pending_remove_listener: None,
        }
    }
}

impl OverviewState {
    pub(crate) fn select_agent(&mut self) {
        self.add_agent_notice = None;
        self.pending_remove_key = None;
        self.pending_remove_listener = None;
    }

    pub(crate) fn start_add(&mut self) {
        self.add_agent_open = true;
        self.add_agent_notice = None;
        self.pending_remove_key = None;
        self.pending_remove_listener = None;
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

    pub(crate) fn request_remove(&mut self, key: String) {
        self.pending_remove_key = Some(key);
        self.pending_remove_listener = None;
        self.add_agent_notice = None;
    }

    pub(crate) fn cancel_remove(&mut self) {
        self.pending_remove_key = None;
    }

    pub(crate) fn removed_agent(&mut self) {
        self.pending_remove_key = None;
        self.add_agent_notice = Some(AddAgentNotice {
            message: "agent removed".to_string(),
            is_error: false,
        });
    }

    pub(crate) fn bound_listener(&mut self) {
        self.listener_draft.clear();
        self.pending_remove_listener = None;
        self.add_agent_notice = Some(AddAgentNotice {
            message: "listener bind requested".to_string(),
            is_error: false,
        });
    }

    pub(crate) fn request_remove_listener(&mut self, addr: String) {
        self.pending_remove_listener = Some(addr);
        self.pending_remove_key = None;
        self.add_agent_notice = None;
    }

    pub(crate) fn cancel_remove_listener(&mut self) {
        self.pending_remove_listener = None;
    }

    pub(crate) fn removed_listener(&mut self) {
        self.pending_remove_listener = None;
        self.add_agent_notice = Some(AddAgentNotice {
            message: "listener removal requested".to_string(),
            is_error: false,
        });
    }

    pub(crate) fn rejected_agent(&mut self, error: String) {
        self.add_agent_open = true;
        self.pending_remove_key = None;
        self.pending_remove_listener = None;
        self.add_agent_notice = Some(AddAgentNotice {
            message: error,
            is_error: true,
        });
    }

    pub(crate) fn rejected_remove(&mut self, error: String) {
        self.pending_remove_key = None;
        self.pending_remove_listener = None;
        self.add_agent_notice = Some(AddAgentNotice {
            message: error,
            is_error: true,
        });
    }

    pub(crate) fn rejected_listener(&mut self, error: String) {
        self.pending_remove_listener = None;
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
    let gap = crate::components::layout::section_gap(ui);
    let main_min = config::AGENTS_PANEL_HEIGHT_MIN;
    let event_min = config::EVENT_LOG_HEIGHT_MIN;
    let available = (limits.main_height + limits.event_log_height).max(main_min + event_min);
    let flex_id = ui.make_persistent_id("overview-agents-event-flex");
    let mut flex = ui.ctx().data_mut(|data| {
        data.get_temp::<[f32; 2]>(flex_id).unwrap_or([
            limits.main_height.max(main_min),
            limits.event_log_height.max(event_min),
        ])
    });
    let total = flex[0] + flex[1];
    let mut agents_height =
        (available * flex[0] / total.max(f32::EPSILON)).clamp(main_min, available - event_min);
    let mut event_height = available - agents_height;
    let notice = page.add_agent_notice.as_ref().map(|notice| AgentNotice {
        message: notice.message.as_str(),
        is_error: notice.is_error,
    });
    let events = state.events().iter().collect::<Vec<_>>();
    let mut action = None;

    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        action = agents::show(
            ui,
            compact,
            state.nodes(),
            state.listeners(),
            state.edge_decorations(),
            &mut page.add_agent_draft,
            &mut page.listener_draft,
            &mut page.agent_filter,
            page.add_agent_open,
            page.pending_remove_key.as_deref(),
            page.pending_remove_listener.as_deref(),
            notice,
            panel_width,
            agents_height,
        )
        .map(OverviewAction::from);

        let delta = crate::components::layout::vertical_resize_handle(ui, panel_width, gap);
        event_log::show(ui, panel_width, event_height, &events);

        if delta != 0.0 {
            agents_height = (agents_height + delta).clamp(main_min, available - event_min);
            event_height = available - agents_height;
            flex = [agents_height, event_height];
        }
    });
    ui.ctx().data_mut(|data| data.insert_temp(flex_id, flex));

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
            AgentOverviewAction::RequestRemove(key) => Self::RequestRemove(key),
            AgentOverviewAction::ConfirmRemove(key) => Self::ConfirmRemove(key),
            AgentOverviewAction::CancelRemove => Self::CancelRemove,
            AgentOverviewAction::SaveListener(addr) => Self::SaveListener(addr),
            AgentOverviewAction::RequestRemoveListener(addr) => Self::RequestRemoveListener(addr),
            AgentOverviewAction::ConfirmRemoveListener(addr) => Self::ConfirmRemoveListener(addr),
            AgentOverviewAction::CancelRemoveListener => Self::CancelRemoveListener,
        }
    }
}
