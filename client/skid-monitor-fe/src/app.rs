use crate::config;
use crate::platform::Ingress;
use crate::state::DashboardState;
use crate::view::{ControlRoomUiState, ControlRoomView};
use eframe::egui;

pub(crate) struct ControlRoomApp {
    ingress: Ingress,
    pub(crate) state: DashboardState,
    pub(crate) ui: ControlRoomUiState,
}

impl ControlRoomApp {
    pub(crate) fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.global_style_mut(|style| {
            style.spacing.item_spacing = config::GLOBAL_ITEM_SPACING;
            style.spacing.button_padding = config::GLOBAL_BUTTON_PADDING;
        });

        #[cfg(all(not(target_arch = "wasm32"), feature = "high-spec"))]
        if !crate::components::avatar::install_vrm_renderer(cc) {
            log::warn!(
                "high-spec frontend started without a WGPU render state; VRM will fall back"
            );
        }

        let ingress = Ingress::start(&cc.egui_ctx);

        let mut state = DashboardState::new();
        state.set_ingress_control(ingress.control());
        let ui_state = ControlRoomUiState::new(
            &cc.egui_ctx,
            state.avatar_profile(),
            state.avatar_profile_revision(),
            state.avatar_model_revision(),
        );

        Self {
            ingress,
            state,
            ui: ui_state,
        }
    }
}

impl eframe::App for ControlRoomApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.state.drain_ingress(&mut self.ingress);
        ControlRoomView::new(&mut self.state, &mut self.ui).show(ui);
    }
}
