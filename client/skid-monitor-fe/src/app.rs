use crate::config;
use crate::state::DashboardState;
use crate::view::{ControlRoomUiState, ControlRoomView};
use eframe::egui;
use skid_monitor_client::receiver_loop::{ReceiverMessage, spawn_receiver_with_notify};
use std::sync::mpsc::Receiver;

pub(crate) struct ControlRoomApp {
    rx: Receiver<ReceiverMessage>,
    pub(crate) state: DashboardState,
    pub(crate) ui: ControlRoomUiState,
}

impl ControlRoomApp {
    pub(crate) fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ui_state = ControlRoomUiState::new(&cc.egui_ctx);
        cc.egui_ctx.global_style_mut(|style| {
            style.spacing.item_spacing = config::GLOBAL_ITEM_SPACING;
            style.spacing.button_padding = config::GLOBAL_BUTTON_PADDING;
        });

        let ctx = cc.egui_ctx.clone();

        Self {
            rx: spawn_receiver_with_notify(move || ctx.request_repaint()),
            state: DashboardState::new(),
            ui: ui_state,
        }
    }
}

impl eframe::App for ControlRoomApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.state.drain_messages(&self.rx);
        ControlRoomView::new(&mut self.state, &mut self.ui).show(ui);
    }
}
