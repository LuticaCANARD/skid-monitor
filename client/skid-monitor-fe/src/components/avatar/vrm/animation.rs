use glam::{Mat4, Quat, Vec3, Vec4};
use gltf::animation::{Interpolation, Property, util::ReadOutputs};
use std::collections::{HashMap, HashSet};

const MAX_ANIMATION_CHANNELS: usize = 1024;
const MAX_ANIMATION_KEYFRAMES: usize = 1_000_000;
const MAX_ANIMATION_CLIPS: usize = 64;
const MAX_TOTAL_ANIMATION_KEYFRAMES: usize = 2_000_000;
const MAX_ANIMATION_DURATION_SECONDS: f32 = 24.0 * 60.0 * 60.0;

#[derive(Clone, Copy, Debug)]
pub(super) struct NodeTransform {
    pub(super) translation: Vec3,
    pub(super) rotation: Quat,
    pub(super) scale: Vec3,
}

impl NodeTransform {
    pub(super) fn from_node(node: gltf::Node<'_>) -> Result<Self, String> {
        let (translation, rotation, scale) = node.transform().decomposed();
        let translation = Vec3::from_array(translation);
        let rotation = Quat::from_array(rotation);
        let scale = Vec3::from_array(scale);
        if !translation.is_finite()
            || !rotation.is_finite()
            || rotation.length_squared() <= f32::EPSILON
            || !scale.is_finite()
            || scale.abs().min_element() <= f32::EPSILON
        {
            return Err(format!(
                "VRM node {} has an invalid decomposed transform",
                node.index()
            ));
        }
        Ok(Self {
            translation,
            rotation: rotation.normalize(),
            scale,
        })
    }

