use crate::model::AvatarReactionProfile;
use eframe::egui;
#[cfg(all(not(target_arch = "wasm32"), feature = "high-spec"))]
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
const MAX_IMAGE_FILE_BYTES: u64 = 32 * 1024 * 1024;
#[cfg(not(target_arch = "wasm32"))]
const MAX_MODEL_DIMENSION: u32 = 4096;
#[cfg(not(target_arch = "wasm32"))]
const MAX_MODEL_DECODE_BYTES: u64 = 64 * 1024 * 1024;
#[cfg(not(target_arch = "wasm32"))]
const MODEL_LOAD_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub(crate) struct AvatarModelCache {
    requested_path: Option<String>,
    asset: Option<AvatarModelAsset>,
    error: Option<String>,
    #[cfg(not(target_arch = "wasm32"))]
    generation: u64,
    #[cfg(not(target_arch = "wasm32"))]
    active_generation: Option<u64>,
    #[cfg(not(target_arch = "wasm32"))]
    loader: AvatarModelLoader,
}

pub(super) struct AvatarModelImage {
    texture: egui::TextureHandle,
    size: egui::Vec2,
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
enum AvatarModelAsset {
    Image(AvatarModelImage),
    #[cfg(all(not(target_arch = "wasm32"), feature = "high-spec"))]
    Vrm(Arc<super::vrm::CpuVrmScene>),
}

#[cfg(not(target_arch = "wasm32"))]
struct AvatarModelLoader {
    request_tx: Sender<AvatarModelLoadRequest>,
    result_rx: Receiver<AvatarModelLoadResult>,
}

#[cfg(not(target_arch = "wasm32"))]
struct AvatarModelLoadRequest {
    generation: u64,
    path: String,
}

#[cfg(not(target_arch = "wasm32"))]
struct AvatarModelLoadResult {
    generation: u64,
    result: Result<DecodedAvatarModel, String>,
}

#[cfg(not(target_arch = "wasm32"))]
enum DecodedAvatarModel {
    Image {
        image: egui::ColorImage,
        size: egui::Vec2,
    },
    #[cfg(all(not(target_arch = "wasm32"), feature = "high-spec"))]
    Vrm(super::vrm::CpuVrmScene),
}

impl Default for AvatarModelCache {
    fn default() -> Self {
        Self {
            requested_path: None,
            asset: None,
            error: None,
            #[cfg(not(target_arch = "wasm32"))]
            generation: 0,
            #[cfg(not(target_arch = "wasm32"))]
            active_generation: None,
            #[cfg(not(target_arch = "wasm32"))]
            loader: AvatarModelLoader::start(),
        }
    }
}

impl AvatarModelCache {
    /// Synchronizes the cached native sprite with the profile's model path.
    /// Repeated calls with the same path do not touch the filesystem again.
    pub(crate) fn sync(&mut self, _ctx: &egui::Context, profile: &AvatarReactionProfile) {
        let requested_path = profile.model_path.trim();
        if self.requested_path.as_deref() != Some(requested_path) {
            self.requested_path = Some(requested_path.to_string());
            self.asset = None;
            self.error = None;
            #[cfg(not(target_arch = "wasm32"))]
            {
                self.generation = self.generation.wrapping_add(1);
            }

            if !requested_path.is_empty() {
                #[cfg(target_arch = "wasm32")]
                {
                    self.error = Some(
                        "custom character model paths are available only in native builds"
                            .to_string(),
                    );
                }
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            self.poll_loader(_ctx);
            self.start_load_if_needed();
            if self.loading() {
                _ctx.request_repaint_after(MODEL_LOAD_POLL_INTERVAL);
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn poll_loader(&mut self, ctx: &egui::Context) {
        let completed = match self.loader.result_rx.try_recv() {
            Ok(completed) => completed,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.active_generation = None;
                if !self
                    .requested_path
                    .as_deref()
                    .unwrap_or_default()
                    .is_empty()
                {
                    self.error = Some(
                        "character model loader stopped before returning a result".to_string(),
                    );
                }
                return;
            }
        };
        self.active_generation = None;

        if completed.generation != self.generation {
            return;
        }

        match completed.result {
            Ok(DecodedAvatarModel::Image { image, size }) => {
                let path = self.requested_path.as_deref().unwrap_or("built-in");
                let texture = ctx.load_texture(
                    format!("avatar-model:{path}"),
                    image,
                    egui::TextureOptions::LINEAR,
                );
                self.asset = Some(AvatarModelAsset::Image(AvatarModelImage { texture, size }));
            }
            #[cfg(all(not(target_arch = "wasm32"), feature = "high-spec"))]
            Ok(DecodedAvatarModel::Vrm(scene)) => {
                self.asset = Some(AvatarModelAsset::Vrm(Arc::new(scene)));
            }
            Err(error) => self.error = Some(error),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn start_load_if_needed(&mut self) {
        let path = self.requested_path.as_deref().unwrap_or_default();
        if path.is_empty()
            || self.asset.is_some()
            || self.error.is_some()
            || self.active_generation.is_some()
        {
            return;
        }

        let request = AvatarModelLoadRequest {
            generation: self.generation,
            path: path.to_string(),
        };
        match self.loader.request_tx.send(request) {
            Ok(()) => self.active_generation = Some(self.generation),
            Err(error) => {
                self.error = Some(format!("failed to queue character model load: {error}"));
            }
        }
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(crate) fn requested_path(&self) -> Option<&str> {
        self.requested_path.as_deref()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn loading(&self) -> bool {
        !self
            .requested_path
            .as_deref()
            .unwrap_or_default()
            .is_empty()
            && self.asset.is_none()
            && self.error.is_none()
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn loading(&self) -> bool {
        false
    }

    /// Forces the current path to be decoded again on the next [`Self::sync`].
    /// This supports replacing or creating an image without changing its path.
    pub(crate) fn invalidate(&mut self) {
        self.requested_path = None;
        self.asset = None;
        self.error = None;
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.generation = self.generation.wrapping_add(1);
        }
    }

    pub(super) fn image(&self) -> Option<&AvatarModelImage> {
        match self.asset.as_ref() {
            Some(AvatarModelAsset::Image(image)) => Some(image),
            #[cfg(all(not(target_arch = "wasm32"), feature = "high-spec"))]
            Some(AvatarModelAsset::Vrm(_)) | None => None,
            #[cfg(not(all(not(target_arch = "wasm32"), feature = "high-spec")))]
            None => None,
        }
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "high-spec"))]
    pub(super) fn vrm_scene(&self) -> Option<&Arc<super::vrm::CpuVrmScene>> {
        match self.asset.as_ref() {
            Some(AvatarModelAsset::Vrm(scene)) => Some(scene),
            Some(AvatarModelAsset::Image(_)) | None => None,
        }
    }

    pub(crate) fn loaded_label(&self) -> Option<&'static str> {
        match self.asset.as_ref() {
            Some(AvatarModelAsset::Image(_)) => Some("2D sprite"),
            #[cfg(all(not(target_arch = "wasm32"), feature = "high-spec"))]
            Some(AvatarModelAsset::Vrm(scene)) => Some(scene.version_label),
            None => None,
        }
    }
}

impl AvatarModelImage {
    pub(super) fn texture_id(&self) -> egui::TextureId {
        self.texture.id()
    }

    pub(super) fn size(&self) -> egui::Vec2 {
        self.size
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl AvatarModelLoader {
    fn start() -> Self {
        let (request_tx, request_rx) = mpsc::channel::<AvatarModelLoadRequest>();
        let (result_tx, result_rx) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(request) = request_rx.recv() {
                let result = std::panic::catch_unwind(|| {
                    decode_avatar_model(&request.path, request.generation)
                })
                .unwrap_or_else(|_| {
                    Err("character model decoder rejected malformed input".to_string())
                });
                if result_tx
                    .send(AvatarModelLoadResult {
                        generation: request.generation,
                        result,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            request_tx,
            result_rx,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn decode_avatar_model(path: &str, generation: u64) -> Result<DecodedAvatarModel, String> {
    if has_vrm_extension(path) {
        #[cfg(feature = "high-spec")]
        {
            return super::vrm::decode(path, generation).map(DecodedAvatarModel::Vrm);
        }
        #[cfg(not(feature = "high-spec"))]
        {
            let _ = generation;
            return Err(
                "VRM rendering requires the native high-spec build; using built-in fallback"
                    .to_string(),
            );
        }
    }

    decode_avatar_image(path)
}

#[cfg(not(target_arch = "wasm32"))]
fn decode_avatar_image(path: &str) -> Result<DecodedAvatarModel, String> {
    if !has_supported_image_extension(path) {
        return Err("character model must use a .png, .jpg, .jpeg, or .vrm extension".to_string());
    }

    let file_bytes = std::fs::metadata(path)
        .map_err(|error| format!("failed to inspect character image {path}: {error}"))?
        .len();
    if file_bytes > MAX_IMAGE_FILE_BYTES {
        return Err(format!(
            "character image exceeds the {} MiB file limit",
            MAX_IMAGE_FILE_BYTES / 1024 / 1024
        ));
    }

    let mut reader = image::ImageReader::open(path)
        .map_err(|error| format!("failed to open character image {path}: {error}"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_MODEL_DIMENSION);
    limits.max_image_height = Some(MAX_MODEL_DIMENSION);
    limits.max_alloc = Some(MAX_MODEL_DECODE_BYTES);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|error| format!("failed to decode character image {path}: {error}"))?
        .to_rgba8();
    let width = usize::try_from(decoded.width()).map_err(|_| "image width is too large")?;
    let height = usize::try_from(decoded.height()).map_err(|_| "image height is too large")?;
    if width == 0 || height == 0 {
        return Err("character image must not be empty".to_string());
    }

    let image = egui::ColorImage::from_rgba_unmultiplied([width, height], decoded.as_raw());

    Ok(DecodedAvatarModel::Image {
        image,
        size: egui::vec2(width as f32, height as f32),
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn has_vrm_extension(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("vrm"))
}

#[cfg(not(target_arch = "wasm32"))]
fn has_supported_image_extension(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg"
            )
        })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn empty_path_uses_built_in_model_without_an_error() {
        let mut cache = AvatarModelCache::default();
        cache.sync(&egui::Context::default(), &AvatarReactionProfile::default());

        assert!(cache.image().is_none());
        assert!(cache.error().is_none());
    }

    #[test]
    fn native_png_is_loaded_cached_and_reloadable() {
        let path = test_image_path("load");
        image::RgbaImage::from_pixel(2, 3, image::Rgba([12, 34, 56, 255]))
            .save(&path)
            .expect("write png fixture");
        let profile = AvatarReactionProfile {
            model_path: path.display().to_string(),
            ..AvatarReactionProfile::default()
        };
        let mut cache = AvatarModelCache::default();
        let ctx = egui::Context::default();

        wait_for_load(&mut cache, &ctx, &profile);

        assert!(cache.image().is_some());
        assert!(cache.error().is_none());
        assert_eq!(cache.image().expect("image").size(), egui::vec2(2.0, 3.0));

        image::RgbaImage::from_pixel(4, 1, image::Rgba([65, 43, 21, 255]))
            .save(&path)
            .expect("replace png fixture");
        cache.sync(&ctx, &profile);
        assert_eq!(
            cache.image().expect("cached image").size(),
            egui::vec2(2.0, 3.0)
        );

        cache.invalidate();
        wait_for_load(&mut cache, &ctx, &profile);
        assert_eq!(
            cache.image().expect("reloaded image").size(),
            egui::vec2(4.0, 1.0)
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn failed_load_keeps_the_built_in_fallback_available() {
        let path = test_image_path("missing");
        let profile = AvatarReactionProfile {
            model_path: path.display().to_string(),
            ..AvatarReactionProfile::default()
        };
        let mut cache = AvatarModelCache::default();

        let ctx = egui::Context::default();
        wait_for_load(&mut cache, &ctx, &profile);

        assert!(cache.image().is_none());
        assert!(cache.error().is_some());
    }

    #[test]
    fn path_changes_are_serialized_and_only_the_latest_result_is_applied() {
        let first_path = test_image_path("serial-first");
        let second_path = test_image_path("serial-second");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([12, 34, 56, 255]))
            .save(&first_path)
            .expect("write first png fixture");
        image::RgbaImage::from_pixel(5, 1, image::Rgba([65, 43, 21, 255]))
            .save(&second_path)
            .expect("write second png fixture");
        let first = AvatarReactionProfile {
            model_path: first_path.display().to_string(),
            ..AvatarReactionProfile::default()
        };
        let second = AvatarReactionProfile {
            model_path: second_path.display().to_string(),
            ..AvatarReactionProfile::default()
        };
        let mut cache = AvatarModelCache::default();
        let ctx = egui::Context::default();

        cache.sync(&ctx, &first);
        cache.sync(&ctx, &second);
        wait_for_load(&mut cache, &ctx, &second);

        assert_eq!(
            cache.image().expect("latest image").size(),
            egui::vec2(5.0, 1.0)
        );
        let _ = std::fs::remove_file(first_path);
        let _ = std::fs::remove_file(second_path);
    }

    #[test]
    fn oversized_model_dimensions_use_the_built_in_fallback() {
        let path = test_image_path("oversized");
        image::RgbaImage::from_pixel(MAX_MODEL_DIMENSION + 1, 1, image::Rgba([12, 34, 56, 255]))
            .save(&path)
            .expect("write oversized png fixture");
        let profile = AvatarReactionProfile {
            model_path: path.display().to_string(),
            ..AvatarReactionProfile::default()
        };
        let mut cache = AvatarModelCache::default();

        let ctx = egui::Context::default();
        wait_for_load(&mut cache, &ctx, &profile);

        assert!(cache.image().is_none());
        assert!(cache.error().is_some());
        let _ = std::fs::remove_file(path);
    }

    #[cfg(not(feature = "high-spec"))]
    #[test]
    fn low_spec_rejects_vrm_without_touching_the_file() {
        let result = decode_avatar_model("/path/that/does/not/exist/avatar.vrm", 1);

        let error = match result {
            Ok(_) => panic!("low-spec must not load VRM"),
            Err(error) => error,
        };
        assert!(error.contains("high-spec"));
    }

    fn test_image_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "skid-monitor-avatar-{name}-{}.png",
            std::process::id()
        ))
    }

    fn wait_for_load(
        cache: &mut AvatarModelCache,
        ctx: &egui::Context,
        profile: &AvatarReactionProfile,
    ) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            cache.sync(ctx, profile);
            if !cache.loading() {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "avatar image load did not complete"
            );
            std::thread::yield_now();
        }
    }
}
