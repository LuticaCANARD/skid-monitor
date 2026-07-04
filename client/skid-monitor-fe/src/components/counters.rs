use crate::components::primitives::stat_tile;
use crate::config;
use crate::model::SignalCounters;
use eframe::egui;

pub(crate) fn show(ui: &mut egui::Ui, counters: &SignalCounters) {
    let stats = [
        ("metric batches", counters.metrics),
        ("metric points", counters.metric_points),
        ("trace batches", counters.traces),
        ("spans", counters.spans),
        ("log batches", counters.logs),
        ("log records", counters.log_records),
    ];
    let spacing = ui.spacing().item_spacing.x;
    let columns = counter_columns(ui.available_width(), spacing, stats.len());
    let total_spacing = spacing * columns.saturating_sub(1) as f32;
    let tile_width = ((ui.available_width() - total_spacing) / columns as f32).clamp(
        config::COUNTER_TILE_MIN_WIDTH,
        config::COUNTER_TILE_MAX_WIDTH,
    );
    let grid_width = tile_width * columns as f32 + total_spacing;

    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(((ui.available_width() - grid_width) * 0.5).max(0.0));
        egui::Grid::new("counter-grid")
            .num_columns(columns)
            .spacing(config::COUNTER_GRID_SPACING)
            .show(ui, |ui| {
                for (index, (label, value)) in stats.iter().copied().enumerate() {
                    stat_tile(ui, label, value, tile_width);
                    if (index + 1) % columns == 0 {
                        ui.end_row();
                    }
                }
            });
    });
}

fn counter_columns(available_width: f32, spacing: f32, item_count: usize) -> usize {
    config::COUNTER_COLUMN_OPTIONS
        .into_iter()
        .filter(|columns| *columns <= item_count)
        .find(|columns| {
            let total_spacing = spacing * columns.saturating_sub(1) as f32;
            let required_width = config::COUNTER_TILE_MIN_WIDTH * *columns as f32 + total_spacing;
            available_width >= required_width
        })
        .unwrap_or(1)
}
