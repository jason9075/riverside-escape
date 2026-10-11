//! Shared gameplay state and the small P1 WebSocket protocol.

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

pub const SIMULATION_HZ: u32 = 60;
pub const SNAPSHOT_HZ: u32 = 20;
pub const COUNTDOWN_SECONDS: u8 = 5;
pub const MAX_SPEED: f32 = 11.0;
pub const TRACK_HALF_WIDTH: f32 = 2.3;
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

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GamePhase {
    #[default]
    WaitingForPlayers,
    ReadyCheck,
    Countdown,
    Running,
    Finished,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchResult {
    RidersVictory,
    RidersDefeat,
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
    SetReady {
        ready: bool,
    },
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
        seat: SeatKind,
        solo_mode: bool,
    },
    SeatChanged {
        seat: SeatKind,
    },
    Snapshot(Box<WorldSnapshot>),
    RoomFull,
    SoloUnavailable,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct SeatStatus {
    pub connected: bool,
    pub ready: bool,
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

pub fn gunner_aim_direction(bike: BikeState, input: GunnerInput) -> WorldPosition {
    let yaw = bike.yaw + std::f32::consts::PI + input.yaw;
    let pitch_cos = input.pitch.cos();
    WorldPosition {
        x: -yaw.sin() * pitch_cos,
        y: input.pitch.sin(),
        z: -yaw.cos() * pitch_cos,
    }
}

pub fn evaluate_gunner_aim(
    bike: BikeState,
    input: GunnerInput,
    target: WorldPosition,
) -> AimEvaluation {
    let origin_yaw = bike.yaw + std::f32::consts::PI;
    let origin = WorldPosition {
        x: bike.x - origin_yaw.sin() * GUNNER_AIM_ORIGIN_REAR_M,
        y: GUNNER_AIM_ORIGIN_HEIGHT_M,
        z: bike.z - origin_yaw.cos() * GUNNER_AIM_ORIGIN_REAR_M,
    };
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

fn line_of_sight_blocked(origin: WorldPosition, target: WorldPosition) -> bool {
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
    false
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
    pub countdown_seconds: Option<u8>,
    pub bike: BikeState,
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

        // The prototype lane is bounded by the road shoulders.
        if next_x.abs() > TRACK_HALF_WIDTH - 0.28 {
            self.speed = (self.speed - 4.0 * dt).max(0.0);
            self.x = self
                .x
                .clamp(-TRACK_HALF_WIDTH + 0.28, TRACK_HALF_WIDTH - 0.28);
            return previous_speed > self.speed + 0.5;
        }

        self.x = next_x;
        self.z = next_z;
        previous_speed > 0.5 && self.speed == 0.0
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
        BikeState, GunnerInput, WorldPosition, evaluate_gunner_aim, line_of_sight_blocked,
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
