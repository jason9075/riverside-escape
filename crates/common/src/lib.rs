//! Shared gameplay state and the small P1 WebSocket protocol.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::OnceLock,
};

use serde::{Deserialize, Serialize};

pub const SIMULATION_HZ: u32 = 60;
pub const SNAPSHOT_HZ: u32 = 20;
pub const COUNTDOWN_SECONDS: u8 = 5;
pub const MAX_SPEED: f32 = 11.0;
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
        y: GUNNER_AIM_ORIGIN_HEIGHT_M,
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
        obstructed: line_of_sight_blocked(origin, target),
    }
}

pub fn line_of_sight_blocked(origin: WorldPosition, target: WorldPosition) -> bool {
    for obstacle in OBSTACLES {
        let (height, padding) = match obstacle.kind {
            ObstacleKind::Cone => (0.9, 0.08),
            ObstacleKind::Bollard => (1.2, 0.08),
            ObstacleKind::Barrier => (1.4, 0.0),
        };
        let min_x = obstacle.x - obstacle.half_width - padding;
        let max_x = obstacle.x + obstacle.half_width + padding;
        let min_z = obstacle.z - obstacle.half_depth - padding;
        let max_z = obstacle.z + obstacle.half_depth + padding;
        let Some((entry, exit)) = segment_box_interval(origin, target, min_x, max_x, min_z, max_z)
        else {
            continue;
        };
        let t = entry.clamp(0.0, 1.0);
        if t <= exit && origin.y + (target.y - origin.y) * t <= height {
            return true;
        }
    }
    first_cover_intersection(origin, target).is_some()
}

/// Returns the first entry fraction along a 3D line segment that intersects
/// shared, server-safe cover geometry.
pub fn first_cover_intersection(origin: WorldPosition, target: WorldPosition) -> Option<f32> {
    riverside_course()
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
}

impl Default for BikeState {
    fn default() -> Self {
        Self {
            x: 0.0,
            z: 2.0,
            yaw: 0.0,
            speed: 0.0,
            steering_angle: 0.0,
        }
    }
}

