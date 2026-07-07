use crate::config;
use eframe::egui;
use std::path::Path;

pub(super) struct BackgroundImage {
    texture: egui::TextureHandle,
    size: egui::Vec2,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum BackgroundTheme {
    Obsidian,
    Graphite,
    DeepGreen,
    Paper,
    Porcelain,
}

pub(super) const BACKGROUND_THEMES: [BackgroundTheme; 5] = [
    BackgroundTheme::Obsidian,
    BackgroundTheme::Graphite,
    BackgroundTheme::DeepGreen,
    BackgroundTheme::Paper,
    BackgroundTheme::Porcelain,
];

impl BackgroundTheme {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Obsidian => "Obsidian",
            Self::Graphite => "Graphite",
            Self::DeepGreen => "Deep green",
            Self::Paper => "Paper",
            Self::Porcelain => "Porcelain",
        }
    }

    pub(super) fn fill(self) -> egui::Color32 {
        match self {
            Self::Obsidian => config::PAGE_BACKGROUND,
            Self::Graphite => egui::Color32::from_rgb(22, 23, 25),
            Self::DeepGreen => egui::Color32::from_rgb(12, 24, 21),
            Self::Paper => egui::Color32::from_rgb(241, 244, 248),
            Self::Porcelain => egui::Color32::from_rgb(250, 251, 253),
        }
    }
}

pub(super) fn background_theme_row(
    ui: &mut egui::Ui,
    selected: bool,
    theme: BackgroundTheme,
) -> egui::Response {
    ui.horizontal(|ui| {
        let desired = egui::vec2(24.0, 24.0);
        let (rect, swatch_response) = ui.allocate_exact_size(desired, egui::Sense::click());
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(4), theme.fill());
        ui.painter().rect_stroke(
            rect,
            egui::CornerRadius::same(4),
            egui::Stroke::new(
                if selected { 2.0 } else { 1.0 },
                if selected {
                    config::STATUS_LISTENING_COLOR
                } else {
                    config::STAT_TILE_BORDER
                },
            ),
            egui::StrokeKind::Inside,
        );
        let label_response = ui.selectable_label(selected, theme.label());
        swatch_response.union(label_response)
    })
    .inner
}

pub(super) fn load_background_texture(
    ctx: &egui::Context,
    path: impl AsRef<Path>,
) -> Result<BackgroundImage, String> {
    let path = path.as_ref();
    let image = image::open(path)
        .map_err(|error| format!("failed to load {}: {error}", path.display()))?
        .to_rgba8();
    let width = usize::try_from(image.width()).map_err(|_| "image width is too large")?;
    let height = usize::try_from(image.height()).map_err(|_| "image height is too large")?;
    let color_image = egui::ColorImage::from_rgba_unmultiplied([width, height], image.as_raw());
    let texture = ctx.load_texture(
        format!("background-image:{}", path.display()),
        color_image,
        egui::TextureOptions::LINEAR,
    );

    Ok(BackgroundImage {
        texture,
        size: egui::vec2(width as f32, height as f32),
    })
}

pub(super) fn paint_cover_image(ui: &egui::Ui, image: &BackgroundImage) {
    let rect = ui.max_rect();
    if image.size.x <= 0.0 || image.size.y <= 0.0 || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }

    let image_aspect = image.size.x / image.size.y;
    let rect_aspect = rect.width() / rect.height();
    let uv = if image_aspect > rect_aspect {
        let visible_width = (rect_aspect / image_aspect).clamp(0.0, 1.0);
        let inset = (1.0 - visible_width) * 0.5;
        egui::Rect::from_min_max(egui::pos2(inset, 0.0), egui::pos2(1.0 - inset, 1.0))
    } else {
        let visible_height = (image_aspect / rect_aspect).clamp(0.0, 1.0);
        let inset = (1.0 - visible_height) * 0.5;
        egui::Rect::from_min_max(egui::pos2(0.0, inset), egui::pos2(1.0, 1.0 - inset))
    };

    ui.painter()
        .image(image.texture.id(), rect, uv, egui::Color32::WHITE);
}

pub(super) fn dropped_image_path(ctx: &egui::Context) -> Option<String> {
    ctx.input(|input| {
        input
            .raw
            .dropped_files
            .iter()
            .find_map(|file| file.path.as_ref())
            .map(|path| path.display().to_string())
    })
}
