//! Shared prototype weapon rules. Hits and ammunition are resolved by the server.
use crate::{RiversideCourse, WorldPosition, first_cover_intersection};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WeaponKind {
    #[default]
    Launcher,
    Shotgun,
}

pub const SHOTGUN_STARTING_AMMO: u8 = 24;
pub const SHOTGUN_PELLETS: usize = 8;
pub const SHOTGUN_RANGE_M: f32 = 40.0;
pub const SHOTGUN_PELLET_DAMAGE: u16 = 4;
pub const SHOTGUN_COOLDOWN_TICKS: u64 = 45;
pub const SHOTGUN_HIP_SPREAD: f32 = 0.10;
pub const SHOTGUN_AIM_SPREAD: f32 = 0.04;

pub fn default_shotgun_ammo() -> u8 {
    SHOTGUN_STARTING_AMMO
}

/// Fixed symmetric spread is reproducible; clients never report their own hits.
/// Each pellet stops at the first cover or target, including a miss into scenery.
pub fn trace_shotgun(
    course: &RiversideCourse,
    origin: WorldPosition,
    direction: WorldPosition,
    aimed: bool,
    target: Option<WorldPosition>,
) -> (u8, [WorldPosition; SHOTGUN_PELLETS]) {
    let spread = if aimed {
        SHOTGUN_AIM_SPREAD
    } else {
        SHOTGUN_HIP_SPREAD
    };
    let horizontal = direction.x.hypot(direction.z).max(0.0001);
    let right = [direction.z / horizontal, 0.0, -direction.x / horizontal];
    // up = direction × right, so it stays perpendicular to the aim at any
    // pitch. (A sign slip here squashed and then flipped the vertical spread
    // when aiming up at the helicopter.)
    let up = [
        direction.y * right[2],
        direction.z * right[0] - direction.x * right[2],
        -direction.y * right[0],
    ];
    let pattern = [
        (0.0, 0.0),
        (-0.7, -0.3),
        (0.7, -0.3),
        (-0.4, 0.7),
        (0.4, 0.7),
        (0.0, -0.9),
        (-1.0, 0.2),
        (1.0, 0.2),
    ];
    let mut hits = 0;
    let endpoints = std::array::from_fn(|i| {
        let (x, y) = pattern[i];
        let v = [
            direction.x + spread * (right[0] * x + up[0] * y),
            direction.y + spread * up[1] * y,
            direction.z + spread * (right[2] * x + up[2] * y),
        ];
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(0.0001);
        let v = v.map(|n| n / len);
        let at = |d| WorldPosition {
            x: origin.x + v[0] * d,
            y: origin.y + v[1] * d,
            z: origin.z + v[2] * d,
        };
        let cover = first_cover_intersection(course, origin, at(SHOTGUN_RANGE_M))
            .map(|f| f * SHOTGUN_RANGE_M);
        let hit_distance = target.and_then(|target| {
            let offset = [
                target.x - origin.x,
                target.y - origin.y,
                target.z - origin.z,
            ];
            let along = offset[0] * v[0] + offset[1] * v[1] + offset[2] * v[2];
            let distance_squared = offset.iter().map(|v| v * v).sum::<f32>();
            let discriminant =
                along * along - distance_squared + crate::PLAYER_MISSILE_HIT_RADIUS_M.powi(2);
            if discriminant < 0.0 || along < 0.0 {
                return None;
            }
            let distance = (along - discriminant.sqrt()).max(0.0);
            (distance <= SHOTGUN_RANGE_M && cover.is_none_or(|c| distance < c)).then_some(distance)
        });
        if let Some(distance) = hit_distance {
            hits += 1;
            at(distance)
        } else {
            at(cover.unwrap_or(SHOTGUN_RANGE_M))
        }
    });
    (hits, endpoints)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pellets_miss_out_of_range_and_respect_bridge_cover() {
        let course = crate::riverside_course();
        let origin = WorldPosition {
            x: 0.0,
            y: 2.0,
            z: 0.0,
        };
        let forward = WorldPosition {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        };
        assert_eq!(
            trace_shotgun(
                course,
                origin,
                forward,
                true,
                Some(WorldPosition { z: 12.0, ..origin })
            )
            .0,
            8
        );
        assert_eq!(
            trace_shotgun(
                course,
                origin,
                forward,
                true,
                Some(WorldPosition { z: 50.0, ..origin })
            )
            .0,
            0
        );
        let bridge = course
            .cover_volumes()
            .iter()
            .find(|c| c.kind == crate::CoverKind::BridgeDeck)
            .unwrap();
        let origin = WorldPosition {
            x: bridge.center_x,
            y: 2.0,
            z: bridge.center_z,
        };
        let (_, endpoints) = trace_shotgun(
            course,
            origin,
            WorldPosition {
                x: 0.0,
                y: 1.0,
                z: 0.001,
            },
            true,
            Some(WorldPosition { y: 12.0, ..origin }),
        );
        assert!(endpoints.iter().all(|p| p.y <= bridge.min_y + 0.01));
    }
    #[test]
    fn spread_pattern_stays_symmetric_when_aiming_upward() {
        let course = crate::riverside_course();
        let origin = WorldPosition {
            x: 0.0,
            y: 2.0,
            z: 0.0,
        };
        for pitch in [0.0_f32, 0.45, 0.8, 1.1] {
            for yaw in [0.0_f32, 1.3, -2.4] {
                let direction = WorldPosition {
                    x: yaw.sin() * pitch.cos(),
                    y: pitch.sin(),
                    z: -yaw.cos() * pitch.cos(),
                };
                let (_, endpoints) = trace_shotgun(course, origin, direction, false, None);
                let angle_of = |p: &WorldPosition| {
                    let v = [p.x - origin.x, p.y - origin.y, p.z - origin.z];
                    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                    ((v[0] * direction.x + v[1] * direction.y + v[2] * direction.z) / len)
                        .clamp(-1.0, 1.0)
                        .acos()
                };
                // Pellet 5 is straight "down" (0, -0.9) in the pattern: its
                // angle must match the pattern radius at every pitch.
                let down = angle_of(&endpoints[5]);
                let expected = (SHOTGUN_HIP_SPREAD * 0.9).atan();
                assert!(
                    (down - expected).abs() < 0.002,
                    "pitch {pitch}: down pellet at {down:.4} rad, expected {expected:.4}"
                );
                // The upper pair (y = 0.7) sits above the aim line, never below.
                let above =
                    |p: &WorldPosition| (p.y - origin.y) - direction.y * SHOTGUN_RANGE_M > 0.0;
                assert!(
                    above(&endpoints[3]) && above(&endpoints[4]),
                    "pitch {pitch}"
                );
                assert!(
                    endpoints
                        .iter()
                        .all(|p| angle_of(p) <= SHOTGUN_HIP_SPREAD.atan() * 1.3)
                );
            }
        }
    }

    #[test]
    fn ads_tightens_spread_and_old_inputs_default_to_launcher() {
        let origin = WorldPosition {
            x: 0.0,
            y: 3.0,
            z: 0.0,
        };
        let direction = WorldPosition {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        };
        let target = Some(WorldPosition { z: 35.0, ..origin });
        let course = crate::riverside_course();
        assert!(
            trace_shotgun(course, origin, direction, true, target).0
                > trace_shotgun(course, origin, direction, false, target).0
        );
        let input: crate::GunnerInput =
            serde_json::from_str(r#"{"yaw":0,"pitch":0,"aim_held":true}"#).unwrap();
        assert_eq!(input.weapon, WeaponKind::Launcher);
    }
}