impl BikeState {
    /// Arcade/kinematic bicycle controller shared by offline preview and the server.
    /// `steer < 0` is left and `steer > 0` is right; positive yaw turns right.
    pub fn step(&mut self, input: RiderInput, dt: f32) -> bool {
        let dt = dt.clamp(0.0, 0.05);
        let previous_speed = self.speed;
        if input.brake_held {
            self.speed = (self.speed - 5.5 * dt).max(0.0);
        } else if input.pedal_held {
            self.speed = (self.speed + 2.8 * dt).min(MAX_SPEED);
        } else {
            // Releasing the pedal coasts down slowly. Braking remains a distinct action.
            self.speed = (self.speed - 0.28 * dt).max(0.0);
        }

        let steer = input.steer.clamp(-1.0, 1.0);
        let target_steering_angle = steer * MAX_FRONT_STEER_ANGLE;
        let steering_rate = if steer == 0.0 { 1.8 } else { 1.35 };
        self.steering_angle = move_toward(
            self.steering_angle,
            target_steering_angle,
            steering_rate * dt,
        );
        if self.speed > 0.0 {
            // A simple bicycle controller: steering changes curvature, while
            // body yaw can only change as the bicycle travels forward.
            let steer_ratio = self.steering_angle / MAX_FRONT_STEER_ANGLE;
            let yaw_rate = self.speed * 0.148 * steer_ratio;
            self.yaw = (self.yaw + yaw_rate * dt).rem_euclid(std::f32::consts::TAU);
        }

        let next_x = self.x + self.yaw.sin() * self.speed * dt;
        let next_z = self.z - self.yaw.cos() * self.speed * dt;
        if OBSTACLES
            .iter()
            .any(|obstacle| obstacle.blocks(next_x, next_z))
        {
            self.speed = 0.0;
            return previous_speed > 0.5;
        }

        // Prediction and authority use the same authored road centerline and
        // width. This also keeps turns continuous instead of imposing the old
        // short prototype's global X boundary on the whole level.
        if !riverside_course().contains(next_x, next_z) {
            self.speed = (self.speed - 4.0 * dt).max(0.0);
            return previous_speed > self.speed + 0.5;
        }

        self.x = next_x;
        self.z = next_z;
        previous_speed > 0.5 && self.speed == 0.0
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

#[derive(Debug)]
pub struct RiversideCourse {
    pieces: Vec<RoadPiece>,
    points: Vec<CoursePoint>,
    cover_volumes: Vec<CoverVolume>,
    checkpoints_m: Vec<f32>,
    finish_distance_m: f32,
    length_m: f32,
}

/// The level layout is built once from the same lightweight data used by
/// rendering, Rider prediction, and the headless Server.
pub fn riverside_course() -> &'static RiversideCourse {
    static COURSE: OnceLock<RiversideCourse> = OnceLock::new();
    COURSE.get_or_init(RiversideCourse::build)
}

impl RiversideCourse {
    fn build() -> Self {
        let mut builder = CourseBuilder::new();

        // Four alternating S-bends keep the open section legible while
        // keeping the existing prototype obstacles on its initial straight.
        for (index, straights) in [11, 11, 11, 12].into_iter().enumerate() {
            let lead = straights / 2;
            builder.straights(lead, RoadSection::OpenRiverside);
            let first_turn = if index % 2 == 0 {
                RoadPieceKind::CurveRight
            } else {
                RoadPieceKind::CurveLeft
            };
            builder.curve(first_turn, RoadSection::OpenRiverside);
            builder.straights(straights - lead, RoadSection::OpenRiverside);
            builder.curve(opposite_turn(first_turn), RoadSection::OpenRiverside);
        }

        builder.straights(50, RoadSection::BridgeUnderpass);

        // Repeated S-bends make the last section visibly more technical but
        // preserve broad enough radii for the existing bicycle controller.
        for index in 0..6 {
            let first_turn = if index % 2 == 0 {
                RoadPieceKind::CurveRight
            } else {
                RoadPieceKind::CurveLeft
            };
            builder.straights(5, RoadSection::NarrowBends);
            builder.curve(first_turn, RoadSection::NarrowBends);
            builder.straights(2, RoadSection::NarrowBends);
            builder.curve(opposite_turn(first_turn), RoadSection::NarrowBends);
        }

        let mut course = builder.finish();
        course.add_route_markers();
        course.add_bridge_cover();
        course
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
        lateral_offset.abs() <= TRACK_HALF_WIDTH - BIKE_COLLISION_HALF_WIDTH
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
        let pair = self
            .points
            .windows(2)
            .find(|pair| target <= pair[1].distance_m)
            .unwrap_or_else(|| {
                let end = self.points.len() - 1;
                &self.points[end - 1..=end]
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
            section: self.section_at(target),
            center_x: pair[0].x + (pair[1].x - pair[0].x) * alpha,
            center_z: pair[0].z + (pair[1].z - pair[0].z) * alpha,
            yaw: approach_course_angle(pair[0].yaw, pair[1].yaw, alpha),
        }
    }

    pub fn project(&self, x: f32, z: f32) -> CourseProjection {
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
        for pair in self.points.windows(2) {
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
            best = (
                distance_squared,
                CourseProjection {
                    distance_m: pair[0].distance_m
                        + (pair[1].distance_m - pair[0].distance_m) * alpha,
                    lateral_m: offset_x * yaw.cos() + offset_z * yaw.sin(),
                    center_distance_m: distance_squared.sqrt(),
                    section: self.section_at(
                        pair[0].distance_m + (pair[1].distance_m - pair[0].distance_m) * alpha,
                    ),
                    center_x,
                    center_z,
                    yaw,
                },
            );
        }
        best.1
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
    x: f32,
    z: f32,
    yaw: f32,
    distance_m: f32,
}

impl CourseBuilder {
    fn new() -> Self {
        Self {
            pieces: Vec::new(),
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

    fn finish(self) -> RiversideCourse {
        RiversideCourse {
            pieces: self.pieces,
            points: self.points,
            cover_volumes: Vec::new(),
            checkpoints_m: Vec::new(),
            finish_distance_m: 0.0,
            length_m: self.distance_m,
        }
    }
}

fn local_to_world(local_x: f32, local_z: f32, yaw: f32) -> (f32, f32) {
    (
        yaw.cos() * local_x - yaw.sin() * local_z,
        yaw.sin() * local_x + yaw.cos() * local_z,
    )
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
        BIKE_COLLISION_HALF_WIDTH, BRIDGE_DECK_BOTTOM_M, BRIDGE_DECK_TOP_M, BikeState, CoverKind,
        FIXED_DT, MAX_SPEED, ROAD_CURVE_LENGTH_M, ROAD_STRAIGHT_LENGTH_M, RiderInput,
        RoadPieceKind, RoadSection, TRACK_HALF_WIDTH, WorldPosition, first_cover_intersection,
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
        let outside_before = position(-0.5, 2.2);
        let outside_after = position(0.5, 2.2);
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
        assert!(line_of_sight_blocked(under_roof, helicopter));
        let hit = first_cover_intersection(under_roof, helicopter).unwrap();
        let hit_y = under_roof.y + (helicopter.y - under_roof.y) * hit;
        assert!((hit_y - BRIDGE_DECK_BOTTOM_M).abs() < 0.01);

        let bike_lane = WorldPosition {
            y: BRIDGE_DECK_BOTTOM_M - 0.2,
            ..under_roof
        };
        assert!(!line_of_sight_blocked(
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

    #[test]
    fn fixed_step_rider_can_follow_the_full_route_and_avoid_the_early_barrier() {
        let course = riverside_course();
        let mut bike = BikeState::default();
        let mut highest_distance = 0.0_f32;
        let mut reached = false;
        let mut elapsed_ticks = 0;
        for _ in 0..60 * 240 {
            elapsed_ticks += 1;
            let projection = course.project(bike.x, bike.z);
            highest_distance = highest_distance.max(projection.distance_m);
            if projection.distance_m >= course.length_m() - 3.0 {
                reached = true;
                break;
            }
            let lookahead = course.point_at((projection.distance_m + 4.0).min(course.length_m()));
            let obstacle_detour = if (30.0..45.0).contains(&projection.distance_m) {
                1.25
            } else {
                0.0
            };
            let target_x = lookahead.center_x + lookahead.yaw.cos() * obstacle_detour;
            let target_z = lookahead.center_z + lookahead.yaw.sin() * obstacle_detour;
            let desired_yaw = (target_x - bike.x).atan2(bike.z - target_z);
            let heading_error = angle_delta(bike.yaw, desired_yaw);
            let desired_curvature = course.curvature_at(projection.distance_m + 2.0);
            let steer = ((desired_curvature + heading_error * 0.65) / 0.148).clamp(-1.0, 1.0);
            bike.step(
                RiderInput {
                    pedal_held: true,
                    steer,
                    ..RiderInput::default()
                },
                FIXED_DT,
            );
            let after = course.project(bike.x, bike.z);
            highest_distance = highest_distance.max(after.distance_m);
            assert!(
                course.contains(bike.x, bike.z),
                "bike left road at {:.1} m ({:.2}, {:.2})",
                after.distance_m,
                bike.x,
                bike.z
            );
        }
        assert!(
            reached,
            "rider only reached {:.1} / {:.1} m at {:.1} m/s (max speed {:.1})",
            highest_distance,
            course.length_m(),
            bike.speed,
            MAX_SPEED
        );
        eprintln!(
            "controlled Rider traversed {:.1} m in {:.1} s ({} fixed ticks)",
            highest_distance,
            elapsed_ticks as f32 * FIXED_DT,
            elapsed_ticks
        );
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
}

impl RiderPrediction {
    pub fn new(tick: u64, bike: BikeState) -> Self {
        let mut prediction = Self {
            current_tick: tick,
            bike,
            history: VecDeque::new(),
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
        let collision = self.bike.step(input, FIXED_DT);
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

#[derive(Clone, Copy, Debug)]
pub struct Obstacle {
    pub kind: ObstacleKind,
    pub x: f32,
    pub z: f32,
    pub half_width: f32,
    pub half_depth: f32,
}

impl Obstacle {
    pub fn blocks(self, x: f32, z: f32) -> bool {
        (x - self.x).abs() <= self.half_width + 0.28 && (z - self.z).abs() <= self.half_depth + 0.45
    }
}

pub const OBSTACLES: [Obstacle; 5] = [
    Obstacle {
        kind: ObstacleKind::Cone,
        x: -1.15,
        z: -11.0,
        half_width: 0.28,
        half_depth: 0.28,
    },
    Obstacle {
        kind: ObstacleKind::Bollard,
        x: 1.35,
        z: -24.0,
        half_width: 0.35,
        half_depth: 0.35,
    },
    Obstacle {
        kind: ObstacleKind::Barrier,
        x: -0.55,
        z: -38.0,
        half_width: 0.72,
        half_depth: 0.3,
    },
    Obstacle {
        kind: ObstacleKind::Cone,
        x: 1.1,
        z: -50.0,
        half_width: 0.28,
        half_depth: 0.28,
    },
    Obstacle {
        kind: ObstacleKind::Bollard,
        x: -1.45,
        z: -65.0,
        half_width: 0.35,
        half_depth: 0.35,
    },
];

#[cfg(test)]
mod combat_aim_tests {
    use super::{
        BikeState, GUNNER_AIM_ORIGIN_REAR_M, GunnerInput, WorldPosition, evaluate_gunner_aim,
        gunner_aim_origin, line_of_sight_blocked,
    };

    #[test]
    fn default_rear_seat_can_aim_at_the_chase_helicopter() {
        let bike = BikeState::default();
        let origin_y = 1.84;
        let pitch = (8.0_f32 - origin_y).atan2(14.0 - 1.03);
        let aim = evaluate_gunner_aim(
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
                bike,
                GunnerInput {
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
            speed: 0.0,
            steering_angle: 0.0,
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
        assert!(line_of_sight_blocked(low_origin, low_target));

        let high_target = WorldPosition {
            y: 8.0,
            ..low_target
        };
        assert!(!line_of_sight_blocked(low_origin, high_target));
    }
}
