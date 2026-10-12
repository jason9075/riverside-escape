//! Cosmetic animation only: the authoritative bike, aim and hit positions stay in common/server.
use crate::game::{FollowCamera, GameSession, GunnerVisual};
use bevy::{gltf::GltfAssetLabel, prelude::*, world_serialization::WorldAssetRoot};
use riverside_common::{GamePhase, SeatKind, WeaponKind};

#[derive(Resource, Default)]
pub struct MotionClock {
    wheel: f32,
    crank: f32,
    rotor: f32,
}

#[derive(Component)]
pub struct WeaponVisual {
    kind: WeaponKind,
    first_person: bool,
}
#[derive(Component)]
pub struct MuzzleFlash;
#[derive(Component)]
pub struct MovingPart {
    rest: Transform,
    kind: Part,
}
#[derive(Clone, Copy)]
enum Part {
    Wheel,
    Crank,
    Pedal,
    MainRotor,
    TailRotor,
    Hip(f32),
    Knee(f32),
    Ankle(f32),
    Pump,
}

pub fn spawn_weapons(
    mut commands: Commands,
    assets: Res<AssetServer>,
    cameras: Query<Entity, With<FollowCamera>>,
    gunners: Query<Entity, With<GunnerVisual>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let (Ok(camera), Ok(gunner)) = (cameras.single(), gunners.single()) else {
        return;
    };
    let skin = materials.add(Color::srgb(0.65, 0.36, 0.22));
    let glove = materials.add(Color::srgb(0.025, 0.05, 0.06));
    let arm = meshes.add(Cylinder::new(0.038, 1.0).mesh().resolution(10));
    let hand = meshes.add(Sphere::new(1.0).mesh().uv(10, 6));
    let flash_mesh = meshes.add(Sphere::new(0.07).mesh().ico(1).unwrap());
    let flash_material = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.75, 0.22),
        unlit: true,
        ..default()
    });
    for kind in [WeaponKind::Launcher, WeaponKind::Shotgun] {
        let path = match kind {
            WeaponKind::Launcher => "models/launcher.glb",
            WeaponKind::Shotgun => "models/shotgun.glb",
        };
        for (parent, first_person) in [(camera, true), (gunner, false)] {
            let weapon = commands
                .spawn((
                    WeaponVisual { kind, first_person },
                    ChildOf(parent),
                    WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(path))),
                    Transform::default(),
                    Visibility::Hidden,
                ))
                .id();
            if first_person {
                for (wrist, elbow) in [
                    (Vec3::new(0.0, -0.13, 0.12), Vec3::new(0.24, -0.34, 0.31)),
                    (
                        Vec3::new(-0.025, -0.08, -0.25),
                        Vec3::new(-0.29, -0.30, 0.12),
                    ),
                ] {
                    let delta = wrist - elbow;
                    commands.spawn((
                        ChildOf(weapon),
                        Mesh3d(arm.clone()),
                        MeshMaterial3d(skin.clone()),
                        Transform::from_translation((wrist + elbow) * 0.5)
                            .with_rotation(Quat::from_rotation_arc(Vec3::Y, delta.normalize()))
                            .with_scale(Vec3::new(1.0, delta.length(), 1.0)),
                    ));
                    commands.spawn((
                        ChildOf(weapon),
                        Mesh3d(hand.clone()),
                        MeshMaterial3d(glove.clone()),
                        Transform::from_translation(wrist)
                            .with_scale(Vec3::new(0.047, 0.052, 0.055)),
                    ));
                }
            }
            commands.spawn((
                MuzzleFlash,
                ChildOf(weapon),
                Mesh3d(flash_mesh.clone()),
                MeshMaterial3d(flash_material.clone()),
                Transform::from_xyz(
                    0.0,
                    if kind == WeaponKind::Shotgun {
                        0.026
                    } else {
                        0.0
                    },
                    if kind == WeaponKind::Shotgun {
                        -0.66
                    } else {
                        -0.41
                    },
                ),
                Visibility::Hidden,
            ));
        }
    }
}