    pub(super) fn matrix(self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct RetargetTarget {
    pub(super) target_node: usize,
    pub(super) source_rest: NodeTransform,
    pub(super) target_rest: NodeTransform,
    pub(super) translation_scale: f32,
    pub(super) hips: bool,
}

#[derive(Debug)]
pub(super) struct CpuAnimationClip {
    channels: Vec<AnimationChannel>,
    duration: f32,
    pub(super) source_label: &'static str,
}

#[derive(Debug)]
struct AnimationChannel {
    target_node: usize,
    interpolation: Interpolation,
    times: Vec<f32>,
    values: AnimationValues,
}

#[derive(Debug)]
enum AnimationValues {
    Translation(Vec<Vec3>),
    Rotation(Vec<Vec4>),
    Scale(Vec<Vec3>),
}

pub(super) fn decode_clips(
    gltf: &gltf::Gltf,
    blob: &[u8],
    active_nodes: Option<&[bool]>,
    retarget: Option<&HashMap<usize, RetargetTarget>>,
) -> Result<Vec<CpuAnimationClip>, String> {
    let is_vrma = retarget.is_some();
    let mut clips = Vec::new();
    for animation in gltf.animations() {
        if let Some(clip) = decode_clip(animation, blob, active_nodes, retarget)? {
            clips.push(clip);
        }
    }
    validate_clip_collection(&clips)?;
    if clips.is_empty() && is_vrma {
        return Err("VRMA contains no humanoid skeletal animation clips".to_string());
    }
    Ok(clips)
}

pub(super) fn validate_clip_collection(clips: &[CpuAnimationClip]) -> Result<(), String> {
    if clips.len() > MAX_ANIMATION_CLIPS {
        return Err(format!(
            "animation input has more than {MAX_ANIMATION_CLIPS} clips"
        ));
    }
    let total_keyframes = clips.iter().try_fold(0_usize, |total, clip| {
        clip.channels.iter().try_fold(total, |total, channel| {
            total
                .checked_add(channel.times.len())
                .ok_or_else(|| "animation keyframe count overflowed".to_string())
        })
    })?;
    if total_keyframes > MAX_TOTAL_ANIMATION_KEYFRAMES {
        return Err(format!(
            "animation input has more than {MAX_TOTAL_ANIMATION_KEYFRAMES} total keyframes"
        ));
    }
    Ok(())
}

fn decode_clip(
    animation: gltf::Animation<'_>,
    blob: &[u8],
    active_nodes: Option<&[bool]>,
    retarget: Option<&HashMap<usize, RetargetTarget>>,
) -> Result<Option<CpuAnimationClip>, String> {
    let is_vrma = retarget.is_some();
    let mut channels = Vec::new();
    let mut total_keyframes = 0_usize;
    let mut duration = 0.0_f32;
    let mut targets = HashSet::new();

    for channel in animation.channels() {
        if channels.len() >= MAX_ANIMATION_CHANNELS {
            return Err(format!(
                "animation has more than {MAX_ANIMATION_CHANNELS} supported channels"
            ));
        }
        let source_node = channel.target().node().index();
        let property = channel.target().property();
        if property == Property::MorphTargetWeights {
            continue;
        }

        let (target_node, retarget_target) = if let Some(retarget) = retarget {
            let Some(target) = retarget.get(&source_node).copied() else {
                continue;
            };
            if property == Property::Scale {
                return Err("VRMA humanoid animation must not contain scale channels".to_string());
            }
            if property == Property::Translation && !target.hips {
                return Err("VRMA humanoid animation may translate only the hips bone".to_string());
            }
            (target.target_node, Some(target))
        } else {
            if active_nodes
                .and_then(|active| active.get(source_node))
                .copied()
                != Some(true)
            {
                continue;
            }
            (source_node, None)
        };

        let property_key = match property {
            Property::Translation => 0_u8,
            Property::Rotation => 1,
            Property::Scale => 2,
            Property::MorphTargetWeights => unreachable!(),
        };
        if !targets.insert((target_node, property_key)) {
            return Err(format!(
                "animation targets node {target_node} property {property_key} more than once"
            ));
        }

        let sampler = channel.sampler();
        let input = sampler.input();
        super::loader::validate_accessor_iteration(&input, "animation input")?;
        if input.data_type() != gltf::accessor::DataType::F32
            || input.dimensions() != gltf::accessor::Dimensions::Scalar
            || input.normalized()
        {
            return Err("animation input accessor must be non-normalized F32 SCALAR".to_string());
        }
        total_keyframes = total_keyframes
            .checked_add(input.count())
            .ok_or_else(|| "animation keyframe count overflowed".to_string())?;
        if total_keyframes > MAX_ANIMATION_KEYFRAMES {
            return Err(format!(
                "animation has more than {MAX_ANIMATION_KEYFRAMES} keyframes"
            ));
        }

        let reader = channel.reader(|buffer| match buffer.source() {
            gltf::buffer::Source::Bin => Some(blob),
            gltf::buffer::Source::Uri(_) => None,
        });
        let times = reader
            .read_inputs()
            .ok_or_else(|| "animation input data could not be decoded".to_string())?
            .collect::<Vec<_>>();
        validate_times(&times)?;
        duration = duration.max(*times.last().expect("validated non-empty animation input"));

        let output = sampler.output();
        super::loader::validate_accessor_iteration(&output, "animation output")?;
        let multiplier = if sampler.interpolation() == Interpolation::CubicSpline {
            3
        } else {
            1
        };
        let expected = times
            .len()
            .checked_mul(multiplier)
            .ok_or_else(|| "animation output count overflowed".to_string())?;
        if output.count() != expected {
            return Err(format!(
                "animation output count {} does not match {expected} values required by its input",
                output.count()
            ));
        }

        let outputs = reader
            .read_outputs()
            .ok_or_else(|| "animation output data could not be decoded".to_string())?;
        let values = match (property, outputs) {
            (Property::Translation, ReadOutputs::Translations(values)) => {
                let mut values = values.map(Vec3::from_array).collect::<Vec<_>>();
                validate_vec3_values(&values, "translation")?;
                if let Some(target) = retarget_target {
                    retarget_translations(&mut values, sampler.interpolation(), target);
                }
                AnimationValues::Translation(values)
            }
            (Property::Rotation, ReadOutputs::Rotations(values)) => {
                let mut values = values.into_f32().map(Vec4::from_array).collect::<Vec<_>>();
                validate_rotations(&mut values, sampler.interpolation())?;
                if let Some(target) = retarget_target {
                    retarget_rotations(&mut values, sampler.interpolation(), target)?;
                }
                AnimationValues::Rotation(values)
            }
            (Property::Scale, ReadOutputs::Scales(values)) => {
                let values = values.map(Vec3::from_array).collect::<Vec<_>>();
                validate_scales(&values, sampler.interpolation())?;
                AnimationValues::Scale(values)
            }
            _ => return Err("animation output accessor does not match its target property".into()),
        };
        channels.push(AnimationChannel {
            target_node,
            interpolation: sampler.interpolation(),
            times,
            values,
        });
    }

    if channels.is_empty() {
        return Ok(None);
    }
    if !duration.is_finite() || duration > MAX_ANIMATION_DURATION_SECONDS {
        return Err(format!(
            "animation duration must not exceed {MAX_ANIMATION_DURATION_SECONDS} seconds"
        ));
    }
    Ok(Some(CpuAnimationClip {
        channels,
        duration,
        source_label: if is_vrma { "VRMA" } else { "embedded glTF" },
    }))
}

impl CpuAnimationClip {
    pub(super) fn apply(&self, time: f32, transforms: &mut [NodeTransform]) {
        let sample_time = if self.duration > f32::EPSILON {
            time.rem_euclid(self.duration)
        } else {
            0.0
        };
        for channel in &self.channels {
            let Some(transform) = transforms.get_mut(channel.target_node) else {
                continue;
            };
            match &channel.values {
                AnimationValues::Translation(values) => {
                    transform.translation =
                        sample_vec3(&channel.times, values, channel.interpolation, sample_time);
                }
                AnimationValues::Rotation(values) => {
                    transform.rotation =
                        sample_rotation(&channel.times, values, channel.interpolation, sample_time);
                }
                AnimationValues::Scale(values) => {
                    transform.scale =
                        sample_vec3(&channel.times, values, channel.interpolation, sample_time);
                }
            }
        }
    }
}

pub(super) fn apply_mixed(
    clips: &[CpuAnimationClip],
    time: f32,
    crossfade_seconds: f32,
    transforms: &mut [NodeTransform],
) {
    let Some(first) = clips.first() else {
        return;
    };
    if clips.len() == 1 {
        first.apply(time, transforms);
        return;
    }

    let durations = clips
        .iter()
        .map(|clip| clip.duration.max(1.0 / 60.0))
        .collect::<Vec<_>>();
    let outgoing_fades = clips
        .iter()
        .enumerate()
        .map(|(index, _)| {
            crossfade_seconds
                .max(0.0)
                .min(durations[index] * 0.5)
                .min(durations[(index + 1) % clips.len()] * 0.5)
        })
        .collect::<Vec<_>>();
    let segment_durations = durations
        .iter()
        .enumerate()
        .map(|(index, duration)| {
            let incoming_fade = outgoing_fades[(index + clips.len() - 1) % clips.len()];
            duration - incoming_fade
        })
        .collect::<Vec<_>>();
    let total_duration = segment_durations.iter().sum::<f32>();
    let mut cursor = time.rem_euclid(total_duration);
    let mut active_index = clips.len() - 1;
    for (index, duration) in segment_durations.iter().enumerate() {
        if cursor < *duration {
            active_index = index;
            break;
        }
        cursor -= *duration;
    }

    let active = &clips[active_index];
    let next = &clips[(active_index + 1) % clips.len()];
    let fade = outgoing_fades[active_index];
    let incoming_fade = outgoing_fades[(active_index + clips.len() - 1) % clips.len()];
    let sample_time = incoming_fade + cursor;
    if fade <= f32::EPSILON || sample_time < durations[active_index] - fade {
        active.apply(sample_time, transforms);
        return;
    }

    let base = transforms.to_vec();
    let mut active_pose = base.clone();
    let mut next_pose = base;
    active.apply(sample_time, &mut active_pose);
    let factor = ((sample_time - (durations[active_index] - fade)) / fade).clamp(0.0, 1.0);
    next.apply(factor * fade, &mut next_pose);
    for ((target, from), to) in transforms.iter_mut().zip(active_pose).zip(next_pose) {
        target.translation = from.translation.lerp(to.translation, factor);
        target.rotation = from.rotation.slerp(to.rotation, factor).normalize();
        target.scale = from.scale.lerp(to.scale, factor);
    }
}

fn validate_times(times: &[f32]) -> Result<(), String> {
    if times.is_empty() {
        return Err("animation input accessor must not be empty".to_string());
    }
    if times[0] < 0.0 || !times[0].is_finite() {
        return Err("animation keyframe times must be finite and non-negative".to_string());
    }
    if times
        .windows(2)
        .any(|pair| !pair[1].is_finite() || pair[1] <= pair[0])
    {
        return Err("animation keyframe times must be strictly increasing".to_string());
    }
    Ok(())
}

fn validate_vec3_values(values: &[Vec3], label: &str) -> Result<(), String> {
    if values.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(format!("animation {label} contains NaN or infinity"))
    }
}

fn validate_scales(values: &[Vec3], interpolation: Interpolation) -> Result<(), String> {
    validate_vec3_values(values, "scale")?;
    let cubic = interpolation == Interpolation::CubicSpline;
    if values.iter().enumerate().any(|(index, value)| {
        (!cubic || index % 3 == 1) && value.abs().min_element() <= f32::EPSILON
    }) {
        return Err("animation scale contains a zero component".to_string());
    }
    Ok(())
}

fn validate_rotations(values: &mut [Vec4], interpolation: Interpolation) -> Result<(), String> {
    let cubic = interpolation == Interpolation::CubicSpline;
    for (index, value) in values.iter_mut().enumerate() {
        if !value.is_finite() {
            return Err("animation rotation contains NaN or infinity".to_string());
        }
        if !cubic || index % 3 == 1 {
            if value.length_squared() <= f32::EPSILON {
                return Err("animation rotation key is a zero quaternion".to_string());
            }
            *value = value.normalize();
        }
    }
    Ok(())
}

fn retarget_translations(
    values: &mut [Vec3],
    interpolation: Interpolation,
    target: RetargetTarget,
) {
    let cubic = interpolation == Interpolation::CubicSpline;
    for (index, value) in values.iter_mut().enumerate() {
        if cubic && index % 3 != 1 {
            *value *= target.translation_scale;
        } else {
            *value = target.target_rest.translation
                + (*value - target.source_rest.translation) * target.translation_scale;
        }
    }
}

fn retarget_rotations(
    values: &mut [Vec4],
    interpolation: Interpolation,
    target: RetargetTarget,
) -> Result<(), String> {
    let offset = target.target_rest.rotation * target.source_rest.rotation.inverse();
    let cubic = interpolation == Interpolation::CubicSpline;
    for (index, value) in values.iter_mut().enumerate() {
        let transformed = offset * Quat::from_xyzw(value.x, value.y, value.z, value.w);
        *value = Vec4::from_array(transformed.to_array());
        if !cubic || index % 3 == 1 {
            if value.length_squared() <= f32::EPSILON || !value.is_finite() {
                return Err("retargeted animation rotation is invalid".to_string());
            }
            *value = value.normalize();
        }
    }
    Ok(())
}

fn sample_vec3(times: &[f32], values: &[Vec3], interpolation: Interpolation, time: f32) -> Vec3 {
    let (left, right, factor, span) = sample_interval(times, time);
    match interpolation {
        Interpolation::Step => values[left],
        Interpolation::Linear => values[left].lerp(values[right], factor),
        Interpolation::CubicSpline => cubic_vec3(values, left, right, factor, span),
    }
}

fn sample_rotation(
    times: &[f32],
    values: &[Vec4],
    interpolation: Interpolation,
    time: f32,
) -> Quat {
    let (left, right, factor, span) = sample_interval(times, time);
    match interpolation {
        Interpolation::Step => Quat::from_array(values[left].to_array()).normalize(),
        Interpolation::Linear => Quat::from_array(values[left].to_array())
            .slerp(Quat::from_array(values[right].to_array()), factor)
            .normalize(),
        Interpolation::CubicSpline => {
            let value = cubic_vec4(values, left, right, factor, span);
            Quat::from_array(value.normalize().to_array())
        }
    }
}

fn sample_interval(times: &[f32], time: f32) -> (usize, usize, f32, f32) {
    if times.len() == 1 || time <= times[0] {
        return (0, 0, 0.0, 0.0);
    }
    let upper = times.partition_point(|key| *key <= time);
    if upper >= times.len() {
        let last = times.len() - 1;
        return (last, last, 0.0, 0.0);
    }
    let left = upper - 1;
    let span = times[upper] - times[left];
    (left, upper, (time - times[left]) / span, span)
}

fn cubic_vec3(values: &[Vec3], left: usize, right: usize, t: f32, span: f32) -> Vec3 {
    let p0 = values[left * 3 + 1];
    let m0 = values[left * 3 + 2] * span;
    let p1 = values[right * 3 + 1];
    let m1 = values[right * 3] * span;
    hermite_vec3(p0, m0, p1, m1, t)
}

fn cubic_vec4(values: &[Vec4], left: usize, right: usize, t: f32, span: f32) -> Vec4 {
    let p0 = values[left * 3 + 1];
    let m0 = values[left * 3 + 2] * span;
    let p1 = values[right * 3 + 1];
    let m1 = values[right * 3] * span;
    let t2 = t * t;
    let t3 = t2 * t;
    p0 * (2.0 * t3 - 3.0 * t2 + 1.0)
        + m0 * (t3 - 2.0 * t2 + t)
        + p1 * (-2.0 * t3 + 3.0 * t2)
        + m1 * (t3 - t2)
}

fn hermite_vec3(p0: Vec3, m0: Vec3, p1: Vec3, m1: Vec3, t: f32) -> Vec3 {
    let t2 = t * t;
    let t3 = t2 * t;
    p0 * (2.0 * t3 - 3.0 * t2 + 1.0)
        + m0 * (t3 - 2.0 * t2 + t)
        + p1 * (-2.0 * t3 + 3.0 * t2)
        + m1 * (t3 - t2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_translation_interpolates_between_keys() {
        let value = sample_vec3(
            &[0.0, 2.0],
            &[Vec3::ZERO, Vec3::new(2.0, 4.0, 6.0)],
            Interpolation::Linear,
            0.5,
        );

        assert_eq!(value, Vec3::new(0.5, 1.0, 1.5));
    }

    #[test]
    fn step_translation_holds_the_previous_key() {
        let value = sample_vec3(
            &[0.0, 1.0],
            &[Vec3::ZERO, Vec3::ONE],
            Interpolation::Step,
            0.75,
        );

        assert_eq!(value, Vec3::ZERO);
    }

    #[test]
    fn fk_rotation_retarget_maps_source_rest_to_target_rest() {
        let source = NodeTransform {
            translation: Vec3::ZERO,
            rotation: Quat::from_rotation_x(0.3),
            scale: Vec3::ONE,
        };
        let target = NodeTransform {
            translation: Vec3::ZERO,
            rotation: Quat::from_rotation_y(-0.4),
            scale: Vec3::ONE,
        };
        let mut values = vec![Vec4::from_array(source.rotation.to_array())];

        retarget_rotations(
            &mut values,
            Interpolation::Linear,
            RetargetTarget {
                target_node: 0,
                source_rest: source,
                target_rest: target,
                translation_scale: 1.0,
                hips: false,
            },
        )
        .expect("retarget");

        let actual = Quat::from_array(values[0].to_array());
        assert!(actual.abs_diff_eq(target.rotation, 1e-5));
    }

    #[test]
    fn mixer_crossfades_between_two_clips() {
        let clip = |from: f32, to: f32| CpuAnimationClip {
            channels: vec![AnimationChannel {
                target_node: 0,
                interpolation: Interpolation::Linear,
                times: vec![0.0, 1.0],
                values: AnimationValues::Translation(vec![
                    Vec3::new(from, 0.0, 0.0),
                    Vec3::new(to, 0.0, 0.0),
                ]),
            }],
            duration: 1.0,
            source_label: "test",
        };
        let clips = [clip(0.0, 1.0), clip(10.0, 11.0)];
        let mut pose = [NodeTransform {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }];

        apply_mixed(&clips, 0.625, 0.25, &mut pose);

        assert!(pose[0].translation.x > 5.0);
        assert!(pose[0].translation.x < 6.0);

        let mut before_boundary = [NodeTransform {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }];
        let mut after_boundary = before_boundary;
        apply_mixed(&clips, 0.75 - 1e-5, 0.25, &mut before_boundary);
        apply_mixed(&clips, 0.75, 0.25, &mut after_boundary);
        assert!((before_boundary[0].translation.x - after_boundary[0].translation.x).abs() < 1e-3);
    }

    #[test]
    fn animation_collection_rejects_more_than_the_runtime_clip_limit() {
        let clips = (0..=MAX_ANIMATION_CLIPS)
            .map(|_| CpuAnimationClip {
                channels: Vec::new(),
                duration: 0.0,
                source_label: "test",
            })
            .collect::<Vec<_>>();

        let error = validate_clip_collection(&clips).expect_err("clip limit must be enforced");

        assert!(error.contains("more than 64 clips"));
    }
}
