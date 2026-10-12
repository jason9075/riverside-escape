//! Shared gameplay state and the small P1 WebSocket protocol.

mod weapons;
pub use weapons::*;

use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex, OnceLock},
};

use serde::{Deserialize, Serialize};

pub const SIMULATION_HZ: u32 = 60;
pub const SNAPSHOT_HZ: u32 = 20;
pub const COUNTDOWN_SECONDS: u8 = 5;
pub const MAX_SPEED: f32 = 13.0;
pub const TRACK_HALF_WIDTH: f32 = 2.3;
pub const BIKE_COLLISION_HALF_WIDTH: f32 = 0.28;
pub const ROAD_STRAIGHT_LENGTH_M: f32 = 8.0;
pub const ROAD_CURVE_RADIUS_M: f32 = 10.0;
pub const ROAD_CURVE_ANGLE_RADIANS: f32 = std::f32::consts::PI / 6.0;
pub const ROAD_CURVE_LENGTH_M: f32 = ROAD_CURVE_RADIUS_M * ROAD_CURVE_ANGLE_RADIANS;
pub const BRIDGE_SPAN_LENGTH_M: f32 = 32.0;
pub const BRIDGE_DECK_HALF_WIDTH_M: f32 = 6.5;
pub const BRIDGE_DECK_BOTTOM_M: f32 = 4.9;
pub const BRIDGE_DECK_TOP_M: f32 = 5.7;
pub const BRIDGE_PIER_OFFSET_M: f32 = 6.4;
pub const ESCAPE_FINISH_MARGIN_M: f32 = 1.0;
pub const FIXED_DT: f32 = 1.0 / SIMULATION_HZ as f32;
pub const FIXED_DT_SECONDS: f64 = 1.0 / SIMULATION_HZ as f64;
pub const INTERPOLATION_DELAY_TICKS: u64 = SIMULATION_HZ as u64 / SNAPSHOT_HZ as u64;
pub const PREDICTION_HISTORY_TICKS: usize = (SIMULATION_HZ * 5) as usize;
pub const MAX_INPUT_TICK_AGE: u64 = SIMULATION_HZ as u64 * 5;
pub const MAX_INPUT_TICK_FUTURE: u64 = 3;
pub const MAX_FRONT_STEER_ANGLE: f32 = 0.45;
pub const BIKE_PEDAL_ACCEL: f32 = 7.0;
/// Fraction of the pedal acceleration lost at top speed.
pub const BIKE_PEDAL_FALLOFF: f32 = 0.6;
pub const BIKE_BRAKE_DECEL: f32 = 9.0;
pub const BIKE_COAST_DECEL: f32 = 0.4;
pub const BIKE_REVERSE_ACCEL: f32 = 3.0;
pub const BIKE_REVERSE_MAX_SPEED: f32 = 2.5;
pub const BIKE_REVERSE_DELAY_SECS: f32 = 0.2;
pub const BIKE_FORK_STEER_RATE: f32 = 6.0;
pub const BIKE_YAW_RATE: f32 = 1.9;
pub const BIKE_FULL_STEER_SPEED: f32 = 2.5;
pub const BIKE_BRAKE_STEER_MULTIPLIER: f32 = 1.5;
/// Front and rear collision probes along the bike centerline.
pub const BIKE_PROBE_OFFSET_M: f32 = 0.45;
pub const BIKE_CONTACT_SPEED_LOSS: f32 = 0.6;
pub const BIKE_CONTACT_DEFLECT: f32 = 0.35;
pub const BIKE_CONTACT_REPORT_SPEED_LOSS: f32 = 0.5;
pub const BIKE_CRASH_HEAD_ON_COS: f32 = 0.7;
pub const BIKE_CRASH_SPEED: f32 = MAX_SPEED * 0.7;
/// Speed kept after a fast head-on hit on a large obstacle (barrier).
pub const BIKE_CRASH_SPEED_KEPT: f32 = 0.25;
/// Small solid obstacles (bollards) only take part of the speed.
pub const BIKE_SMALL_CRASH_SPEED_KEPT: f32 = 0.55;
/// Knocking a cone aside trims speed without stopping the bike.
pub const BIKE_CONE_SPEED_KEPT: f32 = 0.85;
pub const GRASS_MAX_SPEED_RATIO: f32 = 0.55;
pub const GRASS_SCRUB_DECEL: f32 = 6.0;
pub const WATER_MAX_SPEED_RATIO: f32 = 0.25;
pub const WATER_SCRUB_DECEL: f32 = 14.0;
/// Paved paths are rail-free; the park ends this far from the nearest path.
pub const PARK_BOUNDARY_M: f32 = 20.0;
pub const BRANCH_HALF_WIDTH_M: f32 = 1.8;
pub const GRAVITY: f32 = 9.8;
/// Vertical launch speed as a fraction of forward speed at a ramp lip.
pub const BIKE_RAMP_LAUNCH_RATIO: f32 = 0.35;
pub const BIKE_MIN_LAUNCH_SPEED: f32 = 2.0;
pub const BIKE_AIR_STEER_MULTIPLIER: f32 = 0.25;
pub const BIKE_LANDING_BOOST: f32 = 1.0;
pub const BIKE_SPLASH_SPEED_KEPT: f32 = 0.45;
pub const BIKE_UNSTUCK_SPEED: f32 = 4.0;
pub const BIKE_UNSTUCK_NUDGE_MPS: f32 = 1.5;
pub const BIKE_UNSTUCK_YAW_RATE: f32 = 1.2;
pub const BIKE_RENDER_CORRECTION_TAU_SECS: f32 = 0.085;
pub const HELICOPTER_CHASE_SECONDS: f32 = 3.0;
pub const ROCKET_WARNING_SECONDS: f32 = 2.0;
pub const ROCKET_FLIGHT_SECONDS: f32 = 0.8;
pub const HELICOPTER_REPOSITION_SECONDS: f32 = 2.0;
pub const HELICOPTER_CHASE_DISTANCE_M: f32 = 14.0;
pub const HELICOPTER_REPOSITION_OFFSET_M: f32 = 8.0;
pub const HELICOPTER_ALTITUDE_M: f32 = 8.0;
pub const HELICOPTER_SPEED_MPS: f32 = 9.0;
pub const ROCKET_AIM_LEAD_SECONDS: f32 = 0.6;
pub const ROCKET_BLAST_RADIUS_M: f32 = 2.5;
pub const COMBAT_EVENT_RETENTION_SECONDS: u64 = 5;
pub const COMBAT_EVENT_RETENTION_TICKS: u64 = SIMULATION_HZ as u64 * COMBAT_EVENT_RETENTION_SECONDS;
pub const GUNNER_LOCK_CONE_HALF_ANGLE_DEGREES: f32 = 6.0;
pub const GUNNER_LOCK_CONE_HALF_ANGLE_RADIANS: f32 =
    std::f32::consts::PI * GUNNER_LOCK_CONE_HALF_ANGLE_DEGREES / 180.0;
pub const GUNNER_LOCK_DISTANCE_M: f32 = 120.0;
pub const GUNNER_LOCK_SECONDS: f32 = 2.0;
pub const GUNNER_LOCK_TICKS: u64 = (SIMULATION_HZ as f32 * GUNNER_LOCK_SECONDS) as u64;
pub const GUNNER_LOCK_GRACE_SECONDS: f32 = 0.2;
pub const GUNNER_LOCK_GRACE_TICKS: u64 = (SIMULATION_HZ as f32 * GUNNER_LOCK_GRACE_SECONDS) as u64;
pub const GUNNER_PITCH_MIN_RADIANS: f32 = -std::f32::consts::PI / 4.0;
pub const GUNNER_PITCH_MAX_RADIANS: f32 = std::f32::consts::PI * 75.0 / 180.0;
pub const GUNNER_AIM_ORIGIN_HEIGHT_M: f32 = 1.84;
pub const GUNNER_AIM_ORIGIN_REAR_M: f32 = 1.03;
pub const PLAYER_MISSILE_SPEED_MPS: f32 = 42.0;
pub const PLAYER_MISSILE_HIT_RADIUS_M: f32 = 1.75;
pub const PLAYER_MISSILE_LIFETIME_SECONDS: f32 = 6.0;
pub const PLAYER_MISSILE_LIFETIME_TICKS: u64 =
    (SIMULATION_HZ as f32 * PLAYER_MISSILE_LIFETIME_SECONDS) as u64;
pub const GUNNER_FIRE_COOLDOWN_SECONDS: f32 = 2.0;
pub const GUNNER_FIRE_COOLDOWN_TICKS: u64 =
    (SIMULATION_HZ as f32 * GUNNER_FIRE_COOLDOWN_SECONDS) as u64;
pub const GUNNER_STARTING_AMMO: u8 = 6;
pub const BIKE_STARTING_HEALTH: u16 = 100;
pub const BIKE_ROCKET_DAMAGE: u16 = 25;
pub const HELICOPTER_STARTING_HEALTH: u16 = 100;
pub const PLAYER_MISSILE_DAMAGE: u16 = 34;
const MAX_EXTRAPOLATION_TICKS: f64 = (SIMULATION_HZ / 10) as f64;

pub const HELICOPTER_CHASE_TICKS: u64 = (SIMULATION_HZ as f32 * HELICOPTER_CHASE_SECONDS) as u64;
pub const ROCKET_WARNING_TICKS: u64 = (SIMULATION_HZ as f32 * ROCKET_WARNING_SECONDS) as u64;
pub const ROCKET_FLIGHT_TICKS: u64 = (SIMULATION_HZ as f32 * ROCKET_FLIGHT_SECONDS) as u64;
pub const HELICOPTER_REPOSITION_TICKS: u64 =
    (SIMULATION_HZ as f32 * HELICOPTER_REPOSITION_SECONDS) as u64;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeatKind {
    #[default]
    Rider,
    Gunner,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeatChoiceRejection {
    SeatOccupied,
    MatchLocked,
    NotRoomMember,
    SoloUsesRoleSwitch,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeatSwapRejection {
    SeatsNotOccupied,
    AlreadyPending,
    MatchLocked,
    NoPendingRequest,
    NotSwapTarget,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeatSwapCancellationReason {
    Rejected,
    Expired,
    MemberLeft,
    SeatChanged,
}

impl SeatKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Rider => "Rider",
            Self::Gunner => "Gunner",
        }
    }
}