#[allow(clippy::type_complexity)]
pub fn bind_parts(
    mut commands: Commands,
    nodes: Query<(Entity, &Name, &Transform), (Added<Name>, Without<MovingPart>)>,
) {
    for (entity, name, transform) in &nodes {
        let name = name.as_str();
        let part = match name {
            "bicycle.WheelFront" | "bicycle.WheelRear" => Part::Wheel,
            "bicycle.Crank" => Part::Crank,
            "bicycle.Pedal_L" | "bicycle.Pedal_R" => Part::Pedal,
            "helicopter.MainRotor" => Part::MainRotor,
            "helicopter.TailRotor" => Part::TailRotor,
            "rider.Hip_-1" => Part::Hip(-1.0),
            "rider.Hip_1" => Part::Hip(1.0),
            "rider.Knee_-1" => Part::Knee(-1.0),
            "rider.Knee_1" => Part::Knee(1.0),
            "rider.Ankle_-1" => Part::Ankle(-1.0),
            "rider.Ankle_1" => Part::Ankle(1.0),
            "shotgun.Pump" => Part::Pump,
            _ => continue,
        };
        commands.entity(entity).insert(MovingPart {
            rest: *transform,
            kind: part,
        });
    }
}

/// Two rigid bones, bent toward the handlebars. Returns absolute hip rotation
/// and knee/ankle rotations relative to their parents. Foot stays level.
fn leg_pose(side: f32, crank: f32) -> (Quat, Quat, Quat) {
    let hip = Vec3::new(side * 0.13, 1.08, 0.205);
    let pedal = Vec3::new(
        side * 0.2145,
        0.35 - side * 0.15 * crank.cos(),
        0.10 + side * 0.15 * crank.sin(),
    );
    let target = pedal + Vec3::new(0.0, 0.07, 0.0);
    let delta = target - hip;
    let length = delta.length().clamp(0.02, 0.849);
    let axis = delta.normalize();
    let bend = (Vec3::NEG_Z - axis * Vec3::NEG_Z.dot(axis)).normalize();
    let along = (0.42_f32.powi(2) - 0.43_f32.powi(2) + length * length) / (2.0 * length);
    let knee = axis * along + bend * (0.42_f32.powi(2) - along * along).max(0.0).sqrt();
    let upper = Quat::from_rotation_arc(Vec3::NEG_Y, knee.normalize());
    let lower = Quat::from_rotation_arc(Vec3::NEG_Y, (delta - knee).normalize());
    (upper, upper.inverse() * lower, lower.inverse())
}

pub fn animate_parts(
    time: Res<Time>,
    session: Res<GameSession>,
    mut clock: ResMut<MotionClock>,
    mut nodes: Query<(&MovingPart, &mut Transform)>,
) {
    let dt = time.delta_secs().min(0.1);
    if session.phase == GamePhase::Running {
        clock.wheel =
            (clock.wheel + session.bike.speed * dt / 0.349).rem_euclid(std::f32::consts::TAU);
        let pedaling = if session.local_seat == SeatKind::Rider {
            session.rider_input.pedal_held && !session.rider_input.brake_held
        } else {
            session.rider_pedaling
        };
        if pedaling && session.bike.speed.abs() > 0.05 {
            clock.crank = (clock.crank + (session.bike.speed.abs() / 2.2).clamp(1.4, 7.0) * dt)
                .rem_euclid(std::f32::consts::TAU);
        }
        clock.rotor = (clock.rotor + dt * 28.0).rem_euclid(std::f32::consts::TAU);
    }
    for (part, mut transform) in &mut nodes {
        *transform = part.rest;
        transform.rotation = match part.kind {
            Part::Wheel => Quat::from_rotation_x(-clock.wheel) * part.rest.rotation,
            Part::Crank => Quat::from_rotation_x(-clock.crank),
            Part::Pedal => Quat::from_rotation_x(clock.crank),
            Part::MainRotor => Quat::from_rotation_y(clock.rotor) * part.rest.rotation,
            Part::TailRotor => Quat::from_rotation_x(clock.rotor * 2.0) * part.rest.rotation,
            Part::Hip(side) => leg_pose(side, clock.crank).0,
            Part::Knee(side) => leg_pose(side, clock.crank).1,
            Part::Ankle(side) => leg_pose(side, clock.crank).2,
            Part::Pump => {
                let cycle = ((0.75 - session.visual_fire_cooldown) / 0.75).clamp(0.0, 1.0);
                if session.gunner_input.weapon == WeaponKind::Shotgun
                    && session.visual_fire_cooldown > 0.0
                {
                    transform.translation.z += (cycle * std::f32::consts::PI).sin() * 0.10;
                }
                part.rest.rotation
            }
        };
    }
}

