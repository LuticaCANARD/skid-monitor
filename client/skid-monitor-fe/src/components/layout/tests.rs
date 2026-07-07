use super::*;

fn test_section_gap() -> f32 {
    config::SECTION_GAP + config::GLOBAL_ITEM_SPACING.y
}

fn assert_close(left: f32, right: f32) {
    assert!(
        (left - right).abs() < 0.001,
        "expected {left} to match {right}"
    );
}

#[test]
fn split_main_columns_end_at_the_same_height() {
    let gap = test_section_gap();
    let limits = PanelLimits::for_remaining_height(520.0, LayoutMode::Split, gap);

    assert_close(
        limits.sources_height + gap + limits.trends_height,
        limits.metrics_height,
    );
}

#[test]
fn stacked_main_panels_include_actual_gaps_in_the_height_budget() {
    let gap = test_section_gap();
    let limits = PanelLimits::for_remaining_height(760.0, LayoutMode::Stacked, gap);

    assert_close(
        limits.sources_height + gap + limits.trends_height + gap + limits.metrics_height,
        config::MAIN_AREA_HEIGHT,
    );
}