/// Seats required to start the current prototype match. Extend this set together with `SeatKind`.
pub const REQUIRED_SEATS: [SeatKind; 2] = [SeatKind::Rider, SeatKind::Gunner];
pub const SEAT_SWAP_TIMEOUT_SECONDS: u8 = 15;
pub const RECONNECT_RESERVATION_SECONDS: u8 = 30;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GamePhase {
    #[default]
    WaitingForPlayers,
    ReadyCheck,
    Countdown,
    Paused,
    Running,
    Finished,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchResult {
    RidersVictory,
    RidersDefeat,
    Aborted,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchVictoryReason {
    HelicopterDestroyed,
    Escaped,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DamageTarget {
    Bike,
    Helicopter,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FireRejectionReason {
    NotRunning,
    StaleInput,
    NotAiming,
    NoTarget,
    TargetOutsideCone,
    OutOfRange,
    Obstructed,
    NotLocked,
    Cooldown,
    NoAmmo,
    MissileInFlight,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct RiderInput {
    pub pedal_held: bool,
    pub brake_held: bool,
    /// -1 turns left; +1 turns right. The key binding is a replaceable prototype choice.
    pub steer: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiderInputSample {
    pub sequence: u32,
    pub tick: u64,
    pub input: RiderInput,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct GunnerInput {
    #[serde(default)]
    pub weapon: WeaponKind,
    /// Chair yaw offset in radians from its rear-facing prototype home heading.
    pub yaw: f32,
    /// Camera pitch in radians.
    pub pitch: f32,
    pub aim_held: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ClientMessage {
    ChooseSeat {
        seat: SeatKind,
    },
    RequestSeatSwap,
    RespondSeatSwap {
        request_id: u64,
        accept: bool,
    },
    SetReady {
        ready: bool,
    },
    LeaveRoom,
    ForceStartSolo,
    SwitchSeat {
        seat: SeatKind,
    },
    RiderInput {
        sequence: u32,
        tick: u64,
        input: RiderInput,
    },
    GunnerInput {
        sequence: u32,
        tick: u64,
        input: GunnerInput,
    },
    FireRequest {
        request_id: u64,
        tick: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ServerMessage {
    Welcome {
        player_id: u64,
        room_code: String,
        seat: Option<SeatKind>,
        solo_mode: bool,
        #[serde(default)]
        reconnect_token: Option<String>,
    },
    SeatAssigned {
        seat: SeatKind,
    },
    SeatChoiceRejected {
        seat: SeatKind,
        reason: SeatChoiceRejection,
    },
    SeatSwapProposed {
        request_id: u64,
        requester_id: u64,
        target_id: u64,
        from_seat: SeatKind,
        to_seat: SeatKind,
        expires_in_seconds: u8,
    },
    SeatSwapProposalRejected {
        reason: SeatSwapRejection,
    },
    SeatSwapCancelled {
        request_id: u64,
        reason: SeatSwapCancellationReason,
    },
    SeatSwapCompleted {
        request_id: u64,
        seat: SeatKind,
    },
    SeatChanged {
        seat: SeatKind,
    },
    Snapshot(Box<WorldSnapshot>),
    RoomFull,
    RoomLimitReached,
    RoomNotFound,
    RoomSelectionRequired,
    InvalidRoomCode,
    SoloUnavailable,
    ReconnectUnavailable,
    ReconnectInUse,
    RoomLeft,
    ConnectionLost,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct SeatStatus {
    pub connected: bool,
    pub ready: bool,
    #[serde(default)]
    pub reserved: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct WorldPosition {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HelicopterPhase {
    #[default]
    Chasing,
    Telegraph,
    RocketInFlight,
    Repositioning,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct RocketWarning {
    pub attack_id: u64,
    /// Ground position selected when the warning begins; it does not track later bike motion.
    pub impact_position: WorldPosition,
    pub impact_tick: u64,
    pub blast_radius_m: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct HelicopterSnapshot {
    pub position: WorldPosition,
    pub yaw: f32,
    pub phase: HelicopterPhase,
    pub warning: Option<RocketWarning>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct RocketSnapshot {
    pub attack_id: u64,
    pub launch_position: WorldPosition,
    pub position: WorldPosition,
    pub impact_position: WorldPosition,
    pub launch_tick: u64,
    pub impact_tick: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct PlayerMissileSnapshot {
    pub attack_id: u64,
    pub launch_position: WorldPosition,
    pub position: WorldPosition,
    pub launch_tick: u64,
    pub expires_tick: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CombatEventKind {
    RocketWarning,
    RocketLaunched,
    RocketExploded {
        hit: bool,
        hit_count: u32,
    },
    BikeDamaged {
        health_remaining: u16,
    },
    GunnerFireAccepted {
        request_id: u64,
        ammo_remaining: u8,
    },
    GunnerFireRejected {
        request_id: u64,
        reason: FireRejectionReason,
    },
    ShotgunFired {
        request_id: u64,
        ammo_remaining: u8,
        pellets_hit: u8,
        helicopter_health: u16,
        endpoints: [WorldPosition; SHOTGUN_PELLETS],
    },
    PlayerMissileImpacted {
        hit: bool,
        helicopter_health: u16,
    },
    MatchFinished {
        result: MatchResult,
        #[serde(default)]
        victory_reason: Option<MatchVictoryReason>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct CombatEvent {
    pub event_id: u64,
    pub attack_id: u64,
    pub tick: u64,
    pub position: WorldPosition,
    pub kind: CombatEventKind,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct AimEvaluation {
    pub distance_m: f32,
    pub angle_degrees: f32,
    pub in_cone: bool,
    pub in_range: bool,
    pub obstructed: bool,
}

/// World-space Gunner origin behind the bike, using the shared gameplay yaw
/// convention where positive yaw turns toward +X from the starting heading.
pub fn gunner_aim_origin(bike: BikeState) -> WorldPosition {
    WorldPosition {
        x: bike.x - bike.yaw.sin() * GUNNER_AIM_ORIGIN_REAR_M,
        y: bike.y + GUNNER_AIM_ORIGIN_HEIGHT_M,
        z: bike.z + bike.yaw.cos() * GUNNER_AIM_ORIGIN_REAR_M,
    }
}

pub fn gunner_aim_direction(bike: BikeState, input: GunnerInput) -> WorldPosition {
    let yaw = bike.yaw + std::f32::consts::PI + input.yaw;
    let pitch_cos = input.pitch.cos();
    WorldPosition {
        x: yaw.sin() * pitch_cos,
        y: input.pitch.sin(),
        z: -yaw.cos() * pitch_cos,
    }
}

pub fn evaluate_gunner_aim(
    course: &RiversideCourse,
    bike: BikeState,
    input: GunnerInput,
    target: WorldPosition,
) -> AimEvaluation {
    let origin = gunner_aim_origin(bike);
    let dx = target.x - origin.x;
    let dy = target.y - origin.y;
    let dz = target.z - origin.z;
    let distance_squared = dx * dx + dy * dy + dz * dz;
    let distance_m = distance_squared.sqrt();
    let direction = gunner_aim_direction(bike, input);
    let dot = if distance_m > f32::EPSILON {
        ((direction.x * dx + direction.y * dy + direction.z * dz) / distance_m).clamp(-1.0, 1.0)
    } else {
        1.0
    };
    let angle_degrees = dot.acos().to_degrees();
    AimEvaluation {
        distance_m,
        angle_degrees,
        in_cone: angle_degrees <= GUNNER_LOCK_CONE_HALF_ANGLE_DEGREES,
        in_range: distance_m <= GUNNER_LOCK_DISTANCE_M,
        obstructed: line_of_sight_blocked(course, origin, target),
    }
}

pub fn line_of_sight_blocked(
    course: &RiversideCourse,
    origin: WorldPosition,
    target: WorldPosition,
) -> bool {
    for obstacle in course.obstacles() {
        let (height, padding) = match obstacle.kind {
            ObstacleKind::Cone => (0.9, 0.08),
            ObstacleKind::Bollard => (1.2, 0.08),
            ObstacleKind::Barrier => (1.4, 0.0),
        };
        // Test the segment in the obstacle's local frame so road-aligned
        // obstacles on angled straights keep their exact footprint.
        let local = |position: WorldPosition| {
            let (x, z) = obstacle.to_local(position.x, position.z);
            WorldPosition {
                x,
                y: position.y,
                z,
            }
        };
        let (local_origin, local_target) = (local(origin), local(target));
        let Some((entry, exit)) = segment_box_interval(
            local_origin,
            local_target,
            -obstacle.half_width - padding,
            obstacle.half_width + padding,
            -obstacle.half_depth - padding,
            obstacle.half_depth + padding,
        ) else {
            continue;
        };
        let t = entry.clamp(0.0, 1.0);
        if t <= exit && origin.y + (target.y - origin.y) * t <= height {
            return true;
        }
    }
    first_cover_intersection(course, origin, target).is_some()
}

/// Returns the first entry fraction along a 3D line segment that intersects
/// shared, server-safe cover geometry.
pub fn first_cover_intersection(
    course: &RiversideCourse,
    origin: WorldPosition,
    target: WorldPosition,
) -> Option<f32> {
    course
        .cover_volumes()
        .iter()
        .filter_map(|cover| {
            segment_cover_interval(origin, target, *cover).map(|interval| interval.0)
        })
        .min_by(f32::total_cmp)
}

fn segment_box_interval(
    origin: WorldPosition,
    target: WorldPosition,
    min_x: f32,
    max_x: f32,
    min_z: f32,
    max_z: f32,
) -> Option<(f32, f32)> {
    let mut entry: f32 = 0.0;
    let mut exit: f32 = 1.0;
    let delta_x = target.x - origin.x;
    let delta_z = target.z - origin.z;
    for (origin, delta, min, max) in [
        (origin.x, delta_x, min_x, max_x),
        (origin.z, delta_z, min_z, max_z),
    ] {
        if delta.abs() <= f32::EPSILON {
            if origin < min || origin > max {
                return None;
            }
            continue;
        }
        let a = (min - origin) / delta;
        let b = (max - origin) / delta;
        entry = entry.max(a.min(b));
        exit = exit.min(a.max(b));
        if entry > exit {
            return None;
        }
    }
    (exit >= 0.0 && entry <= 1.0).then_some((entry, exit))
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WorldSnapshot {
    #[serde(default)]
    pub gunner_input: GunnerInput,
    #[serde(default)]
    pub rider_pedaling: bool,
    #[serde(default = "default_shotgun_ammo")]
    pub shotgun_ammo: u8,
    pub tick: u64,
    pub phase: GamePhase,
    pub seats: BTreeMap<SeatKind, SeatStatus>,
    /// Room members that have not selected a Rider or Gunner seat yet.
    #[serde(default)]
    pub unassigned_players: u8,
    #[serde(default)]
    pub reconnect_seconds_remaining: Option<u8>,
    pub countdown_seconds: Option<u8>,
    pub bike: BikeState,
    #[serde(default)]
    pub route_section: RoadSection,
    #[serde(default)]
    pub route_distance_m: f32,
    #[serde(default)]
    pub route_checkpoints_passed: u8,
    #[serde(default)]
    pub route_checkpoints_total: u8,
    #[serde(default)]
    pub route_length_m: f32,
    /// Seed of the round's course layout; `AUTHORED_COURSE_SEED` is the baseline.
    #[serde(default)]
    pub course_seed: u64,
    /// Highest Rider input sequence applied by the simulation up through this snapshot tick.
    #[serde(default)]
    pub last_applied_rider_sequence: u32,
    #[serde(default)]
    pub helicopter: Option<HelicopterSnapshot>,
    #[serde(default)]
    pub rocket: Option<RocketSnapshot>,
    /// Repeated briefly in snapshots; clients deduplicate by event_id.
    #[serde(default)]
    pub combat_events: Vec<CombatEvent>,
    #[serde(default)]
    pub rocket_hit_count: u32,
    #[serde(default)]
    pub player_missile: Option<PlayerMissileSnapshot>,
    #[serde(default = "default_starting_bike_health")]
    pub bike_health: u16,
    #[serde(default = "default_starting_helicopter_health")]
    pub helicopter_health: u16,
    #[serde(default = "default_starting_ammo")]
    pub gunner_ammo: u8,
    #[serde(default)]
    pub gunner_lock_progress_ticks: u16,
    #[serde(default)]
    pub gunner_lock_grace_ticks_remaining: u16,
    #[serde(default)]
    pub gunner_locked: bool,
    #[serde(default)]
    pub gunner_can_fire: bool,
    #[serde(default)]
    pub fire_cooldown_ticks_remaining: u16,
    #[serde(default)]
    pub match_result: Option<MatchResult>,
    #[serde(default)]
    pub match_victory_reason: Option<MatchVictoryReason>,
}

fn default_starting_bike_health() -> u16 {
    BIKE_STARTING_HEALTH
}

fn default_starting_helicopter_health() -> u16 {
    HELICOPTER_STARTING_HEALTH
}

fn default_starting_ammo() -> u8 {
    GUNNER_STARTING_AMMO
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct BikeState {
    pub x: f32,
    pub z: f32,
    pub yaw: f32,
    pub speed: f32,
    /// Front fork/handlebar angle relative to the bicycle frame.
    #[serde(default)]
    pub steering_angle: f32,
    /// Time spent stopped with only the brake held; reverse engages after a short delay.
    #[serde(default)]
    pub stopped_brake_secs: f32,
    /// Height above the lawn; non-zero on ramps and in the air.
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub vy: f32,
    #[serde(default)]
    pub airborne: bool,
}

impl Default for BikeState {
    fn default() -> Self {
        Self {
            x: 0.0,
            z: 2.0,
            yaw: 0.0,
            speed: 0.0,
            steering_angle: 0.0,
            stopped_brake_secs: 0.0,
            y: 0.0,
            vy: 0.0,
            airborne: false,
        }
    }
}

impl BikeState {
    /// Arcade/kinematic bicycle controller shared by offline preview and the server.
    /// `steer < 0` is left and `steer > 0` is right; positive yaw turns right.
    ///
    /// Handling follows the Road Redemption reference: the heading responds
    /// almost immediately with a near speed-independent yaw rate, glancing
    /// contacts slide instead of stopping, and only fast head-on impacts count
    /// as a crash. Returns true when the bike lost meaningful speed to contact.
    pub fn step(&mut self, input: RiderInput, dt: f32, course: &RiversideCourse) -> bool {
        let dt = dt.clamp(0.0, 0.05);
        let steer = input.steer.clamp(-1.0, 1.0);
        let surface = course.surface_at(self.x, self.z);
        if !self.airborne {
            self.step_throttle(input, dt, surface);
        }
        let previous_speed = self.speed;

        self.steering_angle = move_toward(
            self.steering_angle,
            steer * MAX_FRONT_STEER_ANGLE,
            BIKE_FORK_STEER_RATE * dt,
        );
        // Body yaw still only changes while rolling, but reaches full authority
        // at a walking pace so low-speed dodges stay responsive.
        let steer_ratio = self.steering_angle / MAX_FRONT_STEER_ANGLE;
        let rolling = (self.speed / BIKE_FULL_STEER_SPEED).clamp(-1.0, 1.0);
        let mut yaw_rate = BIKE_YAW_RATE * steer_ratio * rolling;
        if self.airborne {
            yaw_rate *= BIKE_AIR_STEER_MULTIPLIER;
        } else if input.brake_held && self.speed > BIKE_FULL_STEER_SPEED {
            yaw_rate *= BIKE_BRAKE_STEER_MULTIPLIER;
        }
        self.yaw = (self.yaw + yaw_rate * dt).rem_euclid(std::f32::consts::TAU);

        let (start_x, start_z) = (self.x, self.z);
        self.x += self.yaw.sin() * self.speed * dt;
        self.z -= self.yaw.cos() * self.speed * dt;

        let mut impact = self.step_vertical(dt, course, start_x, start_z);

        let mut touching_obstacle = false;
        for &obstacle in course.obstacles() {
            if self.y > obstacle.height() {
                continue;
            }
            if obstacle.kind == ObstacleKind::Cone {
                // Cones are knocked aside: entering one only trims speed.
                let was_inside = self.overlaps_obstacle(obstacle, start_x, start_z);
                if !was_inside && self.overlaps_obstacle(obstacle, self.x, self.z) {
                    self.speed *= BIKE_CONE_SPEED_KEPT;
                    impact = true;
                }
                continue;
            }
            for probe in [BIKE_PROBE_OFFSET_M, -BIKE_PROBE_OFFSET_M] {
                if let Some(normal) = self.push_out_of_obstacle(obstacle, probe) {
                    touching_obstacle = true;
                    impact |= self.absorb_impact(normal, Some(obstacle.crash_speed_kept()));
                }
            }
        }
        // Low-speed contact turns steering into a sideways nudge so the
        // Rider can always work free of an obstacle without reversing.
        if touching_obstacle && self.speed.abs() < BIKE_UNSTUCK_SPEED && steer != 0.0 {
            let nudge = steer * BIKE_UNSTUCK_NUDGE_MPS * dt;
            self.x += self.yaw.cos() * nudge;
            self.z += self.yaw.sin() * nudge;
            self.yaw =
                (self.yaw + steer * BIKE_UNSTUCK_YAW_RATE * dt).rem_euclid(std::f32::consts::TAU);
        }

        // There are no road rails any more; only the park boundary, a fixed
        // distance from every paved path, stops the bike.
        let nearest = course.nearest_road_point(self.x, self.z);
        if nearest.distance_m > PARK_BOUNDARY_M {
            let (dx, dz) = (self.x - nearest.x, self.z - nearest.z);
            let scale = PARK_BOUNDARY_M / nearest.distance_m;
            self.x = nearest.x + dx * scale;
            self.z = nearest.z + dz * scale;
            let normal = (-dx / nearest.distance_m, -dz / nearest.distance_m);
            self.absorb_impact(normal, None);
        }

        impact || previous_speed.abs() > self.speed.abs() + BIKE_CONTACT_REPORT_SPEED_LOSS
    }

    /// Ramps lift the bike; leaving a ramp lip with speed launches it.
    /// Returns true when a landing cost speed (splashing into water).
    fn step_vertical(
        &mut self,
        dt: f32,
        course: &RiversideCourse,
        start_x: f32,
        start_z: f32,
    ) -> bool {
        let ground = course.ground_height(self.x, self.z);
        if self.airborne {
            self.vy -= GRAVITY * dt;
            self.y += self.vy * dt;
            if self.y > ground {
                return false;
            }
            self.y = ground;
            self.vy = 0.0;
            self.airborne = false;
            return match course.surface_at(self.x, self.z) {
                Surface::Water => {
                    self.speed *= BIKE_SPLASH_SPEED_KEPT;
                    true
                }
                Surface::Road => {
                    self.speed = (self.speed + BIKE_LANDING_BOOST).min(MAX_SPEED);
                    false
                }
                Surface::Grass => false,
            };
        }
        let previous_ground = course.ground_height(start_x, start_z);
        if previous_ground > ground + 0.05 && self.speed > BIKE_MIN_LAUNCH_SPEED {
            self.airborne = true;
            self.y = previous_ground;
            self.vy = self.speed * BIKE_RAMP_LAUNCH_RATIO;
        } else {
            self.y = ground;
        }
        false
    }

    fn overlaps_obstacle(&self, obstacle: Obstacle, x: f32, z: f32) -> bool {
        let forward = (self.yaw.sin(), -self.yaw.cos());
        [BIKE_PROBE_OFFSET_M, -BIKE_PROBE_OFFSET_M]
            .into_iter()
            .any(|probe| {
                let (local_x, local_z) =
                    obstacle.to_local(x + forward.0 * probe, z + forward.1 * probe);
                let dx = local_x.abs() - obstacle.half_width;
                let dz = local_z.abs() - obstacle.half_depth;
                let (dx, dz) = (dx.max(0.0), dz.max(0.0));
                dx * dx + dz * dz < BIKE_COLLISION_HALF_WIDTH * BIKE_COLLISION_HALF_WIDTH
            })
    }

    fn step_throttle(&mut self, input: RiderInput, dt: f32, surface: Surface) {
        let (top_speed, accel_scale, scrub) = match surface {
            Surface::Road => (MAX_SPEED, 1.0, 0.0),
            Surface::Grass => (MAX_SPEED * GRASS_MAX_SPEED_RATIO, 0.7, GRASS_SCRUB_DECEL),
            Surface::Water => (MAX_SPEED * WATER_MAX_SPEED_RATIO, 0.5, WATER_SCRUB_DECEL),
        };
        if self.speed.abs() < 0.05 && input.brake_held && !input.pedal_held {
            self.stopped_brake_secs += dt;
        } else {
            self.stopped_brake_secs = 0.0;
        }
        let reversing = self.speed < 0.0 || self.stopped_brake_secs >= BIKE_REVERSE_DELAY_SECS;
        if reversing {
            let reverse_top = BIKE_REVERSE_MAX_SPEED.min(top_speed);
            if input.pedal_held {
                self.speed = (self.speed + BIKE_BRAKE_DECEL * dt).min(0.0);
            } else if input.brake_held {
                self.speed = (self.speed - BIKE_REVERSE_ACCEL * accel_scale * dt).max(-reverse_top);
            } else {
                self.speed = (self.speed + BIKE_COAST_DECEL * 4.0 * dt).min(0.0);
            }
            if self.speed < -reverse_top {
                self.speed = (self.speed + scrub * dt).min(-reverse_top);
            }
            if self.speed < 0.0 || input.pedal_held {
                return;
            }
        }
        if input.brake_held {
            self.speed = (self.speed - BIKE_BRAKE_DECEL * dt).max(0.0);
        } else if input.pedal_held && self.speed < top_speed {
            // Strong launch that tapers toward top speed.
            let falloff = 1.0 - BIKE_PEDAL_FALLOFF * (self.speed / top_speed).clamp(0.0, 1.0);
            self.speed =
                (self.speed + BIKE_PEDAL_ACCEL * accel_scale * falloff * dt).min(top_speed);
        } else {
            self.speed = (self.speed - BIKE_COAST_DECEL * dt).max(0.0);
        }
        // Soft ground scrubs off speed above its own top speed.
        if self.speed > top_speed {
            self.speed = (self.speed - scrub * dt).max(top_speed);
        }
    }

    /// Pushes a point on the bike's centerline out of an obstacle box and
    /// returns the outward world-space contact normal when they overlapped.
    fn push_out_of_obstacle(
        &mut self,
        obstacle: Obstacle,
        probe_offset: f32,
    ) -> Option<(f32, f32)> {
        let forward = (self.yaw.sin(), -self.yaw.cos());
        let (probe_x, probe_z) = obstacle.to_local(
            self.x + forward.0 * probe_offset,
            self.z + forward.1 * probe_offset,
        );
        let (half_x, half_z) = (obstacle.half_width, obstacle.half_depth);
        let closest_x = probe_x.clamp(-half_x, half_x);
        let closest_z = probe_z.clamp(-half_z, half_z);
        let (dx, dz) = (probe_x - closest_x, probe_z - closest_z);
        let distance = (dx * dx + dz * dz).sqrt();
        if distance >= BIKE_COLLISION_HALF_WIDTH {
            return None;
        }
        let (normal, target) = if distance > 1e-4 {
            let normal = (dx / distance, dz / distance);
            let push = BIKE_COLLISION_HALF_WIDTH - distance;
            (
                normal,
                (probe_x + normal.0 * push, probe_z + normal.1 * push),
            )
        } else {
            // The probe is inside the box: leave through the nearest face.
            let faces = [
                (probe_x + half_x, (-1.0, 0.0)),
                (half_x - probe_x, (1.0, 0.0)),
                (probe_z + half_z, (0.0, -1.0)),
                (half_z - probe_z, (0.0, 1.0)),
            ];
            let (_, normal) = faces
                .into_iter()
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .expect("four faces");
            let edge = (
                if normal.0 == 0.0 {
                    probe_x
                } else {
                    normal.0 * half_x
                },
                if normal.1 == 0.0 {
                    probe_z
                } else {
                    normal.1 * half_z
                },
            );
            let target = (
                edge.0 + normal.0 * BIKE_COLLISION_HALF_WIDTH,
                edge.1 + normal.1 * BIKE_COLLISION_HALF_WIDTH,
            );
            (normal, target)
        };
        let (world_probe_x, world_probe_z) = obstacle.to_world(probe_x, probe_z);
        let (world_target_x, world_target_z) = obstacle.to_world(target.0, target.1);
        self.x += world_target_x - world_probe_x;
        self.z += world_target_z - world_probe_z;
        Some(local_to_world(normal.0, normal.1, obstacle.yaw))
    }

    /// Removes the velocity component into a contact surface. Glancing hits
    /// keep most speed and deflect the heading along the surface; a fast
    /// head-on obstacle hit is a crash that costs most of the speed.
    fn absorb_impact(&mut self, normal: (f32, f32), crash_speed_kept: Option<f32>) -> bool {
        let direction = self.speed.signum();
        let forward = (self.yaw.sin() * direction, -self.yaw.cos() * direction);
        let into_surface = -(forward.0 * normal.0 + forward.1 * normal.1);
        if into_surface <= 0.0 || self.speed == 0.0 {
            return false;
        }
        if let Some(kept) = crash_speed_kept
            && into_surface > BIKE_CRASH_HEAD_ON_COS
            && self.speed > BIKE_CRASH_SPEED
        {
            self.speed *= kept;
            return true;
        }
        let tangent = (
            forward.0 + normal.0 * into_surface,
            forward.1 + normal.1 * into_surface,
        );
        let tangent_length = (tangent.0 * tangent.0 + tangent.1 * tangent.1).sqrt();
        self.speed *= (1.0 - into_surface * BIKE_CONTACT_SPEED_LOSS).max(0.0);
        if tangent_length > 1e-3 && direction > 0.0 {
            let tangent_yaw = (tangent.0 / tangent_length).atan2(-tangent.1 / tangent_length);
            let deflect = shortest_angle_delta(self.yaw, tangent_yaw) * BIKE_CONTACT_DEFLECT;
            self.yaw = (self.yaw + deflect).rem_euclid(std::f32::consts::TAU);
        }
        false
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoadSection {
    #[default]
    OpenRiverside,
    BridgeUnderpass,
    NarrowBends,
}

impl RoadSection {
    pub const fn label(self) -> &'static str {
        match self {
            Self::OpenRiverside => "Open Riverside",
            Self::BridgeUnderpass => "Bridge Underpass",
            Self::NarrowBends => "Narrow Bends",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoadPieceKind {
    Straight,
    CurveRight,
    CurveLeft,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoadPiece {
    pub kind: RoadPieceKind,
    pub section: RoadSection,
    /// Asset root pose. Curves turn in the asset's forward direction; a left
    /// turn uses the same GLB traversed from its exit back to its entry.
    pub root_x: f32,
    pub root_z: f32,
    pub root_yaw: f32,
    pub reversed: bool,
    pub start_distance_m: f32,
    pub length_m: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoverKind {
    BridgeDeck,
    BridgePier,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoverVolume {
    pub kind: CoverKind,
    pub center_x: f32,
    pub center_z: f32,
    pub yaw: f32,
    /// Half extent along the road's local X axis.
    pub half_width_m: f32,
    /// Half extent along the road's local Z axis.
    pub half_depth_m: f32,
    pub min_y: f32,
    pub max_y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CoursePoint {
    x: f32,
    z: f32,
    yaw: f32,
    distance_m: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CourseProjection {
    pub distance_m: f32,
    /// Signed offset from the centerline; positive is to the Rider's right.
    pub lateral_m: f32,
    pub center_distance_m: f32,
    pub section: RoadSection,
    pub center_x: f32,
    pub center_z: f32,
    pub yaw: f32,
}

/// What the bike is rolling on; the lawn and creeks are slower than paving.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Surface {
    Road,
    Grass,
    Water,
}

/// An oriented rectangle in the same local frame convention as obstacles:
/// local X across the heading, local Z along it (negative Z is forward).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundPatch {
    pub x: f32,
    pub z: f32,
    pub yaw: f32,
    pub half_width: f32,
    pub half_length: f32,
}

impl GroundPatch {
    pub fn to_local(self, x: f32, z: f32) -> (f32, f32) {
        let (dx, dz) = (x - self.x, z - self.z);
        let (sin, cos) = self.yaw.sin_cos();
        (dx * cos + dz * sin, -dx * sin + dz * cos)
    }

    pub fn contains(self, x: f32, z: f32) -> bool {
        let (local_x, local_z) = self.to_local(x, z);
        local_x.abs() <= self.half_width && local_z.abs() <= self.half_length
    }
}

/// A wedge rising toward its forward lip; the bike launches off the lip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ramp {
    pub patch: GroundPatch,
    pub height: f32,
}

impl Ramp {
    pub fn height_at(self, x: f32, z: f32) -> Option<f32> {
        if !self.patch.contains(x, z) {
            return None;
        }
        let (_, local_z) = self.patch.to_local(x, z);
        // Local -Z is forward, so the lip sits at -half_length.
        let rise = (self.patch.half_length - local_z) / (2.0 * self.patch.half_length);
        Some(self.height * rise.clamp(0.0, 1.0))
    }
}

/// An alternative paved path that leaves the main route and rejoins it.
#[derive(Clone, Debug, PartialEq)]
pub struct CourseBranch {
    pub start_m: f32,
    pub end_m: f32,
    points: Vec<CoursePoint>,
}

impl CourseBranch {
    pub fn length_m(&self) -> f32 {
        self.points.last().map_or(0.0, |point| point.distance_m)
    }

    /// Centreline point at a distance along the branch.
    pub fn point_at(&self, distance_m: f32) -> CourseProjection {
        polyline_point_at(&self.points, distance_m, RoadSection::OpenRiverside)
    }

    pub fn project(&self, x: f32, z: f32) -> CourseProjection {
        project_polyline(&self.points, x, z, |_| RoadSection::OpenRiverside)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NearestRoadPoint {
    pub x: f32,
    pub z: f32,
    pub distance_m: f32,
}

#[derive(Debug)]
pub struct RiversideCourse {
    seed: u64,
    obstacles: Vec<Obstacle>,
    branches: Vec<CourseBranch>,
    ramps: Vec<Ramp>,
    water: Vec<GroundPatch>,
    pieces: Vec<RoadPiece>,
    points: Vec<CoursePoint>,
    cover_volumes: Vec<CoverVolume>,
    checkpoints_m: Vec<f32>,
    finish_distance_m: f32,
    length_m: f32,
}

/// Seed of the hand-authored baseline layout used by tests and local preview.
pub const AUTHORED_COURSE_SEED: u64 = 0;

/// The baseline layout, built once from the same lightweight data used by
/// rendering, Rider prediction, and the headless Server.
pub fn riverside_course() -> &'static RiversideCourse {
    static COURSE: OnceLock<RiversideCourse> = OnceLock::new();
    COURSE.get_or_init(|| RiversideCourse::build(AUTHORED_COURSE_SEED))
}

/// Returns the course for a seed. Client and Server derive identical
/// geometry from the same seed, so only the seed crosses the network.
pub fn course_for_seed(seed: u64) -> Arc<RiversideCourse> {
    const CACHED_COURSES: usize = 8;
    static CACHE: OnceLock<Mutex<Vec<Arc<RiversideCourse>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(Vec::new()));
    let mut cache = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(index) = cache.iter().position(|course| course.seed == seed) {
        let course = cache.remove(index);
        cache.push(course.clone());
        return course;
    }
    let course = Arc::new(RiversideCourse::build(seed));
    if cache.len() >= CACHED_COURSES {
        cache.remove(0);
    }
    cache.push(course.clone());
    course
}

/// Small deterministic generator (SplitMix64). Integer-only state keeps
/// native Server and WASM Client layouts bit-identical.
struct CourseRng(u64);

impl CourseRng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform integer in `low..=high`.
    fn range(&mut self, low: usize, high: usize) -> usize {
        low + (self.next_u64() % (high - low + 1) as u64) as usize
    }

    /// Uniform float in `[0, 1)`.
    fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn chance(&mut self, probability: f32) -> bool {
        self.unit() < probability
    }
}

/// Shortcut geometry: the creek sits at this fraction of the chord, and the
/// ramp lip stops just short of it.
const SHORTCUT_CREEK_FRACTION: f32 = 0.55;
const SHORTCUT_CREEK_HALF_LENGTH_M: f32 = 3.5;
const SHORTCUT_CREEK_HALF_WIDTH_M: f32 = 10.0;
const SHORTCUT_RAMP_HALF_LENGTH_M: f32 = 2.0;
const SHORTCUT_RAMP_HEIGHT_M: f32 = 0.9;

impl RiversideCourse {
    fn build(seed: u64) -> Self {
        if seed == AUTHORED_COURSE_SEED {
            return Self::build_authored();
        }
        let mut rng = CourseRng(seed);
        let mut builder = CourseBuilder::new();

        // Fixed skeleton: open riverside bends with one long detour, the
        // bridge underpass landmark, then tighter technical bends. A straight
        // shortcut with a creek jump cuts across every detour.
        let open_groups = rng.range(3, 5);
        let detour_group = rng.range(0, open_groups - 1);
        for index in 0..open_groups {
            if index == detour_group {
                builder.straights(2, RoadSection::OpenRiverside);
                let straights = rng.range(3, 4);
                builder.detour(straights, RoadSection::OpenRiverside);
                builder.straights(3, RoadSection::OpenRiverside);
                continue;
            }
            let straights = rng.range(9, 13);
            let lead = straights / 2;
            builder.straights(lead, RoadSection::OpenRiverside);
            let first_turn = alternating_turn(index);
            builder.curve(first_turn, RoadSection::OpenRiverside);
            builder.straights(straights - lead, RoadSection::OpenRiverside);
            builder.curve(opposite_turn(first_turn), RoadSection::OpenRiverside);
        }

        builder.straights(50, RoadSection::BridgeUnderpass);

        let narrow_groups = rng.range(5, 7);
        let narrow_detour = rng.chance(0.6).then(|| rng.range(1, narrow_groups - 1));
        for index in 0..narrow_groups {
            if narrow_detour == Some(index) {
                builder.straights(2, RoadSection::NarrowBends);
                builder.detour(3, RoadSection::NarrowBends);
                builder.straights(3, RoadSection::NarrowBends);
                continue;
            }
            let first_turn = alternating_turn(index);
            builder.straights(rng.range(4, 6), RoadSection::NarrowBends);
            builder.curve(first_turn, RoadSection::NarrowBends);
            builder.straights(rng.range(2, 3), RoadSection::NarrowBends);
            builder.curve(opposite_turn(first_turn), RoadSection::NarrowBends);
        }

        let detours = builder.detours.clone();
        let mut course = builder.finish(seed);
        course.add_route_markers();
        course.add_bridge_cover();
        for (start_m, end_m) in detours {
            course.add_shortcut(start_m, end_m);
        }
        course.add_random_obstacles(&mut rng);
        course
    }

    fn build_authored() -> Self {
        let mut builder = CourseBuilder::new();

        // Four alternating S-bends keep the open section legible while
        // keeping the existing prototype obstacles on its initial straight.
        for (index, straights) in [11, 11, 11, 12].into_iter().enumerate() {
            let lead = straights / 2;
            builder.straights(lead, RoadSection::OpenRiverside);
            let first_turn = alternating_turn(index);
            builder.curve(first_turn, RoadSection::OpenRiverside);
            builder.straights(straights - lead, RoadSection::OpenRiverside);
            builder.curve(opposite_turn(first_turn), RoadSection::OpenRiverside);
        }

        builder.straights(50, RoadSection::BridgeUnderpass);

        // Repeated S-bends make the last section visibly more technical but
        // preserve broad enough radii for the existing bicycle controller.
        for index in 0..6 {
            let first_turn = alternating_turn(index);
            builder.straights(5, RoadSection::NarrowBends);
            builder.curve(first_turn, RoadSection::NarrowBends);
            builder.straights(2, RoadSection::NarrowBends);
            builder.curve(opposite_turn(first_turn), RoadSection::NarrowBends);
        }

        let mut course = builder.finish(AUTHORED_COURSE_SEED);
        course.add_route_markers();
        course.add_bridge_cover();
        course.obstacles = OBSTACLES.to_vec();
        course
    }

    /// A straight paved chord across a detour, interrupted by a creek. A
    /// ramp before the creek lets a fast bike fly over it; a slow one lands
    /// in the water and loses most of the time it hoped to save.
    fn add_shortcut(&mut self, start_m: f32, end_m: f32) {
        let from = self.point_at(start_m);
        let to = self.point_at(end_m);
        let (dx, dz) = (to.center_x - from.center_x, to.center_z - from.center_z);
        let length_m = (dx * dx + dz * dz).sqrt();
        let yaw = wrap_course_angle(dx.atan2(-dz));
        let steps = length_m.ceil() as usize;
        let points = (0..=steps)
            .map(|step| {
                let t = step as f32 / steps as f32;
                CoursePoint {
                    x: from.center_x + dx * t,
                    z: from.center_z + dz * t,
                    yaw,
                    distance_m: length_m * t,
                }
            })
            .collect();
        let branch = CourseBranch {
            start_m,
            end_m,
            points,
        };
        let creek_m = length_m * SHORTCUT_CREEK_FRACTION;
        let creek = branch.point_at(creek_m);
        self.water.push(GroundPatch {
            x: creek.center_x,
            z: creek.center_z,
            yaw,
            half_width: SHORTCUT_CREEK_HALF_WIDTH_M,
            half_length: SHORTCUT_CREEK_HALF_LENGTH_M,
        });
        let ramp = branch
            .point_at(creek_m - SHORTCUT_CREEK_HALF_LENGTH_M - 0.2 - SHORTCUT_RAMP_HALF_LENGTH_M);
        self.ramps.push(Ramp {
            patch: GroundPatch {
                x: ramp.center_x,
                z: ramp.center_z,
                yaw,
                half_width: BRANCH_HALF_WIDTH_M,
                half_length: SHORTCUT_RAMP_HALF_LENGTH_M,
            },
            height: SHORTCUT_RAMP_HEIGHT_M,
        });
        self.branches.push(branch);
    }

    /// Scatters at most one road-aligned obstacle per straight piece. Each
    /// obstacle sits off-centre so the opposite side always leaves a lane
    /// much wider than the bike, and gates and the start stay clear.
    fn add_random_obstacles(&mut self, rng: &mut CourseRng) {
        const START_CLEAR_M: f32 = 24.0;
        const GATE_CLEAR_M: f32 = 8.0;
        let mut gates = self.checkpoints_m.clone();
        gates.push(self.finish_distance_m);
        let mut previous_piece_used = false;
        for piece in self.pieces.clone() {
            if piece.kind != RoadPieceKind::Straight {
                previous_piece_used = false;
                continue;
            }
            let probability = match piece.section {
                RoadSection::OpenRiverside => 0.45,
                RoadSection::BridgeUnderpass => 0.22,
                RoadSection::NarrowBends => 0.35,
            };
            let roll = rng.chance(probability);
            let along_m = piece.start_distance_m + 2.0 + rng.unit() * 4.0;
            let kind = match rng.unit() {
                roll if roll < 0.45 => ObstacleKind::Cone,
                roll if roll < 0.8 => ObstacleKind::Bollard,
                _ => ObstacleKind::Barrier,
            };
            let side = if rng.chance(0.5) { -1.0 } else { 1.0 };
            let offset_m = match kind {
                ObstacleKind::Barrier => 0.4 + rng.unit() * 0.6,
                _ => 0.5 + rng.unit() * 1.0,
            };
            let near_gate = gates
                .iter()
                .any(|gate| (gate - along_m).abs() < GATE_CLEAR_M);
            if !roll
                || previous_piece_used
                || along_m < START_CLEAR_M
                || near_gate
                || along_m > self.length_m - GATE_CLEAR_M
            {
                previous_piece_used = false;
                continue;
            }
            let center = self.point_at(along_m);
            let lateral_m = side * offset_m;
            self.obstacles.push(Obstacle::new(
                kind,
                center.center_x + center.yaw.cos() * lateral_m,
                center.center_z + center.yaw.sin() * lateral_m,
                center.yaw,
            ));
            previous_piece_used = true;
        }
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn obstacles(&self) -> &[Obstacle] {
        &self.obstacles
    }

    pub fn pieces(&self) -> &[RoadPiece] {
        &self.pieces
    }

    pub fn length_m(&self) -> f32 {
        self.length_m
    }

    pub fn cover_volumes(&self) -> &[CoverVolume] {
        &self.cover_volumes
    }

    pub fn checkpoints_m(&self) -> &[f32] {
        &self.checkpoints_m
    }

    pub fn finish_distance_m(&self) -> f32 {
        self.finish_distance_m
    }

    /// Returns true only when the bike segment moves forward across this
    /// route-normal gate and intersects its paved width.
    pub fn crosses_forward_gate(
        &self,
        from_x: f32,
        from_z: f32,
        to_x: f32,
        to_z: f32,
        distance_m: f32,
    ) -> bool {
        let gate = self.point_at(distance_m);
        let forward_x = gate.yaw.sin();
        let forward_z = -gate.yaw.cos();
        let lateral_x = gate.yaw.cos();
        let lateral_z = gate.yaw.sin();
        let signed_distance =
            |x: f32, z: f32| (x - gate.center_x) * forward_x + (z - gate.center_z) * forward_z;
        let from_along = signed_distance(from_x, from_z);
        let to_along = signed_distance(to_x, to_z);
        if from_along > 0.0 || to_along <= 0.0 {
            return false;
        }
        let fraction = -from_along / (to_along - from_along);
        let crossing_x = from_x + (to_x - from_x) * fraction;
        let crossing_z = from_z + (to_z - from_z) * fraction;
        let lateral_offset =
            (crossing_x - gate.center_x) * lateral_x + (crossing_z - gate.center_z) * lateral_z;
        // Without rails the Rider may cross a gate on the lawn beside it.
        lateral_offset.abs() <= PARK_BOUNDARY_M
    }

    fn add_route_markers(&mut self) {
        let section_end = |section: RoadSection, pieces: &[RoadPiece]| {
            pieces
                .iter()
                .filter(|piece| piece.section == section)
                .map(|piece| piece.start_distance_m + piece.length_m)
                .max_by(f32::total_cmp)
        };
        let open_end = section_end(RoadSection::OpenRiverside, &self.pieces)
            .expect("the route includes an open riverside section");
        let bridge_end = section_end(RoadSection::BridgeUnderpass, &self.pieces)
            .expect("the route includes a bridge underpass section");
        self.checkpoints_m = vec![open_end - 8.0, bridge_end - 8.0];
        self.finish_distance_m = (self.length_m - ESCAPE_FINISH_MARGIN_M).max(0.0);
    }

    fn add_bridge_cover(&mut self) {
        let Some(bridge) = self
            .pieces
            .iter()
            .find(|piece| piece.section == RoadSection::BridgeUnderpass)
            .copied()
        else {
            return;
        };
        let bridge_end_m = self
            .pieces
            .iter()
            .find(|piece| {
                piece.start_distance_m > bridge.start_distance_m
                    && piece.section != RoadSection::BridgeUnderpass
            })
            .map_or(self.length_m, |piece| piece.start_distance_m);

        let mut span_start_m = bridge.start_distance_m;
        while span_start_m < bridge_end_m {
            let span_end_m = (span_start_m + BRIDGE_SPAN_LENGTH_M).min(bridge_end_m);
            let center = self.point_at((span_start_m + span_end_m) * 0.5);
            self.cover_volumes.push(CoverVolume {
                kind: CoverKind::BridgeDeck,
                center_x: center.center_x,
                center_z: center.center_z,
                yaw: center.yaw,
                half_width_m: BRIDGE_DECK_HALF_WIDTH_M,
                half_depth_m: (span_end_m - span_start_m) * 0.5,
                min_y: BRIDGE_DECK_BOTTOM_M,
                max_y: BRIDGE_DECK_TOP_M,
            });

            let support = self.point_at(span_start_m);
            let right_x = support.yaw.cos() * BRIDGE_PIER_OFFSET_M;
            let right_z = support.yaw.sin() * BRIDGE_PIER_OFFSET_M;
            for side in [-1.0, 1.0] {
                self.cover_volumes.push(CoverVolume {
                    kind: CoverKind::BridgePier,
                    center_x: support.center_x + right_x * side,
                    center_z: support.center_z + right_z * side,
                    yaw: support.yaw,
                    half_width_m: 3.15,
                    half_depth_m: 0.85,
                    min_y: 0.0,
                    max_y: 5.725,
                });
            }
            span_start_m = span_end_m;
        }
    }

    pub fn section_at(&self, distance_m: f32) -> RoadSection {
        let distance_m = distance_m.clamp(0.0, self.length_m);
        self.pieces
            .iter()
            .find(|piece| distance_m < piece.start_distance_m + piece.length_m)
            .or_else(|| self.pieces.last())
            .map_or(RoadSection::OpenRiverside, |piece| piece.section)
    }

    pub fn curvature_at(&self, distance_m: f32) -> f32 {
        let distance_m = distance_m.clamp(0.0, self.length_m);
        let piece = self
            .pieces
            .iter()
            .find(|piece| distance_m < piece.start_distance_m + piece.length_m)
            .or_else(|| self.pieces.last());
        match piece.map(|piece| piece.kind) {
            Some(RoadPieceKind::CurveRight) => 1.0 / ROAD_CURVE_RADIUS_M,
            Some(RoadPieceKind::CurveLeft) => -1.0 / ROAD_CURVE_RADIUS_M,
            _ => 0.0,
        }
    }

    pub fn point_at(&self, distance_m: f32) -> CourseProjection {
        let target = distance_m.clamp(0.0, self.length_m);
        polyline_point_at(&self.points, target, self.section_at(target))
    }

    pub fn project(&self, x: f32, z: f32) -> CourseProjection {
        project_polyline(&self.points, x, z, |distance| self.section_at(distance))
    }

    /// Projects onto only the stretch of route between two distances, so a
    /// follower near a detour's apex does not snap to the returning leg.
    pub fn project_window(&self, x: f32, z: f32, from_m: f32, to_m: f32) -> CourseProjection {
        let start = self
            .points
            .partition_point(|point| point.distance_m < from_m)
            .saturating_sub(1);
        let end = (self
            .points
            .partition_point(|point| point.distance_m <= to_m)
            + 1)
        .clamp(start + 2, self.points.len());
        project_polyline(&self.points[start..end], x, z, |distance| {
            self.section_at(distance)
        })
    }

    pub fn branches(&self) -> &[CourseBranch] {
        &self.branches
    }

    pub fn ramps(&self) -> &[Ramp] {
        &self.ramps
    }

    /// Creek patches that cut across shortcut branches.
    pub fn water(&self) -> &[GroundPatch] {
        &self.water
    }

    pub fn surface_at(&self, x: f32, z: f32) -> Surface {
        if self.water.iter().any(|patch| patch.contains(x, z)) {
            return Surface::Water;
        }
        // The bike's centre may reach the paved edge before its tyres leave it.
        if self.project(x, z).center_distance_m <= TRACK_HALF_WIDTH
            || self.ramps.iter().any(|ramp| ramp.patch.contains(x, z))
            || self
                .branches
                .iter()
                .any(|branch| branch.project(x, z).center_distance_m <= BRANCH_HALF_WIDTH_M)
        {
            return Surface::Road;
        }
        Surface::Grass
    }

    pub fn ground_height(&self, x: f32, z: f32) -> f32 {
        self.ramps
            .iter()
            .filter_map(|ramp| ramp.height_at(x, z))
            .fold(0.0, f32::max)
    }

    /// Closest centreline point over the main route and every branch.
    pub fn nearest_road_point(&self, x: f32, z: f32) -> NearestRoadPoint {
        std::iter::once(self.project(x, z))
            .chain(self.branches.iter().map(|branch| branch.project(x, z)))
            .map(|projection| NearestRoadPoint {
                x: projection.center_x,
                z: projection.center_z,
                distance_m: projection.center_distance_m,
            })
            .min_by(|a, b| a.distance_m.total_cmp(&b.distance_m))
            .expect("the main route always exists")
    }

    pub fn contains(&self, x: f32, z: f32) -> bool {
        let projection = self.project(x, z);
        projection.lateral_m.abs() <= TRACK_HALF_WIDTH - BIKE_COLLISION_HALF_WIDTH
            && projection.center_distance_m <= TRACK_HALF_WIDTH - BIKE_COLLISION_HALF_WIDTH
            && projection.distance_m <= self.length_m
    }
}

struct CourseBuilder {
    pieces: Vec<RoadPiece>,
    points: Vec<CoursePoint>,
    /// Main-route (start, end) distances of each detour loop.
    detours: Vec<(f32, f32)>,
    x: f32,
    z: f32,
    yaw: f32,
    distance_m: f32,
}

impl CourseBuilder {
    fn new() -> Self {
        Self {
            pieces: Vec::new(),
            detours: Vec::new(),
            points: vec![CoursePoint {
                x: 0.0,
                z: BikeState::default().z,
                yaw: 0.0,
                distance_m: 0.0,
            }],
            x: 0.0,
            z: BikeState::default().z,
            yaw: 0.0,
            distance_m: 0.0,
        }
    }

    fn straights(&mut self, count: usize, section: RoadSection) {
        for _ in 0..count {
            let start_distance_m = self.distance_m;
            self.pieces.push(RoadPiece {
                kind: RoadPieceKind::Straight,
                section,
                root_x: self.x,
                root_z: self.z,
                root_yaw: self.yaw,
                reversed: false,
                start_distance_m,
                length_m: ROAD_STRAIGHT_LENGTH_M,
            });
            for step in 1..=8 {
                self.push_local_point(
                    0.0,
                    -(step as f32),
                    self.yaw,
                    start_distance_m + step as f32,
                );
            }
            self.distance_m += ROAD_STRAIGHT_LENGTH_M;
            self.advance(0.0, -ROAD_STRAIGHT_LENGTH_M);
        }
    }

    /// A symmetric loop out to the right (away from the river) that returns
    /// to the original heading and line: right 60°, out, left 120°, back,
    /// right 60°. A straight shortcut can then cut across it.
    fn detour(&mut self, straights: usize, section: RoadSection) {
        let start_m = self.distance_m;
        for _ in 0..2 {
            self.curve(RoadPieceKind::CurveRight, section);
        }
        self.straights(straights, section);
        for _ in 0..4 {
            self.curve(RoadPieceKind::CurveLeft, section);
        }
        self.straights(straights, section);
        for _ in 0..2 {
            self.curve(RoadPieceKind::CurveRight, section);
        }
        self.detours.push((start_m, self.distance_m));
    }

    fn curve(&mut self, kind: RoadPieceKind, section: RoadSection) {
        let is_left = kind == RoadPieceKind::CurveLeft;
        let direction = if is_left { -1.0 } else { 1.0 };
        let start_distance_m = self.distance_m;
        let (root_x, root_z, root_yaw) = if is_left {
            // Place the authored right-turn GLB so its exit is the route
            // entry, then travel the geometry backwards for a left turn.
            let root_yaw = self.yaw + 5.0 * std::f32::consts::PI / 6.0;
            let (dx, dz) = local_to_world(1.33975, -5.0, root_yaw);
            (self.x - dx, self.z - dz, root_yaw)
        } else {
            (self.x, self.z, self.yaw)
        };
        self.pieces.push(RoadPiece {
            kind,
            section,
            root_x,
            root_z,
            root_yaw,
            reversed: is_left,
            start_distance_m,
            length_m: ROAD_CURVE_LENGTH_M,
        });

        let mut distance_m = 0.5;
        while distance_m < ROAD_CURVE_LENGTH_M {
            let angle = distance_m / ROAD_CURVE_RADIUS_M;
            let local_x = direction * ROAD_CURVE_RADIUS_M * (1.0 - angle.cos());
            let local_z = -ROAD_CURVE_RADIUS_M * angle.sin();
            let (dx, dz) = local_to_world(local_x, local_z, self.yaw);
            self.points.push(CoursePoint {
                x: self.x + dx,
                z: self.z + dz,
                yaw: wrap_course_angle(self.yaw + direction * angle),
                distance_m: start_distance_m + distance_m,
            });
            distance_m += 0.5;
        }
        let angle = ROAD_CURVE_ANGLE_RADIANS;
        let local_x = direction * ROAD_CURVE_RADIUS_M * (1.0 - angle.cos());
        let local_z = -ROAD_CURVE_RADIUS_M * angle.sin();
        self.push_local_point(
            local_x,
            local_z,
            wrap_course_angle(self.yaw + direction * angle),
            start_distance_m + ROAD_CURVE_LENGTH_M,
        );
        self.distance_m += ROAD_CURVE_LENGTH_M;
        self.advance(local_x, local_z);
        self.yaw = wrap_course_angle(self.yaw + direction * angle);
    }

    fn push_local_point(&mut self, local_x: f32, local_z: f32, yaw: f32, distance_m: f32) {
        let (dx, dz) = local_to_world(local_x, local_z, self.yaw);
        self.points.push(CoursePoint {
            x: self.x + dx,
            z: self.z + dz,
            yaw,
            distance_m,
        });
    }

    fn advance(&mut self, local_x: f32, local_z: f32) {
        let (dx, dz) = local_to_world(local_x, local_z, self.yaw);
        self.x += dx;
        self.z += dz;
    }

    fn finish(self, seed: u64) -> RiversideCourse {
        RiversideCourse {
            seed,
            obstacles: Vec::new(),
            branches: Vec::new(),
            ramps: Vec::new(),
            water: Vec::new(),
            pieces: self.pieces,
            points: self.points,
            cover_volumes: Vec::new(),
            checkpoints_m: Vec::new(),
            finish_distance_m: 0.0,
            length_m: self.distance_m,
        }
    }
}

fn polyline_point_at(
    points: &[CoursePoint],
    distance_m: f32,
    section: RoadSection,
) -> CourseProjection {
    let target = distance_m.clamp(0.0, points.last().map_or(0.0, |point| point.distance_m));
    let pair = points
        .windows(2)
        .find(|pair| target <= pair[1].distance_m)
        .unwrap_or_else(|| {
            let end = points.len() - 1;
            &points[end - 1..=end]
        });
    let span = pair[1].distance_m - pair[0].distance_m;
    let alpha = if span > f32::EPSILON {
        ((target - pair[0].distance_m) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    CourseProjection {
        distance_m: target,
        lateral_m: 0.0,
        center_distance_m: 0.0,
        section,
        center_x: pair[0].x + (pair[1].x - pair[0].x) * alpha,
        center_z: pair[0].z + (pair[1].z - pair[0].z) * alpha,
        yaw: approach_course_angle(pair[0].yaw, pair[1].yaw, alpha),
    }
}

fn project_polyline(
    points: &[CoursePoint],
    x: f32,
    z: f32,
    section_at: impl Fn(f32) -> RoadSection,
) -> CourseProjection {
    let mut best = (
        f32::INFINITY,
        CourseProjection {
            distance_m: 0.0,
            lateral_m: 0.0,
            center_distance_m: f32::INFINITY,
            section: RoadSection::OpenRiverside,
            center_x: 0.0,
            center_z: 0.0,
            yaw: 0.0,
        },
    );
    for pair in points.windows(2) {
        let dx = pair[1].x - pair[0].x;
        let dz = pair[1].z - pair[0].z;
        let length_squared = dx * dx + dz * dz;
        let alpha = if length_squared > f32::EPSILON {
            (((x - pair[0].x) * dx + (z - pair[0].z) * dz) / length_squared).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let center_x = pair[0].x + dx * alpha;
        let center_z = pair[0].z + dz * alpha;
        let offset_x = x - center_x;
        let offset_z = z - center_z;
        let distance_squared = offset_x * offset_x + offset_z * offset_z;
        if distance_squared >= best.0 {
            continue;
        }
        let yaw = approach_course_angle(pair[0].yaw, pair[1].yaw, alpha);
        let distance_m = pair[0].distance_m + (pair[1].distance_m - pair[0].distance_m) * alpha;
        best = (
            distance_squared,
            CourseProjection {
                distance_m,
                lateral_m: offset_x * yaw.cos() + offset_z * yaw.sin(),
                center_distance_m: distance_squared.sqrt(),
                section: RoadSection::OpenRiverside,
                center_x,
                center_z,
                yaw,
            },
        );
    }
    let mut projection = best.1;
    projection.section = section_at(projection.distance_m);
    projection
}

fn local_to_world(local_x: f32, local_z: f32, yaw: f32) -> (f32, f32) {
    (
        yaw.cos() * local_x - yaw.sin() * local_z,
        yaw.sin() * local_x + yaw.cos() * local_z,
    )
}

fn alternating_turn(index: usize) -> RoadPieceKind {
    if index.is_multiple_of(2) {
        RoadPieceKind::CurveRight
    } else {
        RoadPieceKind::CurveLeft
    }
}

fn opposite_turn(kind: RoadPieceKind) -> RoadPieceKind {
    match kind {
        RoadPieceKind::CurveRight => RoadPieceKind::CurveLeft,
        RoadPieceKind::CurveLeft => RoadPieceKind::CurveRight,
        RoadPieceKind::Straight => RoadPieceKind::Straight,
    }
}

fn wrap_course_angle(angle: f32) -> f32 {
    angle.rem_euclid(std::f32::consts::TAU)
}

fn approach_course_angle(from: f32, to: f32, alpha: f32) -> f32 {
    let delta =
        (to - from + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    wrap_course_angle(from + delta * alpha)
}

fn segment_cover_interval(
    origin: WorldPosition,
    target: WorldPosition,
    cover: CoverVolume,
) -> Option<(f32, f32)> {
    let to_local = |position: WorldPosition| {
        let dx = position.x - cover.center_x;
        let dz = position.z - cover.center_z;
        [
            cover.yaw.cos() * dx + cover.yaw.sin() * dz,
            position.y,
            -cover.yaw.sin() * dx + cover.yaw.cos() * dz,
        ]
    };
    let origin = to_local(origin);
    let target = to_local(target);
    let min = [-cover.half_width_m, cover.min_y, -cover.half_depth_m];
    let max = [cover.half_width_m, cover.max_y, cover.half_depth_m];
    let mut entry: f32 = 0.0;
    let mut exit: f32 = 1.0;
    for axis in 0..3 {
        let delta = target[axis] - origin[axis];
        if delta.abs() <= f32::EPSILON {
            if origin[axis] < min[axis] || origin[axis] > max[axis] {
                return None;
            }
            continue;
        }
        let a = (min[axis] - origin[axis]) / delta;
        let b = (max[axis] - origin[axis]) / delta;
        entry = entry.max(a.min(b));
        exit = exit.min(a.max(b));
        if entry > exit {
            return None;
        }
    }
    (exit >= 0.0 && entry <= 1.0).then_some((entry.clamp(0.0, 1.0), exit.clamp(0.0, 1.0)))
}

#[cfg(test)]
mod course_tests {
    use super::{
        AUTHORED_COURSE_SEED, BIKE_COLLISION_HALF_WIDTH, BRIDGE_DECK_BOTTOM_M, BRIDGE_DECK_TOP_M,
        BikeState, CoverKind, FIXED_DT, MAX_SPEED, OBSTACLES, PARK_BOUNDARY_M, ROAD_CURVE_LENGTH_M,
        ROAD_STRAIGHT_LENGTH_M, RiderInput, RiversideCourse, RoadPieceKind, RoadSection, Surface,
        TRACK_HALF_WIDTH, WorldPosition, course_for_seed, first_cover_intersection,
        line_of_sight_blocked, riverside_course,
    };

    fn angle_delta(from: f32, to: f32) -> f32 {
        (to - from + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
    }

    #[test]
    fn course_connects_three_ordered_sections_and_is_about_1_2_km() {
        let course = riverside_course();
        assert!(
            (1_190.0..=1_210.0).contains(&course.length_m()),
            "course length {:.2} m",
            course.length_m()
        );
        assert_eq!(course.section_at(0.0), RoadSection::OpenRiverside);
        let bridge_start = course
            .pieces()
            .iter()
            .find(|piece| piece.section == RoadSection::BridgeUnderpass)
            .unwrap()
            .start_distance_m;
        let bends_start = course
            .pieces()
            .iter()
            .find(|piece| piece.section == RoadSection::NarrowBends)
            .unwrap()
            .start_distance_m;
        assert_eq!(
            course.section_at(bridge_start),
            RoadSection::BridgeUnderpass
        );
        assert_eq!(course.section_at(bends_start), RoadSection::NarrowBends);
        assert!(bridge_start > 390.0 && bridge_start < 410.0);
        assert!(bends_start > 790.0 && bends_start < 810.0);
        assert_eq!(course.checkpoints_m().len(), 2);
        assert!(course.checkpoints_m()[0] < course.checkpoints_m()[1]);
        assert!((course.checkpoints_m()[0] - (bridge_start - 8.0)).abs() < 0.01);
        assert!((course.checkpoints_m()[1] - (bends_start - 8.0)).abs() < 0.01);
        assert!((course.finish_distance_m() - (course.length_m() - 1.0)).abs() < 0.01);

        let straight_count = course
            .pieces()
            .iter()
            .filter(|piece| piece.kind == RoadPieceKind::Straight)
            .count();
        let curve_count = course.pieces().len() - straight_count;
        assert_eq!(straight_count, 137);
        assert_eq!(curve_count, 20);
        assert!(
            (course.length_m() - (137.0 * ROAD_STRAIGHT_LENGTH_M + 20.0 * ROAD_CURVE_LENGTH_M))
                .abs()
                < 0.01
        );
    }

    #[test]
    fn route_gates_require_forward_crossing_inside_the_road_width() {
        let course = riverside_course();
        let gate_distance = course.checkpoints_m()[0];
        let gate = course.point_at(gate_distance);
        let forward = (gate.yaw.sin(), -gate.yaw.cos());
        let lateral = (gate.yaw.cos(), gate.yaw.sin());
        let position = |along: f32, across: f32| {
            (
                gate.center_x + forward.0 * along + lateral.0 * across,
                gate.center_z + forward.1 * along + lateral.1 * across,
            )
        };
        let before = position(-0.5, 0.0);
        let after = position(0.5, 0.0);
        assert!(course.crosses_forward_gate(before.0, before.1, after.0, after.1, gate_distance));
        assert!(!course.crosses_forward_gate(after.0, after.1, before.0, before.1, gate_distance));
        // Rail-free roads let the Rider cross on the lawn beside the gate,
        // but not from beyond the park boundary.
        let lawn_before = position(-0.5, 6.0);
        let lawn_after = position(0.5, 6.0);
        assert!(course.crosses_forward_gate(
            lawn_before.0,
            lawn_before.1,
            lawn_after.0,
            lawn_after.1,
            gate_distance
        ));
        let outside_before = position(-0.5, PARK_BOUNDARY_M + 1.0);
        let outside_after = position(0.5, PARK_BOUNDARY_M + 1.0);
        assert!(!course.crosses_forward_gate(
            outside_before.0,
            outside_before.1,
            outside_after.0,
            outside_after.1,
            gate_distance
        ));
        let same_side = position(-0.25, 0.0);
        assert!(!course.crosses_forward_gate(
            before.0,
            before.1,
            same_side.0,
            same_side.1,
            gate_distance
        ));
    }

    #[test]
    fn every_authored_piece_socket_matches_its_route_entry_and_exit() {
        let course = riverside_course();
        for piece in course.pieces() {
            let entry = course.point_at(piece.start_distance_m);
            let exit = course.point_at(piece.start_distance_m + piece.length_m);
            let angle = -piece.root_yaw;
            let asset_exit_x = piece.root_x + angle.cos() * 1.33975 + angle.sin() * -5.0;
            let asset_exit_z = piece.root_z - angle.sin() * 1.33975 + angle.cos() * -5.0;
            if piece.kind == RoadPieceKind::CurveLeft {
                assert!((asset_exit_x - entry.center_x).abs() < 0.002);
                assert!((asset_exit_z - entry.center_z).abs() < 0.002);
                assert!((piece.root_x - exit.center_x).abs() < 0.002);
                assert!((piece.root_z - exit.center_z).abs() < 0.002);
            } else if piece.kind == RoadPieceKind::CurveRight {
                assert!((piece.root_x - entry.center_x).abs() < 0.002);
                assert!((piece.root_z - entry.center_z).abs() < 0.002);
                assert!((asset_exit_x - exit.center_x).abs() < 0.002);
                assert!((asset_exit_z - exit.center_z).abs() < 0.002);
            } else {
                assert!((piece.root_x - entry.center_x).abs() < 0.002);
                assert!((piece.root_z - entry.center_z).abs() < 0.002);
                assert!(angle_delta(piece.root_yaw, entry.yaw).abs() < 0.002);
                assert!(
                    (exit.center_x - piece.root_x - entry.yaw.sin() * piece.length_m).abs() < 0.002
                );
                assert!(
                    (exit.center_z - piece.root_z + entry.yaw.cos() * piece.length_m).abs() < 0.002
                );
            }
            assert!(angle_delta(entry.yaw, exit.yaw).abs() < 0.53);
        }
    }

    #[test]
    fn road_collision_follows_curves_and_rejects_both_sides_of_road() {
        let course = riverside_course();
        let curve = course
            .pieces()
            .iter()
            .find(|piece| piece.section == RoadSection::NarrowBends)
            .unwrap();
        let center = course.point_at(curve.start_distance_m + ROAD_CURVE_LENGTH_M * 0.5);
        let right_x = center.center_x
            + center.yaw.cos() * (TRACK_HALF_WIDTH - BIKE_COLLISION_HALF_WIDTH - 0.1);
        let right_z = center.center_z
            + center.yaw.sin() * (TRACK_HALF_WIDTH - BIKE_COLLISION_HALF_WIDTH - 0.1);
        let outside_x = center.center_x + center.yaw.cos() * (TRACK_HALF_WIDTH + 0.1);
        let outside_z = center.center_z + center.yaw.sin() * (TRACK_HALF_WIDTH + 0.1);
        assert!(course.contains(center.center_x, center.center_z));
        assert!(course.contains(right_x, right_z));
        assert!(!course.contains(outside_x, outside_z));
        assert!(!course.contains(
            center.center_x - center.yaw.cos() * (TRACK_HALF_WIDTH + 0.1),
            center.center_z - center.yaw.sin() * (TRACK_HALF_WIDTH + 0.1)
        ));
    }

    #[test]
    fn bridge_deck_blocks_direct_sight_and_projectile_paths_but_not_the_road_lane() {
        let course = riverside_course();
        let deck = course
            .cover_volumes()
            .iter()
            .find(|cover| cover.kind == CoverKind::BridgeDeck)
            .unwrap();
        let under_roof = WorldPosition {
            x: deck.center_x,
            y: 1.84,
            z: deck.center_z,
        };
        let helicopter = WorldPosition {
            y: 8.0,
            ..under_roof
        };
        assert!(line_of_sight_blocked(
            riverside_course(),
            under_roof,
            helicopter
        ));
        let hit = first_cover_intersection(riverside_course(), under_roof, helicopter).unwrap();
        let hit_y = under_roof.y + (helicopter.y - under_roof.y) * hit;
        assert!((hit_y - BRIDGE_DECK_BOTTOM_M).abs() < 0.01);

        let bike_lane = WorldPosition {
            y: BRIDGE_DECK_BOTTOM_M - 0.2,
            ..under_roof
        };
        assert!(!line_of_sight_blocked(
            riverside_course(),
            bike_lane,
            WorldPosition {
                y: BRIDGE_DECK_BOTTOM_M - 0.1,
                z: deck.center_z + 8.0,
                ..bike_lane
            }
        ));
        let above_deck = WorldPosition {
            y: BRIDGE_DECK_TOP_M + 1.0,
            ..under_roof
        };
        assert_eq!(
            first_cover_intersection(
                riverside_course(),
                above_deck,
                WorldPosition {
                    y: BRIDGE_DECK_TOP_M + 2.0,
                    ..above_deck
                }
            ),
            None
        );
        assert_eq!(
            course
                .cover_volumes()
                .iter()
                .filter(|cover| cover.kind == CoverKind::BridgeDeck)
                .count(),
            13
        );
        assert_eq!(
            course
                .cover_volumes()
                .iter()
                .filter(|cover| cover.kind == CoverKind::BridgePier)
                .count(),
            26
        );
    }

    struct RouteDrive {
        reached: bool,
        highest_distance_m: f32,
        elapsed_ticks: u32,
        crashes: u32,
        final_speed: f32,
    }

    /// A simple autopilot: follow the centreline and swing to the far side
    /// of the next obstacle ahead, asserting the bike never leaves the road.
    fn drive_route(course: &RiversideCourse) -> RouteDrive {
        let mut bike = BikeState::default();
        let mut drive = RouteDrive {
            reached: false,
            highest_distance_m: 0.0,
            elapsed_ticks: 0,
            crashes: 0,
            final_speed: 0.0,
        };
        let obstacles: Vec<(f32, f32)> = course
            .obstacles()
            .iter()
            .map(|obstacle| {
                let projection = course.project(obstacle.x, obstacle.z);
                (projection.distance_m, projection.lateral_m)
            })
            .collect();
        for _ in 0..60 * 240 {
            drive.elapsed_ticks += 1;
            let projection = course.project_window(
                bike.x,
                bike.z,
                drive.highest_distance_m - 5.0,
                drive.highest_distance_m + 20.0,
            );
            drive.highest_distance_m = drive.highest_distance_m.max(projection.distance_m);
            if projection.distance_m >= course.length_m() - 3.0 {
                drive.reached = true;
                break;
            }
            let detour = obstacles
                .iter()
                .filter(|(distance, _)| {
                    (projection.distance_m - 2.0..projection.distance_m + 14.0).contains(distance)
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map_or(0.0, |(_, lateral)| -lateral.signum() * 1.5);
            let lookahead = course.point_at((projection.distance_m + 4.0).min(course.length_m()));
            let target_x = lookahead.center_x + lookahead.yaw.cos() * detour;
            let target_z = lookahead.center_z + lookahead.yaw.sin() * detour;
            let desired_yaw = (target_x - bike.x).atan2(bike.z - target_z);
            let heading_error = angle_delta(bike.yaw, desired_yaw);
            let desired_curvature = course.curvature_at(projection.distance_m + 2.0);
            let steer = ((desired_curvature + heading_error * 0.65) * bike.speed.max(1.0)
                / crate::BIKE_YAW_RATE)
                .clamp(-1.0, 1.0);
            let speed_before = bike.speed;
            bike.step(
                RiderInput {
                    pedal_held: true,
                    steer,
                    ..RiderInput::default()
                },
                FIXED_DT,
                course,
            );
            if bike.speed < speed_before * 0.5 {
                drive.crashes += 1;
            }
            let after = course.project_window(
                bike.x,
                bike.z,
                drive.highest_distance_m - 5.0,
                drive.highest_distance_m + 20.0,
            );
            drive.highest_distance_m = drive.highest_distance_m.max(after.distance_m);
            assert!(
                course.surface_at(bike.x, bike.z) == Surface::Road,
                "seed {} bike left road at {:.1} m ({:.2}, {:.2})",
                course.seed(),
                after.distance_m,
                bike.x,
                bike.z
            );
        }
        drive.final_speed = bike.speed;
        drive
    }

    #[test]
    fn fixed_step_rider_can_follow_the_full_route_and_avoid_the_early_barrier() {
        let course = riverside_course();
        let drive = drive_route(course);
        assert!(
            drive.reached,
            "rider only reached {:.1} / {:.1} m at {:.1} m/s (max speed {:.1})",
            drive.highest_distance_m,
            course.length_m(),
            drive.final_speed,
            MAX_SPEED
        );
        eprintln!(
            "controlled Rider traversed {:.1} m in {:.1} s ({} fixed ticks)",
            drive.highest_distance_m,
            drive.elapsed_ticks as f32 * FIXED_DT,
            drive.elapsed_ticks
        );
    }

    #[test]
    fn course_seed_reproduces_the_same_layout_and_varies_between_seeds() {
        let first = RiversideCourse::build(42);
        let again = RiversideCourse::build(42);
        assert_eq!(first.pieces(), again.pieces());
        assert_eq!(first.obstacles(), again.obstacles());
        assert_eq!(course_for_seed(42).obstacles(), first.obstacles());

        let layouts: std::collections::BTreeSet<_> = (1..=16)
            .map(|seed| {
                let course = RiversideCourse::build(seed);
                (
                    course.pieces().len(),
                    course.obstacles().len(),
                    (course.length_m() * 10.0) as i64,
                )
            })
            .collect();
        assert!(
            layouts.len() > 8,
            "seeds produced only {} layouts",
            layouts.len()
        );

        let authored = course_for_seed(AUTHORED_COURSE_SEED);
        assert_eq!(authored.obstacles(), &OBSTACLES[..]);
        assert_eq!(authored.length_m(), riverside_course().length_m());
    }

    #[test]
    fn random_obstacles_always_leave_a_clear_lane_and_keep_gates_free() {
        let clear_lane_m = 4.0 * BIKE_COLLISION_HALF_WIDTH;
        for seed in 1..=400 {
            let course = RiversideCourse::build(seed);
            assert!(
                !course.obstacles().is_empty(),
                "seed {seed} has no obstacles"
            );
            assert_eq!(course.checkpoints_m().len(), 2);
            let mut previous_distance = f32::NEG_INFINITY;
            let mut placed: Vec<_> = course
                .obstacles()
                .iter()
                .map(|obstacle| (course.project(obstacle.x, obstacle.z), *obstacle))
                .collect();
            placed.sort_by(|a, b| a.0.distance_m.total_cmp(&b.0.distance_m));
            for (projection, obstacle) in placed {
                assert!(
                    projection.distance_m >= 24.0,
                    "seed {seed} obstacle at start"
                );
                for gate in course
                    .checkpoints_m()
                    .iter()
                    .chain(std::iter::once(&course.finish_distance_m()))
                {
                    assert!((gate - projection.distance_m).abs() >= 8.0);
                }
                assert!(projection.distance_m - previous_distance >= ROAD_STRAIGHT_LENGTH_M);
                previous_distance = projection.distance_m;
                let free_left = projection.lateral_m - obstacle.half_width + TRACK_HALF_WIDTH;
                let free_right = TRACK_HALF_WIDTH - (projection.lateral_m + obstacle.half_width);
                assert!(
                    free_left.max(free_right) >= clear_lane_m,
                    "seed {seed} obstacle at {:.1} m blocks the road",
                    projection.distance_m
                );
            }
        }
    }

    #[test]
    fn autopilot_completes_many_random_courses() {
        for seed in 1..=10 {
            let course = RiversideCourse::build(seed);
            let drive = drive_route(&course);
            assert!(
                drive.reached,
                "seed {seed}: reached {:.1} / {:.1} m",
                drive.highest_distance_m,
                course.length_m()
            );
            assert_eq!(drive.crashes, 0, "seed {seed} crashed into an obstacle");
        }
    }
}

fn move_toward(current: f32, target: f32, max_delta: f32) -> f32 {
    let delta = target - current;
    if delta.abs() <= max_delta {
        target
    } else {
        current + delta.signum() * max_delta
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PredictedFrame {
    pub tick: u64,
    pub input: RiderInput,
    pub bike: BikeState,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReconcileResult {
    pub same_tick_position_error_m: Option<f32>,
    pub same_tick_yaw_error_deg: Option<f32>,
    pub replayed_ticks: u32,
    pub replayed_unacknowledged_inputs: u32,
    pub history_complete: bool,
}

/// Render-only offset retained across an authoritative restore, then decayed
/// according to elapsed wall time so correction speed does not depend on FPS.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BikeRenderOffset {
    pub x: f32,
    pub z: f32,
    pub yaw: f32,
    pub speed: f32,
}

impl BikeRenderOffset {
    pub fn between(visible: BikeState, predicted: BikeState) -> Self {
        Self {
            x: visible.x - predicted.x,
            z: visible.z - predicted.z,
            yaw: shortest_angle_delta(predicted.yaw, visible.yaw),
            speed: visible.speed - predicted.speed,
        }
    }

    pub fn advance(&mut self, delta_secs: f32) -> Self {
        let previous = *self;
        let decay = (-delta_secs.max(0.0) / BIKE_RENDER_CORRECTION_TAU_SECS).exp();
        self.x *= decay;
        self.z *= decay;
        self.yaw *= decay;
        self.speed *= decay;
        previous
    }

    pub fn apply(self, mut bike: BikeState) -> BikeState {
        bike.x += self.x;
        bike.z += self.z;
        bike.yaw = (bike.yaw + self.yaw).rem_euclid(std::f32::consts::TAU);
        bike.speed += self.speed;
        bike
    }
}

/// Server-tick indexed Rider prediction history for rollback and replay.
#[derive(Clone, Debug)]
pub struct RiderPrediction {
    current_tick: u64,
    bike: BikeState,
    history: VecDeque<PredictedFrame>,
    course: Arc<RiversideCourse>,
}

impl RiderPrediction {
    pub fn new(tick: u64, bike: BikeState) -> Self {
        let mut prediction = Self {
            current_tick: tick,
            bike,
            history: VecDeque::new(),
            course: course_for_seed(AUTHORED_COURSE_SEED),
        };
        prediction.push_frame(tick, RiderInput::default());
        prediction
    }

    pub fn reset(&mut self, tick: u64, bike: BikeState) {
        self.current_tick = tick;
        self.bike = bike;
        self.history.clear();
        self.push_frame(tick, RiderInput::default());
    }

    /// Predicts against the room's course; call before resetting to a new round.
    pub fn set_course(&mut self, course: Arc<RiversideCourse>) {
        self.course = course;
    }

    pub fn course(&self) -> &Arc<RiversideCourse> {
        &self.course
    }

    pub fn current_tick(&self) -> u64 {
        self.current_tick
    }

    pub fn bike(&self) -> BikeState {
        self.bike
    }

    pub fn state_at(&self, tick: u64) -> Option<BikeState> {
        self.history
            .iter()
            .find(|frame| frame.tick == tick)
            .map(|frame| frame.bike)
    }

    pub fn step(&mut self, input: RiderInput) -> bool {
        let collision = self.bike.step(input, FIXED_DT, &self.course);
        self.current_tick = self.current_tick.saturating_add(1);
        self.push_frame(self.current_tick, input);
        collision
    }

    pub fn reconcile(
        &mut self,
        tick: u64,
        authoritative_bike: BikeState,
        acknowledged_sequence: u32,
        pending_inputs: &VecDeque<RiderInputSample>,
    ) -> ReconcileResult {
        let predicted_at_tick = self.state_at(tick);
        let same_tick_position_error_m = predicted_at_tick.map(|predicted| {
            (predicted.x - authoritative_bike.x).hypot(predicted.z - authoritative_bike.z)
        });
        let same_tick_yaw_error_deg = predicted_at_tick.map(|predicted| {
            shortest_angle_delta(predicted.yaw, authoritative_bike.yaw)
                .abs()
                .to_degrees()
        });
        let previous_tick = self.current_tick;
        let history_complete = tick >= previous_tick || predicted_at_tick.is_some();
        let mut replay_frames = if history_complete {
            self.history
                .iter()
                .filter(|frame| frame.tick > tick)
                .map(|frame| (frame.tick, frame.input))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let mut replayed_unacknowledged_inputs = 0_u32;
        for sample in pending_inputs
            .iter()
            .filter(|sample| sample.sequence > acknowledged_sequence)
            .filter(|sample| sample.tick > tick)
        {
            if let Some((_, input)) = replay_frames
                .iter_mut()
                .find(|(frame_tick, _)| *frame_tick == sample.tick)
            {
                *input = sample.input;
                replayed_unacknowledged_inputs = replayed_unacknowledged_inputs.saturating_add(1);
            }
        }

        self.reset(tick, authoritative_bike);
        for (frame_tick, input) in replay_frames.iter().copied() {
            if frame_tick != self.current_tick.saturating_add(1) {
                break;
            }
            self.step(input);
        }

        ReconcileResult {
            same_tick_position_error_m,
            same_tick_yaw_error_deg,
            replayed_ticks: replay_frames.len().min(u32::MAX as usize) as u32,
            replayed_unacknowledged_inputs,
            history_complete,
        }
    }

    fn push_frame(&mut self, tick: u64, input: RiderInput) {
        self.history.push_back(PredictedFrame {
            tick,
            input,
            bike: self.bike,
        });
        while self.history.len() > PREDICTION_HISTORY_TICKS + 1 {
            self.history.pop_front();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapshotFrame {
    pub tick: u64,
    pub bike: BikeState,
}

/// Time-based snapshot interpolation for remote bicycles, sampled one snapshot
/// interval behind the latest estimated server tick.
#[derive(Clone, Debug, Default)]
pub struct SnapshotInterpolator {
    frames: VecDeque<SnapshotFrame>,
    elapsed_since_latest_secs: f64,
}

impl SnapshotInterpolator {
    pub fn clear(&mut self) {
        self.frames.clear();
        self.elapsed_since_latest_secs = 0.0;
    }

    pub fn push(&mut self, tick: u64, bike: BikeState) -> bool {
        if self.frames.back().is_some_and(|frame| tick <= frame.tick) {
            return false;
        }
        self.frames.push_back(SnapshotFrame { tick, bike });
        self.elapsed_since_latest_secs = 0.0;
        while self.frames.len() > SNAPSHOT_HZ as usize * 3 {
            self.frames.pop_front();
        }
        true
    }

    pub fn advance_time(&mut self, delta_secs: f64) {
        self.elapsed_since_latest_secs += delta_secs.max(0.0);
    }

    pub fn render_tick(&self) -> Option<f64> {
        self.frames.back().map(|latest| {
            latest.tick as f64 + self.elapsed_since_latest_secs * SIMULATION_HZ as f64
                - INTERPOLATION_DELAY_TICKS as f64
        })
    }

    pub fn estimated_lag_secs(&self) -> Option<f64> {
        self.render_tick().map(|render_tick| {
            ((self.frames.back().expect("render tick has a frame").tick as f64
                + self.elapsed_since_latest_secs * SIMULATION_HZ as f64)
                - render_tick)
                / SIMULATION_HZ as f64
        })
    }

    pub fn sample(&self) -> Option<BikeState> {
        let target_tick = self.render_tick()?;
        let first = self.frames.front()?;
        if target_tick <= first.tick as f64 {
            return Some(first.bike);
        }
        for (a, b) in self.frames.iter().zip(self.frames.iter().skip(1)) {
            if target_tick <= b.tick as f64 {
                return Some(interpolate_bike(
                    a.bike,
                    b.bike,
                    ((target_tick - a.tick as f64) / (b.tick - a.tick) as f64).clamp(0.0, 1.0)
                        as f32,
                ));
            }
        }
        let frames = self.frames.iter().collect::<Vec<_>>();
        if frames.len() >= 2 {
            let previous = frames[frames.len() - 2];
            let latest = frames[frames.len() - 1];
            let interval = (latest.tick - previous.tick) as f32;
            let extrapolation =
                (target_tick - latest.tick as f64).clamp(0.0, MAX_EXTRAPOLATION_TICKS) as f32;
            if interval > 0.0 {
                let delta = extrapolation / interval;
                let mut bike = latest.bike;
                bike.x += (latest.bike.x - previous.bike.x) * delta;
                bike.z += (latest.bike.z - previous.bike.z) * delta;
                bike.yaw = (bike.yaw
                    + shortest_angle_delta(previous.bike.yaw, latest.bike.yaw) * delta)
                    .rem_euclid(std::f32::consts::TAU);
                bike.speed += (latest.bike.speed - previous.bike.speed) * delta;
                bike.steering_angle +=
                    (latest.bike.steering_angle - previous.bike.steering_angle) * delta;
                return Some(bike);
            }
        }
        self.frames.back().map(|frame| frame.bike)
    }
}

fn interpolate_bike(a: BikeState, b: BikeState, alpha: f32) -> BikeState {
    BikeState {
        x: a.x + (b.x - a.x) * alpha,
        z: a.z + (b.z - a.z) * alpha,
        yaw: (a.yaw + shortest_angle_delta(a.yaw, b.yaw) * alpha).rem_euclid(std::f32::consts::TAU),
        speed: a.speed + (b.speed - a.speed) * alpha,
        steering_angle: a.steering_angle + (b.steering_angle - a.steering_angle) * alpha,
        stopped_brake_secs: b.stopped_brake_secs,
        y: a.y + (b.y - a.y) * alpha,
        vy: a.vy + (b.vy - a.vy) * alpha,
        airborne: b.airborne,
    }
}

fn shortest_angle_delta(current: f32, target: f32) -> f32 {
    (target - current + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
        - std::f32::consts::PI
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ObstacleKind {
    Cone,
    Bollard,
    Barrier,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obstacle {
    pub kind: ObstacleKind,
    pub x: f32,
    pub z: f32,
    /// Half extent across the road (the obstacle's local X axis).
    pub half_width: f32,
    /// Half extent along the road (the obstacle's local Z axis).
    pub half_depth: f32,
    /// Same convention as route yaw: the local frame follows the road heading.
    pub yaw: f32,
}

impl Obstacle {
    const fn new(kind: ObstacleKind, x: f32, z: f32, yaw: f32) -> Self {
        let (half_width, half_depth) = match kind {
            ObstacleKind::Cone => (0.28, 0.28),
            ObstacleKind::Bollard => (0.35, 0.35),
            ObstacleKind::Barrier => (0.72, 0.3),
        };
        Self {
            kind,
            x,
            z,
            half_width,
            half_depth,
            yaw,
        }
    }

    /// World offset from the obstacle centre expressed in its local frame.
    pub fn to_local(self, x: f32, z: f32) -> (f32, f32) {
        let (dx, dz) = (x - self.x, z - self.z);
        let (sin, cos) = self.yaw.sin_cos();
        (dx * cos + dz * sin, -dx * sin + dz * cos)
    }

    pub fn to_world(self, local_x: f32, local_z: f32) -> (f32, f32) {
        let (dx, dz) = local_to_world(local_x, local_z, self.yaw);
        (self.x + dx, self.z + dz)
    }

    /// A bike higher than this clears the obstacle in the air.
    pub fn height(self) -> f32 {
        match self.kind {
            ObstacleKind::Cone => 0.7,
            ObstacleKind::Bollard => 1.0,
            ObstacleKind::Barrier => 1.2,
        }
    }

    pub fn crash_speed_kept(self) -> f32 {
        match self.kind {
            ObstacleKind::Barrier => BIKE_CRASH_SPEED_KEPT,
            ObstacleKind::Cone | ObstacleKind::Bollard => BIKE_SMALL_CRASH_SPEED_KEPT,
        }
    }

    pub fn blocks(self, x: f32, z: f32) -> bool {
        let (local_x, local_z) = self.to_local(x, z);
        local_x.abs() <= self.half_width + 0.28 && local_z.abs() <= self.half_depth + 0.45
    }
}

/// The hand-authored obstacles of the baseline course (`AUTHORED_COURSE_SEED`).
pub const OBSTACLES: [Obstacle; 5] = [
    Obstacle::new(ObstacleKind::Cone, -1.15, -11.0, 0.0),
    Obstacle::new(ObstacleKind::Bollard, 1.35, -24.0, 0.0),
    Obstacle::new(ObstacleKind::Barrier, -0.55, -38.0, 0.0),
    Obstacle::new(ObstacleKind::Cone, 1.1, -50.0, 0.0),
    Obstacle::new(ObstacleKind::Bollard, -1.45, -65.0, 0.0),
];

#[cfg(test)]
mod handling_tests {
    use super::{
        BIKE_CRASH_SPEED, BikeState, FIXED_DT, GRASS_MAX_SPEED_RATIO, MAX_SPEED, OBSTACLES,
        Obstacle, PARK_BOUNDARY_M, RiderInput, RiversideCourse, Surface, TRACK_HALF_WIDTH,
        WATER_MAX_SPEED_RATIO, course_for_seed, riverside_course,
    };

    fn pedal(steer: f32) -> RiderInput {
        RiderInput {
            pedal_held: true,
            steer,
            ..RiderInput::default()
        }
    }

    /// Starts the bike just behind an authored obstacle, aimed straight at it.
    fn bike_facing(obstacle: Obstacle, speed: f32) -> BikeState {
        BikeState {
            x: obstacle.x,
            z: obstacle.z + obstacle.half_depth + 1.0,
            speed,
            ..BikeState::default()
        }
    }

    fn bike_facing_cone(speed: f32) -> BikeState {
        bike_facing(OBSTACLES[0], speed)
    }

    #[test]
    fn pedal_launch_reaches_most_of_top_speed_within_two_seconds() {
        let mut bike = BikeState::default();
        for _ in 0..120 {
            bike.step(pedal(0.0), FIXED_DT, riverside_course());
        }
        assert!(
            bike.speed > MAX_SPEED * 0.75,
            "only reached {:.2} m/s",
            bike.speed
        );
    }

    #[test]
    fn slow_head_on_obstacle_contact_can_steer_free() {
        let cone = OBSTACLES[1];
        let mut bike = bike_facing(cone, 3.0);
        for _ in 0..60 {
            bike.step(pedal(0.0), FIXED_DT, riverside_course());
        }
        assert!(bike.z > cone.z, "bike passed through the bollard");
        // Steer left off the bollard, straighten up, then carry on past it.
        for _ in 0..25 {
            bike.step(pedal(-1.0), FIXED_DT, riverside_course());
        }
        for _ in 0..25 {
            bike.step(pedal(1.0), FIXED_DT, riverside_course());
        }
        for _ in 0..30 {
            bike.step(pedal(0.0), FIXED_DT, riverside_course());
        }
        assert!(
            bike.z < cone.z - 1.0 && bike.speed > 3.0,
            "bike stayed stuck at ({:.2}, {:.2}) speed={:.2}",
            bike.x,
            bike.z,
            bike.speed
        );
    }

    #[test]
    fn glancing_obstacle_contact_slides_and_keeps_most_speed() {
        let cone = OBSTACLES[1];
        let mut bike = BikeState {
            // Clips the cone's corner by a few centimetres.
            x: cone.x - cone.half_width - 0.22,
            z: cone.z + 2.0,
            speed: 8.0,
            ..BikeState::default()
        };
        for _ in 0..30 {
            bike.step(pedal(0.0), FIXED_DT, riverside_course());
        }
        assert!(bike.z < cone.z - 1.0, "bike did not slide past the cone");
        assert!(
            bike.speed > 6.0,
            "glancing contact kept only {:.2} m/s",
            bike.speed
        );
    }

    #[test]
    fn fast_head_on_obstacle_hit_is_a_crash() {
        let impact_speed = BIKE_CRASH_SPEED + 1.0;
        let mut bike = bike_facing(OBSTACLES[1], impact_speed);
        let crashed = (0..30).any(|_| bike.step(pedal(0.0), FIXED_DT, riverside_course()));
        assert!(crashed);
        // A small bollard costs speed but never stops the bike dead.
        assert!(bike.speed < impact_speed * 0.7 && bike.speed > 0.0);
    }

    #[test]
    fn riding_onto_the_lawn_is_allowed_but_slower() {
        let course = riverside_course();
        let start = course.point_at(4.0);
        // Turn off the road onto the lawn at full speed.
        let mut bike = BikeState {
            x: start.center_x,
            z: start.center_z,
            yaw: start.yaw + 1.2,
            speed: MAX_SPEED,
            ..BikeState::default()
        };
        for _ in 0..120 {
            bike.step(pedal(0.0), FIXED_DT, course);
        }
        assert_eq!(course.surface_at(bike.x, bike.z), Surface::Grass);
        assert!(course.project(bike.x, bike.z).center_distance_m > TRACK_HALF_WIDTH + 1.0);
        assert!(
            bike.speed <= MAX_SPEED * GRASS_MAX_SPEED_RATIO + 0.01,
            "grass kept {:.2} m/s",
            bike.speed
        );
    }

    #[test]
    fn park_boundary_stops_the_bike_wandering_off() {
        let course = riverside_course();
        let start = course.point_at(30.0);
        let mut bike = BikeState {
            x: start.center_x,
            z: start.center_z,
            yaw: start.yaw + std::f32::consts::FRAC_PI_2,
            ..BikeState::default()
        };
        for _ in 0..60 * 12 {
            bike.step(pedal(0.0), FIXED_DT, course);
            assert!(course.nearest_road_point(bike.x, bike.z).distance_m <= PARK_BOUNDARY_M + 1e-3);
        }
    }

    fn shortcut_course() -> std::sync::Arc<RiversideCourse> {
        (1..)
            .map(course_for_seed)
            .find(|course| !course.branches().is_empty())
            .expect("random courses include shortcuts")
    }

    /// Rides the shortcut chord straight at the creek from a standing start
    /// far enough back to reach `speed`.
    fn ride_shortcut(course: &RiversideCourse, speed: f32) -> (BikeState, bool, f32) {
        let branch = &course.branches()[0];
        let creek = course.water()[0];
        let creek_m = branch.project(creek.x, creek.z).distance_m;
        let start = branch.point_at(creek_m - 16.0);
        let mut bike = BikeState {
            x: start.center_x,
            z: start.center_z,
            yaw: start.yaw,
            speed,
            ..BikeState::default()
        };
        let mut launched = false;
        let mut peak = 0.0_f32;
        for _ in 0..60 * 5 {
            // Hold the speed constant on approach so the test isolates the jump.
            bike.step(RiderInput::default(), FIXED_DT, course);
            if !bike.airborne && !launched {
                bike.speed = speed;
            }
            launched |= bike.airborne;
            peak = peak.max(bike.y);
        }
        (bike, launched, peak)
    }

    #[test]
    fn fast_bike_jumps_the_shortcut_creek_and_a_slow_one_splashes() {
        let course = shortcut_course();
        let (fast, launched, peak) = ride_shortcut(&course, MAX_SPEED);
        assert!(launched && peak > 1.0, "no real jump (peak {peak:.2} m)");
        assert!(!fast.airborne);
        assert_ne!(course.surface_at(fast.x, fast.z), Surface::Water);
        assert!(
            fast.speed > MAX_SPEED * 0.8,
            "fast landing kept {:.2}",
            fast.speed
        );

        let (slow, launched, _) = ride_shortcut(&course, 5.0);
        assert!(launched, "even a slow bike hops off the lip");
        assert!(
            slow.speed <= MAX_SPEED * WATER_MAX_SPEED_RATIO + 0.01,
            "slow bike kept {:.2} m/s after the creek",
            slow.speed
        );
    }

    #[test]
    fn shortcut_is_much_shorter_than_its_detour() {
        let course = shortcut_course();
        for branch in course.branches() {
            assert!(branch.length_m() < (branch.end_m - branch.start_m) * 0.75);
        }
    }

    #[test]
    fn knocking_a_cone_only_trims_speed() {
        let cone = OBSTACLES[0];
        let mut bike = bike_facing_cone(MAX_SPEED);
        let mut hits = 0;
        for _ in 0..30 {
            if bike.step(pedal(0.0), FIXED_DT, riverside_course()) {
                hits += 1;
            }
        }
        assert_eq!(hits, 1, "a cone should only be hit once");
        assert!(bike.z < cone.z - 1.0, "bike should ride through the cone");
        assert!(bike.speed > MAX_SPEED * 0.8);
    }

    #[test]
    fn holding_brake_while_stopped_reverses_after_a_short_delay() {
        let mut bike = BikeState::default();
        let brake = RiderInput {
            brake_held: true,
            ..RiderInput::default()
        };
        for _ in 0..6 {
            bike.step(brake, FIXED_DT, riverside_course());
        }
        assert_eq!(bike.speed, 0.0);
        for _ in 0..60 {
            bike.step(brake, FIXED_DT, riverside_course());
        }
        assert!(bike.speed < -1.0, "reverse speed {:.2}", bike.speed);
        for _ in 0..30 {
            bike.step(pedal(0.0), FIXED_DT, riverside_course());
        }
        assert!(bike.speed >= 0.0, "pedal should cancel reverse");
    }
}

#[cfg(test)]
mod combat_aim_tests {
    use super::{
        BikeState, GUNNER_AIM_ORIGIN_REAR_M, GunnerInput, WorldPosition, evaluate_gunner_aim,
        gunner_aim_origin, line_of_sight_blocked, riverside_course,
    };

    #[test]
    fn default_rear_seat_can_aim_at_the_chase_helicopter() {
        let bike = BikeState::default();
        let origin_y = 1.84;
        let pitch = (8.0_f32 - origin_y).atan2(14.0 - 1.03);
        let aim = evaluate_gunner_aim(
            riverside_course(),
            bike,
            GunnerInput {
                pitch,
                aim_held: true,
                ..GunnerInput::default()
            },
            WorldPosition {
                x: 0.0,
                y: 8.0,
                z: bike.z + 14.0,
            },
        );
        assert!(aim.in_range, "distance {:.2} m", aim.distance_m);
        assert!(aim.in_cone, "angle {:.2}°", aim.angle_degrees);
        assert!(!aim.obstructed);
    }

    #[test]
    fn full_yaw_range_can_aim_forward_and_rearward() {
        let bike = BikeState::default();
        for (yaw, z) in [(0.0, bike.z + 14.0), (std::f32::consts::PI, bike.z - 14.0)] {
            let target = WorldPosition { x: 0.0, y: 8.0, z };
            let origin_z = bike.z + 1.03;
            let pitch = (target.y - 1.84).atan2((target.z - origin_z).abs());
            let aim = evaluate_gunner_aim(
                riverside_course(),
                bike,
                GunnerInput {
                    weapon: Default::default(),
                    yaw,
                    pitch,
                    aim_held: true,
                },
                target,
            );
            assert!(
                aim.in_cone && aim.angle_degrees < 0.05,
                "yaw {yaw:.2}, angle {:.2}°",
                aim.angle_degrees
            );
        }
    }

    #[test]
    fn gunner_aim_origin_and_direction_follow_a_turned_bike() {
        let bike = BikeState {
            x: 4.0,
            z: -3.0,
            yaw: std::f32::consts::FRAC_PI_2,
            ..BikeState::default()
        };
        let origin = gunner_aim_origin(bike);
        assert!(
            (origin.x - (bike.x - GUNNER_AIM_ORIGIN_REAR_M)).abs() < 1.0e-5,
            "turned-bike rear origin is x={:.3}",
            origin.x
        );
        assert!((origin.z - bike.z).abs() < 1.0e-5);

        let target = WorldPosition {
            x: origin.x - 14.0,
            y: origin.y + 6.0,
            z: origin.z,
        };
        let aim = evaluate_gunner_aim(
            riverside_course(),
            bike,
            GunnerInput {
                pitch: (6.0_f32).atan2(14.0),
                aim_held: true,
                ..GunnerInput::default()
            },
            target,
        );
        assert!(aim.in_range);
        assert!(
            aim.in_cone,
            "turned-bike rear aim error {:.3}°",
            aim.angle_degrees
        );
        assert!(!aim.obstructed);
    }

    #[test]
    fn gameplay_obstacle_blocks_low_sightline_but_not_high_sightline() {
        let low_origin = WorldPosition {
            x: -0.55,
            y: 1.0,
            z: -44.0,
        };
        let low_target = WorldPosition {
            x: -0.55,
            y: 1.0,
            z: -30.0,
        };
        assert!(line_of_sight_blocked(
            riverside_course(),
            low_origin,
            low_target
        ));

        let high_target = WorldPosition {
            y: 8.0,
            ..low_target
        };
        assert!(!line_of_sight_blocked(
            riverside_course(),
            low_origin,
            high_target
        ));
    }
}