pub fn update_weapons(
    time: Res<Time>,
    mut session: ResMut<GameSession>,
    mut weapons: Query<(&WeaponVisual, &mut Transform, &mut Visibility), Without<MuzzleFlash>>,
    mut flashes: Query<&mut Visibility, (With<MuzzleFlash>, Without<WeaponVisual>)>,
    mut gizmos: Gizmos,
) {
    session.weapon_recoil = (session.weapon_recoil - time.delta_secs()).max(0.0);
    session.visual_fire_cooldown = (session.visual_fire_cooldown - time.delta_secs()).max(0.0);
    if let Some(tracer) = session.shotgun_tracer.as_mut() {
        tracer.secs_remaining -= time.delta_secs();
        if tracer.secs_remaining <= 0.0 {
            session.shotgun_tracer = None;
        }
    }
    let kick = (session.weapon_recoil / 0.22).clamp(0.0, 1.0);
    for (weapon, mut transform, mut visibility) in &mut weapons {
        let selected = weapon.kind == session.gunner_input.weapon;
        *visibility =
            if selected && (!weapon.first_person || session.local_seat == SeatKind::Gunner) {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        if weapon.first_person {
            let sight_height = if weapon.kind == WeaponKind::Shotgun {
                0.077
            } else {
                0.15
            };
            // The longer shotgun stock sits beyond the near plane in both poses.
            let (hip_depth, ads_depth) = if weapon.kind == WeaponKind::Shotgun {
                (-0.80, -0.95)
            } else {
                (-0.65, -0.70)
            };
            let hip = Vec3::new(0.24, -0.23, hip_depth);
            let ads = Vec3::new(0.0, -sight_height, ads_depth);
            transform.translation =
                hip.lerp(ads, session.gunner_ads) + Vec3::new(0.0, 0.006 * kick, 0.09 * kick);
            transform.rotation = Quat::from_rotation_x(0.065 * kick);
        } else {
            transform.translation = Vec3::new(0.255, 0.56, -0.30);
            transform.rotation = Quat::from_rotation_x(session.gunner_input.pitch);
        }
    }
    for mut flash in &mut flashes {
        *flash = if session.weapon_recoil > 0.17 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    if let Some(tracer) = session.shotgun_tracer {
        let aim_origin = riverside_common::gunner_aim_origin(session.bike);
        let aim_origin = Vec3::new(aim_origin.x, aim_origin.y, aim_origin.z);
        // Pellets leave the barrel, slightly below and ahead of the Gunner's
        // eye; lines from the eye itself would collapse to dots on screen.
        let muzzle = if session.local_seat == SeatKind::Gunner {
            let eye = crate::game::gunner_eye(session.bike, session.gunner_input);
            eye.translation + eye.rotation * Vec3::new(0.0, -0.12, -0.9)
        } else {
            aim_origin
        };
        let fade = (tracer.secs_remaining / crate::game::SHOTGUN_TRACER_SECS).clamp(0.0, 1.0);
        for ray in tracer.rays {
            gizmos.line(
                muzzle,
                aim_origin + ray,
                Color::srgba(1.0, 0.76, 0.33, 0.35 + 0.65 * fade),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn feet_follow_pedals_and_remain_level_for_entire_cycle() {
        for side in [-1.0, 1.0] {
            for step in 0..120 {
                let phase = step as f32 * std::f32::consts::TAU / 120.0;
                let (hip, knee, ankle) = leg_pose(side, phase);
                let foot = Vec3::new(side * 0.13, 1.08, 0.205)
                    + hip * (Vec3::NEG_Y * 0.42)
                    + hip * knee * (Vec3::NEG_Y * 0.43);
                let target = Vec3::new(
                    side * 0.2145,
                    0.42 - side * 0.15 * phase.cos(),
                    0.10 + side * 0.15 * phase.sin(),
                );
                assert!(
                    foot.distance(target) < 0.01,
                    "foot missed pedal: {foot:?} {target:?}"
                );
                assert!((hip * knee * ankle * Vec3::Y).distance(Vec3::Y) < 0.001);
            }
        }
    }
}
