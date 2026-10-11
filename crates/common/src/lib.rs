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
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CombatEventKind {
    RocketWarning,
    RocketLaunched,
    RocketExploded { hit: bool, hit_count: u32 },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct CombatEvent {
    pub event_id: u64,
    pub attack_id: u64,
    pub tick: u64,
    pub position: WorldPosition,
    pub kind: CombatEventKind,
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
