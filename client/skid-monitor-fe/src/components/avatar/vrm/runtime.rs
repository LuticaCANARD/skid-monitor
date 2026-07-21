use super::animation::{CpuAnimationClip, NodeTransform, apply_mixed};
use super::loader::CpuNode;
use glam::{Mat4, Quat, Vec3};
use std::collections::HashMap;

const SPRING_RESET_SECONDS: f32 = 0.25;
const SPRING_MAX_STEP_SECONDS: f32 = 1.0 / 30.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ExpressionOverride {
    None,
    Block,
    Blend,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct MorphBind {
    pub(super) node: usize,
    pub(super) target: usize,
    pub(super) weight: f32,
}

#[derive(Debug)]
pub(super) struct CpuExpression {
    pub(super) is_binary: bool,
    pub(super) morph_binds: Vec<MorphBind>,
    pub(super) override_blink: ExpressionOverride,
    pub(super) override_look_at: ExpressionOverride,
    pub(super) override_mouth: ExpressionOverride,
}

#[derive(Debug, Default)]
pub(super) struct CpuExpressionSet {
    pub(super) expressions: HashMap<String, CpuExpression>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct RangeMap {
    pub(super) input_max_degrees: f32,
    pub(super) output_scale: f32,
}

impl Default for RangeMap {
    fn default() -> Self {
        Self {
            input_max_degrees: 90.0,
            output_scale: 10.0,
        }
    }
}

impl RangeMap {
    fn map(self, degrees: f32) -> f32 {
        if self.input_max_degrees <= f32::EPSILON {
            return if degrees.abs() <= f32::EPSILON {
                0.0
            } else {
                self.output_scale
            };
        }
        degrees.abs().min(self.input_max_degrees) / self.input_max_degrees * self.output_scale
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LookAtKind {
    Bone,
    Expression,
}

#[derive(Debug)]
pub(super) struct CpuLookAt {
    pub(super) kind: LookAtKind,
    pub(super) left_eye: Option<usize>,
    pub(super) right_eye: Option<usize>,
    pub(super) horizontal_inner: RangeMap,
    pub(super) horizontal_outer: RangeMap,
    pub(super) vertical_down: RangeMap,
    pub(super) vertical_up: RangeMap,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum ConstraintKind {
    Rotation,
    Roll(Vec3),
    Aim(Vec3),
}

#[derive(Clone, Copy, Debug)]
pub(super) struct CpuNodeConstraint {
    pub(super) destination: usize,
    pub(super) source: usize,
    pub(super) weight: f32,
    pub(super) kind: ConstraintKind,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum ColliderShape {
    Sphere {
        offset: Vec3,
        radius: f32,
    },
    Capsule {
        offset: Vec3,
        tail: Vec3,
        radius: f32,
    },
}

#[derive(Clone, Copy, Debug)]
pub(super) struct CpuCollider {
    pub(super) node: usize,
    pub(super) shape: ColliderShape,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct CpuSpringJoint {
    pub(super) node: usize,
    pub(super) hit_radius: f32,
    pub(super) stiffness: f32,
    pub(super) gravity_power: f32,
    pub(super) gravity_direction: Vec3,
    pub(super) drag_force: f32,
}

#[derive(Debug)]
pub(super) struct CpuSpring {
    pub(super) joints: Vec<CpuSpringJoint>,
    pub(super) collider_indices: Vec<usize>,
    pub(super) center: Option<usize>,
}

#[derive(Clone, Copy, Debug)]
struct SpringJointState {
    previous_tail: Vec3,
    current_tail: Vec3,
}

#[derive(Debug, Default)]
pub(super) struct VrmRuntimeState {
    spring_joints: Vec<Vec<SpringJointState>>,
    last_time: Option<f32>,
}

pub(super) struct FrameInput<'a> {
    pub(super) time: f32,
    pub(super) crossfade_seconds: f32,
    pub(super) expression: &'a str,
    pub(super) look_yaw_degrees: f32,
    pub(super) look_pitch_degrees: f32,
    pub(super) spring_bone_enabled: bool,
    pub(super) look_at_enabled: bool,
}

pub(super) struct FramePose {
    pub(super) worlds: Vec<Mat4>,
    pub(super) morph_weights: HashMap<(usize, usize), f32>,
}

#[derive(Clone, Copy)]
pub(super) struct RuntimeRig<'a> {
    pub(super) nodes: &'a [CpuNode],
    pub(super) traversal: &'a [usize],
    pub(super) animations: &'a [CpuAnimationClip],
    pub(super) expressions: &'a CpuExpressionSet,
    pub(super) look_at: Option<&'a CpuLookAt>,
    pub(super) constraints: &'a [CpuNodeConstraint],
    pub(super) springs: &'a [CpuSpring],
    pub(super) colliders: &'a [CpuCollider],
}

pub(super) fn evaluate_frame(
    rig: RuntimeRig<'_>,
    input: FrameInput<'_>,
    state: &mut VrmRuntimeState,
) -> FramePose {
    let mut transforms = rig.nodes.iter().map(|node| node.rest).collect::<Vec<_>>();
    apply_mixed(
        rig.animations,
        input.time,
        input.crossfade_seconds,
        &mut transforms,
    );
    let mut expression_values = procedural_expression_values(
        rig.expressions,
        input.expression,
        input.time,
        rig.look_at.filter(|_| input.look_at_enabled),
        input.look_yaw_degrees,
        input.look_pitch_degrees,
    );
    if input.look_at_enabled
        && let Some(look_at) = rig.look_at
    {
        apply_bone_look_at(
            rig.nodes,
            look_at,
            input.look_yaw_degrees,
            input.look_pitch_degrees,
            &mut transforms,
        );
    }
    let mut worlds = compute_worlds(rig.nodes, rig.traversal, &transforms);
    apply_constraints(
        rig.nodes,
        rig.traversal,
        rig.constraints,
        &mut transforms,
        &mut worlds,
    );

    update_spring_bones(
        rig.nodes,
        rig.traversal,
        rig.springs,
        rig.colliders,
        input.time,
        input.spring_bone_enabled,
        &mut transforms,
        &mut worlds,
        state,
    );
    let morph_weights = accumulate_morph_weights(rig.expressions, &mut expression_values);
    FramePose {
        worlds,
        morph_weights,
    }
}

fn compute_worlds(
    nodes: &[CpuNode],
    traversal: &[usize],
    transforms: &[NodeTransform],
) -> Vec<Mat4> {
    let mut worlds = vec![Mat4::IDENTITY; nodes.len()];
    for node_index in traversal {
        let parent = nodes[*node_index]
            .parent
            .map_or(Mat4::IDENTITY, |parent| worlds[parent]);
        worlds[*node_index] = parent * transforms[*node_index].matrix();
    }
    worlds
}

fn apply_constraints(
    nodes: &[CpuNode],
    traversal: &[usize],
    constraints: &[CpuNodeConstraint],
    transforms: &mut [NodeTransform],
    worlds: &mut Vec<Mat4>,
) {
    for constraint in constraints {
        let source = constraint.source;
        let destination = constraint.destination;
        let source_rest = nodes[source].rest.rotation;
        let destination_rest = nodes[destination].rest.rotation;
        let source_rotation = transforms[source].rotation;
        let target = match constraint.kind {
            ConstraintKind::Rotation => {
                destination_rest * (source_rest.inverse() * source_rotation)
            }
            ConstraintKind::Roll(axis) => {
                let source_delta = source_rest.inverse() * source_rotation;
                let delta_in_parent = source_rest * source_delta * source_rest.inverse();
                let delta_in_destination =
                    destination_rest.inverse() * delta_in_parent * destination_rest;
                let rotated_axis = delta_in_destination * axis;
                let swing = rotation_arc(axis, rotated_axis);
                destination_rest * swing.inverse() * delta_in_destination
            }
            ConstraintKind::Aim(axis) => {
                let destination_position = worlds[destination].transform_point3(Vec3::ZERO);
                let source_position = worlds[source].transform_point3(Vec3::ZERO);
                let direction = source_position - destination_position;
                if direction.length_squared() <= f32::EPSILON {
                    destination_rest
                } else {
                    let parent_rotation = nodes[destination]
                        .parent
                        .map_or(Quat::IDENTITY, |parent| world_rotation(worlds[parent]));
                    let from = parent_rotation * destination_rest * axis;
                    let arc = rotation_arc(from, direction.normalize());
                    parent_rotation.inverse() * arc * parent_rotation * destination_rest
                }
            }
        };
        transforms[destination].rotation = destination_rest
            .slerp(target.normalize(), constraint.weight)
            .normalize();
        *worlds = compute_worlds(nodes, traversal, transforms);
    }
}

fn procedural_expression_values(
    expressions: &CpuExpressionSet,
    selected: &str,
    time: f32,
    look_at: Option<&CpuLookAt>,
    yaw: f32,
    pitch: f32,
) -> HashMap<String, f32> {
    let mut values = HashMap::new();
    if expressions.expressions.contains_key(selected) && !selected.is_empty() {
        values.insert(selected.to_string(), 1.0);
    }
    if expressions.expressions.contains_key("blink") {
        let phase = time.rem_euclid(4.2);
        if phase < 0.18 {
            values.insert(
                "blink".to_string(),
                (phase / 0.18 * std::f32::consts::PI).sin(),
            );
        }
    }
    if let Some(look_at) = look_at.filter(|look_at| look_at.kind == LookAtKind::Expression) {
        if yaw >= 0.0 {
            values.insert(
                "lookLeft".to_string(),
                look_at.horizontal_outer.map(yaw).clamp(0.0, 1.0),
            );
        } else {
            values.insert(
                "lookRight".to_string(),
                look_at.horizontal_outer.map(yaw).clamp(0.0, 1.0),
            );
        }
        if pitch >= 0.0 {
            values.insert(
                "lookDown".to_string(),
                look_at.vertical_down.map(pitch).clamp(0.0, 1.0),
            );
        } else {
            values.insert(
                "lookUp".to_string(),
                look_at.vertical_up.map(pitch).clamp(0.0, 1.0),
            );
        }
    }

    let selected_weight = values.get(selected).copied().unwrap_or(0.0);
    if let Some(expression) = expressions.expressions.get(selected) {
        override_procedural(
            &mut values,
            &["blink", "blinkLeft", "blinkRight"],
            expression.override_blink,
            selected_weight,
        );
        override_procedural(
            &mut values,
            &["lookUp", "lookDown", "lookLeft", "lookRight"],
            expression.override_look_at,
            selected_weight,
        );
        override_procedural(
            &mut values,
            &["aa", "ih", "ou", "ee", "oh"],
            expression.override_mouth,
            selected_weight,
        );
    }
    values
}

fn override_procedural(
    values: &mut HashMap<String, f32>,
    names: &[&str],
    mode: ExpressionOverride,
    expression_weight: f32,
) {
    let factor = match mode {
        ExpressionOverride::None => return,
        ExpressionOverride::Block => 0.0,
        ExpressionOverride::Blend => 1.0 - expression_weight.clamp(0.0, 1.0),
    };
    for name in names {
        if let Some(value) = values.get_mut(*name) {
            *value *= factor;
        }
    }
}

fn accumulate_morph_weights(
    expressions: &CpuExpressionSet,
    values: &mut HashMap<String, f32>,
) -> HashMap<(usize, usize), f32> {
    let mut result = HashMap::new();
    for (name, input) in values {
        let Some(expression) = expressions.expressions.get(name) else {
            continue;
        };
        let value = if expression.is_binary {
            if *input >= 0.5 { 1.0 } else { 0.0 }
        } else {
            input.clamp(0.0, 1.0)
        };
        for binding in &expression.morph_binds {
            *result.entry((binding.node, binding.target)).or_insert(0.0) += value * binding.weight;
        }
    }
    for weight in result.values_mut() {
        *weight = weight.clamp(0.0, 1.0);
    }
    result
}

fn apply_bone_look_at(
    nodes: &[CpuNode],
    look_at: &CpuLookAt,
    yaw: f32,
    pitch: f32,
    transforms: &mut [NodeTransform],
) {
    if look_at.kind != LookAtKind::Bone {
        return;
    }
    for (eye, left) in [(look_at.left_eye, true), (look_at.right_eye, false)] {
        let Some(eye) = eye else { continue };
        let yaw_output = if yaw >= 0.0 {
            if left {
                look_at.horizontal_outer.map(yaw)
            } else {
                look_at.horizontal_inner.map(yaw)
            }
        } else if left {
            -look_at.horizontal_inner.map(yaw)
        } else {
            -look_at.horizontal_outer.map(yaw)
        };
        let pitch_output = if pitch >= 0.0 {
            look_at.vertical_down.map(pitch)
        } else {
            -look_at.vertical_up.map(pitch)
        };
        let delta = Quat::from_rotation_y(yaw_output.to_radians())
            * Quat::from_rotation_x(pitch_output.to_radians());
        transforms[eye].rotation = (nodes[eye].rest.rotation * delta).normalize();
    }
}

#[allow(clippy::too_many_arguments)]
fn update_spring_bones(
    nodes: &[CpuNode],
    traversal: &[usize],
    springs: &[CpuSpring],
    colliders: &[CpuCollider],
    time: f32,
    enabled: bool,
    transforms: &mut [NodeTransform],
    worlds: &mut Vec<Mat4>,
    state: &mut VrmRuntimeState,
) {
    let delta = state.last_time.map(|last| time - last);
    state.last_time = Some(time);
    let reset = !enabled
        || delta.is_none_or(|delta| delta <= 0.0 || delta > SPRING_RESET_SECONDS)
        || state.spring_joints.len() != springs.len()
        || state
            .spring_joints
            .iter()
            .zip(springs)
            .any(|(states, spring)| states.len() != spring.joints.len().saturating_sub(1));
    if reset {
        state.spring_joints = springs
            .iter()
            .map(|spring| {
                let center_inverse = spring
                    .center
                    .map_or(Mat4::IDENTITY, |center| worlds[center].inverse());
                spring
                    .joints
                    .windows(2)
                    .map(|pair| {
                        let tail = center_inverse
                            .transform_point3(worlds[pair[1].node].transform_point3(Vec3::ZERO));
                        SpringJointState {
                            previous_tail: tail,
                            current_tail: tail,
                        }
                    })
                    .collect()
            })
            .collect();
        return;
    }
    let delta = delta.unwrap_or_default().min(SPRING_MAX_STEP_SECONDS);
    let center_matrices = springs
        .iter()
        .map(|spring| {
            spring
                .center
                .map_or(Mat4::IDENTITY, |center| worlds[center])
        })
        .collect::<Vec<_>>();
    for (spring_index, spring) in springs.iter().enumerate() {
        for (joint_index, pair) in spring.joints.windows(2).enumerate() {
            let joint = pair[0];
            let tail_node = pair[1].node;
            let joint_position = worlds[joint.node].transform_point3(Vec3::ZERO);
            let animated_tail = worlds[tail_node].transform_point3(Vec3::ZERO);
            let bone = animated_tail - joint_position;
            let length = bone.length();
            if length <= f32::EPSILON {
                continue;
            }
            let spring_state = &mut state.spring_joints[spring_index][joint_index];
            let center_matrix = center_matrices[spring_index];
            let center_inverse = center_matrix.inverse();
            let current_tail = center_matrix.transform_point3(spring_state.current_tail);
            let previous_tail = center_matrix.transform_point3(spring_state.previous_tail);
            let inertia = (current_tail - previous_tail) * (1.0 - joint.drag_force);
            let stiffness = bone.normalize() * joint.stiffness * delta;
            let gravity = joint.gravity_direction * joint.gravity_power * delta;
            let mut next_tail = current_tail + inertia + stiffness + gravity;
            next_tail = constrain_length(joint_position, next_tail, length, bone.normalize());
            let joint_radius = joint.hit_radius * max_axis_scale(worlds[joint.node]);
            for collider_index in &spring.collider_indices {
                let Some(collider) = colliders.get(*collider_index) else {
                    continue;
                };
                next_tail = collide(
                    next_tail,
                    joint_position,
                    length,
                    joint_radius,
                    *collider,
                    worlds,
                );
            }
            spring_state.previous_tail = spring_state.current_tail;
            spring_state.current_tail = center_inverse.transform_point3(next_tail);

            let desired = (next_tail - joint_position).normalize_or_zero();
            let current = bone.normalize();
            let world_delta = rotation_arc(current, desired);
            let parent_rotation = nodes[joint.node]
                .parent
                .map_or(Quat::IDENTITY, |parent| world_rotation(worlds[parent]));
            let current_world = world_rotation(worlds[joint.node]);
            transforms[joint.node].rotation =
                (parent_rotation.inverse() * world_delta * current_world).normalize();
            *worlds = compute_worlds(nodes, traversal, transforms);
        }
    }
}

fn collide(
    tail: Vec3,
    joint_position: Vec3,
    bone_length: f32,
    joint_radius: f32,
    collider: CpuCollider,
    worlds: &[Mat4],
) -> Vec3 {
    let matrix = worlds[collider.node];
    let scale = max_axis_scale(matrix);
    let (start, end, radius) = match collider.shape {
        ColliderShape::Sphere { offset, radius } => {
            let center = matrix.transform_point3(offset);
            (center, center, radius * scale)
        }
        ColliderShape::Capsule {
            offset,
            tail: capsule_tail,
            radius,
        } => {
            let start = matrix.transform_point3(offset);
            let end = matrix.transform_point3(capsule_tail);
            (start, end, radius * scale)
        }
    };
    let minimum = radius + joint_radius;
    let mut result = tail;
    for _ in 0..8 {
        let closest = closest_point_on_segment(result, start, end);
        let delta = result - closest;
        if delta.length_squared() >= minimum * minimum - 1e-8 {
            break;
        }
        let direction = if delta.length_squared() > f32::EPSILON {
            delta.normalize()
        } else {
            perpendicular_direction(joint_position - closest)
        };
        result = constrain_length(
            joint_position,
            closest + direction * minimum,
            bone_length,
            direction,
        );
    }
    result
}

fn max_axis_scale(matrix: Mat4) -> f32 {
    matrix
        .x_axis
        .truncate()
        .length()
        .max(matrix.y_axis.truncate().length())
        .max(matrix.z_axis.truncate().length())
}

fn perpendicular_direction(axis: Vec3) -> Vec3 {
    if axis.length_squared() <= f32::EPSILON {
        return Vec3::X;
    }
    let axis = axis.normalize();
    let basis = if axis.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    axis.cross(basis).normalize_or_zero()
}

fn constrain_length(origin: Vec3, point: Vec3, length: f32, fallback: Vec3) -> Vec3 {
    let delta = point - origin;
    origin
        + if delta.length_squared() > f32::EPSILON {
            delta.normalize() * length
        } else {
            fallback * length
        }
}

fn closest_point_on_segment(point: Vec3, start: Vec3, end: Vec3) -> Vec3 {
    let segment = end - start;
    if segment.length_squared() <= f32::EPSILON {
        return start;
    }
    start + segment * ((point - start).dot(segment) / segment.length_squared()).clamp(0.0, 1.0)
}

fn world_rotation(matrix: Mat4) -> Quat {
    let (_, rotation, _) = matrix.to_scale_rotation_translation();
    rotation.normalize()
}

fn rotation_arc(from: Vec3, to: Vec3) -> Quat {
    if from.length_squared() <= f32::EPSILON || to.length_squared() <= f32::EPSILON {
        Quat::IDENTITY
    } else {
        Quat::from_rotation_arc(from.normalize(), to.normalize()).normalize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(translation: Vec3, parent: Option<usize>) -> CpuNode {
        CpuNode {
            rest: NodeTransform {
                translation,
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
            parent,
            active: true,
        }
    }

    #[test]
    fn sphere_collision_pushes_a_tail_outside_both_radii() {
        let worlds = [Mat4::IDENTITY];
        let result = collide(
            Vec3::new(0.05, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            1.0,
            0.1,
            CpuCollider {
                node: 0,
                shape: ColliderShape::Sphere {
                    offset: Vec3::ZERO,
                    radius: 0.2,
                },
            },
            &worlds,
        );

        assert!(result.length() >= 0.3 - 1e-5);
    }

    #[test]
    fn expression_override_blends_procedural_weight() {
        let mut values = HashMap::from([("blink".to_string(), 1.0)]);
        override_procedural(&mut values, &["blink"], ExpressionOverride::Blend, 0.25);
        assert!((values["blink"] - 0.75).abs() < 1e-6);
    }

    #[test]
    fn zero_input_look_at_range_uses_the_spec_step_behavior() {
        let map = RangeMap {
            input_max_degrees: 0.0,
            output_scale: 1.0,
        };

        assert_eq!(map.map(0.0), 0.0);
        assert_eq!(map.map(0.01), 1.0);
    }

    #[test]
    fn bone_look_at_uses_inner_and_outer_eye_ranges() {
        let nodes = vec![node(Vec3::ZERO, None), node(Vec3::ZERO, None)];
        let mut transforms = nodes.iter().map(|node| node.rest).collect::<Vec<_>>();
        let look_at = CpuLookAt {
            kind: LookAtKind::Bone,
            left_eye: Some(0),
            right_eye: Some(1),
            horizontal_inner: RangeMap {
                input_max_degrees: 90.0,
                output_scale: 20.0,
            },
            horizontal_outer: RangeMap {
                input_max_degrees: 90.0,
                output_scale: 30.0,
            },
            vertical_down: RangeMap::default(),
            vertical_up: RangeMap::default(),
        };

        apply_bone_look_at(&nodes, &look_at, 45.0, 0.0, &mut transforms);
        let left = transforms[0].rotation * Vec3::Z;
        let right = transforms[1].rotation * Vec3::Z;

        assert!((left.x - 15_f32.to_radians().sin()).abs() < 1e-5);
        assert!((right.x - 10_f32.to_radians().sin()).abs() < 1e-5);
    }

    #[test]
    fn rotation_and_aim_constraints_apply_their_weights() {
        let nodes = vec![
            node(Vec3::new(1.0, 0.0, 0.0), None),
            node(Vec3::ZERO, None),
            node(Vec3::ZERO, None),
        ];
        let traversal = [0, 1, 2];
        let mut transforms = nodes.iter().map(|node| node.rest).collect::<Vec<_>>();
        transforms[0].rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let mut worlds = compute_worlds(&nodes, &traversal, &transforms);
        apply_constraints(
            &nodes,
            &traversal,
            &[
                CpuNodeConstraint {
                    destination: 1,
                    source: 0,
                    weight: 0.5,
                    kind: ConstraintKind::Rotation,
                },
                CpuNodeConstraint {
                    destination: 2,
                    source: 0,
                    weight: 1.0,
                    kind: ConstraintKind::Aim(Vec3::Z),
                },
            ],
            &mut transforms,
            &mut worlds,
        );

        let rotated = transforms[1].rotation * Vec3::X;
        let aimed = transforms[2].rotation * Vec3::Z;
        assert!((rotated.x - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
        assert!((rotated.y - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
        assert!((aimed - Vec3::X).length() < 1e-5);
    }

    #[test]
    fn spring_bone_verlet_update_rotates_the_chain_toward_gravity() {
        let nodes = vec![node(Vec3::ZERO, None), node(Vec3::Y, Some(0))];
        let traversal = [0, 1];
        let expressions = CpuExpressionSet::default();
        let springs = [CpuSpring {
            joints: vec![
                CpuSpringJoint {
                    node: 0,
                    hit_radius: 0.0,
                    stiffness: 0.0,
                    gravity_power: 10.0,
                    gravity_direction: Vec3::X,
                    drag_force: 0.5,
                },
                CpuSpringJoint {
                    node: 1,
                    hit_radius: 0.0,
                    stiffness: 0.0,
                    gravity_power: 0.0,
                    gravity_direction: Vec3::NEG_Y,
                    drag_force: 0.5,
                },
            ],
            collider_indices: Vec::new(),
            center: None,
        }];
        let rig = RuntimeRig {
            nodes: &nodes,
            traversal: &traversal,
            animations: &[],
            expressions: &expressions,
            look_at: None,
            constraints: &[],
            springs: &springs,
            colliders: &[],
        };
        let mut state = VrmRuntimeState::default();
        let input = |time| FrameInput {
            time,
            crossfade_seconds: 0.0,
            expression: "",
            look_yaw_degrees: 0.0,
            look_pitch_degrees: 0.0,
            spring_bone_enabled: true,
            look_at_enabled: false,
        };

        let _ = evaluate_frame(rig, input(0.0), &mut state);
        let frame = evaluate_frame(rig, input(1.0 / 60.0), &mut state);
        let tail = frame.worlds[1].transform_point3(Vec3::ZERO);

        assert!(tail.x > 0.1);
        assert!((tail.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn spring_center_space_follows_center_translation_without_false_inertia() {
        let nodes = vec![node(Vec3::ZERO, None), node(Vec3::Y, Some(0))];
        let traversal = [0, 1];
        let springs = [CpuSpring {
            joints: vec![
                CpuSpringJoint {
                    node: 0,
                    hit_radius: 0.0,
                    stiffness: 0.0,
                    gravity_power: 0.0,
                    gravity_direction: Vec3::NEG_Y,
                    drag_force: 0.0,
                },
                CpuSpringJoint {
                    node: 1,
                    hit_radius: 0.0,
                    stiffness: 0.0,
                    gravity_power: 0.0,
                    gravity_direction: Vec3::NEG_Y,
                    drag_force: 0.0,
                },
            ],
            collider_indices: Vec::new(),
            center: Some(0),
        }];
        let mut transforms = nodes.iter().map(|node| node.rest).collect::<Vec<_>>();
        let mut worlds = compute_worlds(&nodes, &traversal, &transforms);
        let mut state = VrmRuntimeState::default();
        update_spring_bones(
            &nodes,
            &traversal,
            &springs,
            &[],
            0.0,
            true,
            &mut transforms,
            &mut worlds,
            &mut state,
        );
        transforms[0].translation = Vec3::new(10.0, 0.0, 0.0);
        worlds = compute_worlds(&nodes, &traversal, &transforms);
        update_spring_bones(
            &nodes,
            &traversal,
            &springs,
            &[],
            1.0 / 60.0,
            true,
            &mut transforms,
            &mut worlds,
            &mut state,
        );
        let tail = worlds[1].transform_point3(Vec3::ZERO);

        assert!((tail - Vec3::new(10.0, 1.0, 0.0)).length() < 1e-5);
    }
}
