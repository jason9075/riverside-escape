use std::collections::{BTreeMap, BTreeSet, VecDeque};

use bevy::{
    gltf::GltfAssetLabel,
    input::mouse::AccumulatedMouseMotion,
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow, WindowFocused},
    world_serialization::WorldAssetRoot,
};
use riverside_common::{
    BIKE_STARTING_HEALTH, BikeRenderOffset, BikeState, ClientMessage, CombatEvent, CombatEventKind,
    FIXED_DT_SECONDS, FireRejectionReason, GUNNER_LOCK_CONE_HALF_ANGLE_DEGREES, GUNNER_LOCK_TICKS,
    GUNNER_PITCH_MAX_RADIANS, GUNNER_PITCH_MIN_RADIANS, GUNNER_STARTING_AMMO, GamePhase,
    GunnerInput, HELICOPTER_ALTITUDE_M, HELICOPTER_STARTING_HEALTH, HelicopterSnapshot,
    MatchResult, OBSTACLES, ObstacleKind, PlayerMissileSnapshot, REQUIRED_SEATS,
    ROAD_STRAIGHT_LENGTH_M, ROCKET_BLAST_RADIUS_M, RiderInput, RiderInputSample, RiderPrediction,
    RoadPieceKind, RocketSnapshot, SIMULATION_HZ, SeatKind, SeatStatus, SnapshotInterpolator,
    WorldPosition, WorldSnapshot, evaluate_gunner_aim, riverside_course,
};

use crate::network::NetworkClient;

const CAMERA_RETURN_SPEED: f32 = 6.0;
const MOUSE_SENSITIVITY: f32 = 0.004;
const GUNNER_HOME_YAW: f32 = std::f32::consts::PI;
const RIDER_CAMERA_PITCH: f32 = -0.19;
const GUNNER_SEAT: Vec3 = Vec3::new(0.0, 1.12, 0.94);
const GUNNER_CAMERA_AIM: Vec3 = Vec3::new(0.0, 0.72, -0.09);
const INPUT_SEND_INTERVAL_SECS: f64 = 1.0 / riverside_common::SNAPSHOT_HZ as f64;

#[derive(Resource)]
pub struct GameSession {
    pub bike: BikeState,
    pub authoritative_bike: BikeState,
    has_authoritative_bike: bool,
    pub local_seat: SeatKind,
    pub phase: GamePhase,
    pub seats: BTreeMap<SeatKind, SeatStatus>,
    pub countdown_seconds: Option<u8>,
    pub client_tick: u64,
    pub server_tick: u64,
    pub is_connected: bool,
    pub solo_mode: bool,
    pub ready: bool,
    pub rider_input: RiderInput,
    pub gunner_input: GunnerInput,
    pub prediction: RiderPrediction,
    pub prediction_initialized: bool,
    prediction_accumulator: f64,
    input_send_accumulator: f64,
    render_offset: BikeRenderOffset,
    pub snapshot_interpolator: SnapshotInterpolator,
    snapshot_received_this_frame: bool,
    pending_authoritative_snapshots: VecDeque<WorldSnapshot>,
    pub snapshot_gap_m: f32,
    pub max_snapshot_gap_m: f32,
    pub same_tick_error_m: f32,
    pub max_same_tick_error_m: f32,
    pub same_tick_yaw_error_deg: f32,
    pub max_same_tick_yaw_error_deg: f32,
    pub replayed_ticks_last_snapshot: u32,
    pub replayed_unacknowledged_inputs_last_snapshot: u32,
    pub history_miss_count: u32,
    pub reconcile_bike_step_m: f32,
    pub max_reconcile_bike_step_m: f32,
    pub gunner_interpolation_lag_ms: f32,
    pub max_update_delta_ms: f32,
    pub last_applied_rider_sequence: u32,
    pub pending_rider_inputs: VecDeque<RiderInputSample>,
    pub max_pending_rider_inputs: usize,
    pub helicopter: Option<HelicopterSnapshot>,
    pub rocket: Option<RocketSnapshot>,
    pub player_missile: Option<PlayerMissileSnapshot>,
    pub rocket_hit_count: u32,
    pub bike_health: u16,
    pub helicopter_health: u16,
    pub gunner_ammo: u8,
    pub gunner_lock_progress_ticks: u16,
    pub gunner_lock_grace_ticks_remaining: u16,
    pub gunner_locked: bool,
    pub gunner_can_fire: bool,
    pub fire_cooldown_ticks_remaining: u16,
    pub match_result: Option<MatchResult>,
    pub combat_notice_secs: f32,
    pub last_combat_event: Option<CombatEvent>,
    pub explosion_position: Option<WorldPosition>,
    seen_combat_event_ids: BTreeSet<u64>,
    combat_event_order: VecDeque<u64>,
    pub collision_flash: f32,
    pub rider_camera_yaw: f32,
    pub rider_camera_pitch: f32,
    pub rider_free_look: bool,
    pub cursor_captured: bool,
    pub focused: bool,
    pub show_diagnostics: bool,
    input_sequence: u32,
    fire_request_sequence: u64,
    predicted_lock_progress_ticks: u16,
    predicted_lock_outside_secs: f32,
    fire_feedback: Option<(u64, Result<u8, FireRejectionReason>)>,
}

impl Default for GameSession {
    fn default() -> Self {
        let seats = REQUIRED_SEATS
            .into_iter()
            .map(|seat| (seat, SeatStatus::default()))
            .collect();
        Self {
            bike: BikeState::default(),
            authoritative_bike: BikeState::default(),
            has_authoritative_bike: false,
            local_seat: SeatKind::Rider,
            phase: GamePhase::Running,
            seats,
            countdown_seconds: None,
            client_tick: 0,
            server_tick: 0,
            is_connected: false,
            solo_mode: false,
            ready: false,
            rider_input: RiderInput::default(),
            gunner_input: GunnerInput::default(),
            prediction: RiderPrediction::new(0, BikeState::default()),
            prediction_initialized: false,
            prediction_accumulator: 0.0,
            input_send_accumulator: 0.0,
            render_offset: BikeRenderOffset::default(),
            snapshot_interpolator: SnapshotInterpolator::default(),
            snapshot_received_this_frame: false,
            pending_authoritative_snapshots: VecDeque::new(),
            snapshot_gap_m: 0.0,
            max_snapshot_gap_m: 0.0,
            same_tick_error_m: 0.0,
            max_same_tick_error_m: 0.0,
            same_tick_yaw_error_deg: 0.0,
            max_same_tick_yaw_error_deg: 0.0,
            replayed_ticks_last_snapshot: 0,
            replayed_unacknowledged_inputs_last_snapshot: 0,
            history_miss_count: 0,
            reconcile_bike_step_m: 0.0,
            max_reconcile_bike_step_m: 0.0,
            gunner_interpolation_lag_ms: 0.0,
            max_update_delta_ms: 0.0,
            last_applied_rider_sequence: 0,
            pending_rider_inputs: VecDeque::new(),
            max_pending_rider_inputs: 0,
            helicopter: None,
            rocket: None,
            player_missile: None,
            rocket_hit_count: 0,
            bike_health: BIKE_STARTING_HEALTH,
            helicopter_health: HELICOPTER_STARTING_HEALTH,
            gunner_ammo: GUNNER_STARTING_AMMO,
            gunner_lock_progress_ticks: 0,
            gunner_lock_grace_ticks_remaining: 0,
            gunner_locked: false,
            gunner_can_fire: false,
            fire_cooldown_ticks_remaining: 0,
            match_result: None,
            combat_notice_secs: 0.0,
            last_combat_event: None,
            explosion_position: None,
            seen_combat_event_ids: BTreeSet::new(),
            combat_event_order: VecDeque::new(),
            collision_flash: 0.0,
            rider_camera_yaw: 0.0,
            rider_camera_pitch: RIDER_CAMERA_PITCH,
            rider_free_look: false,
            cursor_captured: false,
            focused: true,
            show_diagnostics: false,
            input_sequence: 0,
            fire_request_sequence: 0,
            predicted_lock_progress_ticks: 0,
            predicted_lock_outside_secs: 0.0,
            fire_feedback: None,
        }
    }
}

impl GameSession {
    pub fn change_seat(&mut self, seat: SeatKind) {
        self.local_seat = seat;
        self.pending_rider_inputs.clear();
        self.pending_authoritative_snapshots.clear();
        self.predicted_lock_progress_ticks = 0;
        self.predicted_lock_outside_secs = 0.0;
        if seat == SeatKind::Rider && self.prediction_initialized {
            self.prediction
                .reset(self.server_tick, self.authoritative_bike);
            self.client_tick = self.server_tick;
            self.input_send_accumulator = 0.0;
            self.render_offset = BikeRenderOffset::default();
            self.bike = self.authoritative_bike;
        }
    }

    pub fn reset_reconciliation_metrics(&mut self) {
        self.snapshot_gap_m = 0.0;
        self.max_snapshot_gap_m = 0.0;
        self.same_tick_error_m = 0.0;
        self.max_same_tick_error_m = 0.0;
        self.same_tick_yaw_error_deg = 0.0;
        self.max_same_tick_yaw_error_deg = 0.0;
        self.replayed_ticks_last_snapshot = 0;
        self.replayed_unacknowledged_inputs_last_snapshot = 0;
        self.history_miss_count = 0;
        self.reconcile_bike_step_m = 0.0;
        self.max_reconcile_bike_step_m = 0.0;
        self.gunner_interpolation_lag_ms = 0.0;
        self.max_update_delta_ms = 0.0;
        self.pending_rider_inputs.clear();
        self.max_pending_rider_inputs = 0;
    }

    pub fn begin_running(&mut self, tick: u64, bike: BikeState, ack: u32) {
        self.reset_reconciliation_metrics();
        self.prediction.reset(tick, bike);
        self.prediction_initialized = true;
        self.prediction_accumulator = 0.0;
        self.input_send_accumulator = 0.0;
        self.render_offset = BikeRenderOffset::default();
        self.snapshot_interpolator.clear();
        self.snapshot_interpolator.push(tick, bike);
        self.snapshot_received_this_frame = true;
        self.pending_authoritative_snapshots.clear();
        self.bike = bike;
        self.authoritative_bike = bike;
        self.has_authoritative_bike = true;
        self.server_tick = tick;
        self.client_tick = tick;
        self.last_applied_rider_sequence = ack;
        self.fire_feedback = None;
        self.match_result = None;
        self.combat_notice_secs = 0.0;
        self.last_combat_event = None;
    }

    pub fn receive_prestart_snapshot(&mut self, tick: u64, bike: BikeState) {
        self.server_tick = tick;
        self.client_tick = tick;
        self.authoritative_bike = bike;
        self.has_authoritative_bike = true;
        self.bike = bike;
        self.pending_authoritative_snapshots.clear();
    }

    pub fn apply_combat_snapshot(&mut self, snapshot: &WorldSnapshot) {
        self.rocket_hit_count = snapshot.rocket_hit_count;
        self.bike_health = snapshot.bike_health;
        self.helicopter_health = snapshot.helicopter_health;
        self.gunner_ammo = snapshot.gunner_ammo;
        self.gunner_lock_progress_ticks = snapshot.gunner_lock_progress_ticks;
        self.gunner_lock_grace_ticks_remaining = snapshot.gunner_lock_grace_ticks_remaining;
        self.gunner_locked = snapshot.gunner_locked;
        self.gunner_can_fire = snapshot.gunner_can_fire;
        self.fire_cooldown_ticks_remaining = snapshot.fire_cooldown_ticks_remaining;
        self.match_result = snapshot.match_result;
        if snapshot.phase != GamePhase::Running {
            self.helicopter = None;
            self.rocket = None;
            self.player_missile = None;
            self.explosion_position = None;
            if snapshot.phase != GamePhase::Finished {
                self.combat_notice_secs = 0.0;
                self.last_combat_event = None;
                self.fire_feedback = None;
            }
            for event in snapshot.combat_events.iter().copied() {
                self.observe_combat_event(event);
            }
            return;
        }

        self.helicopter = snapshot.helicopter;
        self.rocket = snapshot.rocket;
        self.player_missile = snapshot.player_missile;
        for event in snapshot.combat_events.iter().copied() {
            self.observe_combat_event(event);
        }
    }

    fn observe_combat_event(&mut self, event: CombatEvent) -> bool {
        if !self.seen_combat_event_ids.insert(event.event_id) {
            return false;
        }
        self.combat_event_order.push_back(event.event_id);
        while self.combat_event_order.len() > 256 {
            if let Some(expired) = self.combat_event_order.pop_front() {
                self.seen_combat_event_ids.remove(&expired);
            }
        }
        match event.kind {
            CombatEventKind::RocketExploded { .. }
            | CombatEventKind::PlayerMissileImpacted { .. } => {
                self.explosion_position = Some(event.position);
                self.combat_notice_secs = 1.6;
                self.last_combat_event = Some(event);
            }
            CombatEventKind::GunnerFireAccepted {
                request_id,
                ammo_remaining,
            } => {
                self.fire_feedback = Some((request_id, Ok(ammo_remaining)));
                self.combat_notice_secs = 0.8;
                self.last_combat_event = Some(event);
            }
            CombatEventKind::GunnerFireRejected { request_id, reason } => {
                self.fire_feedback = Some((request_id, Err(reason)));
                self.combat_notice_secs = 1.6;
                self.last_combat_event = Some(event);
            }
            CombatEventKind::MatchFinished { result } => {
                self.match_result = Some(result);
                self.combat_notice_secs = 3.0;
                self.last_combat_event = Some(event);
            }
            _ => {}
        }
        true
    }

    pub fn queue_running_snapshot(&mut self, snapshot: WorldSnapshot) {
        self.pending_authoritative_snapshots.push_back(snapshot);
    }

    pub fn receive_running_snapshot(&mut self, tick: u64, bike: BikeState, ack: u32) -> bool {
        let server_turn_changed = self.has_authoritative_bike
            && shortest_angle_delta(self.authoritative_bike.yaw, bike.yaw).abs() > 0.01;
        self.server_tick = tick;
        self.authoritative_bike = bike;
        self.has_authoritative_bike = true;
        self.last_applied_rider_sequence = ack;
        let result = (self.local_seat == SeatKind::Rider).then(|| {
            self.prediction
                .reconcile(tick, bike, ack, &self.pending_rider_inputs)
        });
        self.pending_rider_inputs
            .retain(|sample| sample.sequence > ack);
        self.max_pending_rider_inputs = self
            .max_pending_rider_inputs
            .max(self.pending_rider_inputs.len());
        self.snapshot_interpolator.push(tick, bike);
        self.snapshot_received_this_frame = true;

        if let Some(result) = result {
            let visible_before = self.bike;
            self.client_tick = self.prediction.current_tick();
            self.replayed_ticks_last_snapshot = result.replayed_ticks;
            self.replayed_unacknowledged_inputs_last_snapshot =
                result.replayed_unacknowledged_inputs;
            if let Some(error) = result.same_tick_position_error_m {
                self.same_tick_error_m = error;
                self.max_same_tick_error_m = self.max_same_tick_error_m.max(error);
            }
            if let Some(error) = result.same_tick_yaw_error_deg {
                self.same_tick_yaw_error_deg = error;
                self.max_same_tick_yaw_error_deg = self.max_same_tick_yaw_error_deg.max(error);
            }
            if !result.history_complete {
                self.history_miss_count = self.history_miss_count.saturating_add(1);
            }
            let predicted_now = self.prediction.bike();
            self.snapshot_gap_m = (predicted_now.x - bike.x).hypot(predicted_now.z - bike.z);
            self.max_snapshot_gap_m = self.max_snapshot_gap_m.max(self.snapshot_gap_m);
            self.render_offset = BikeRenderOffset::between(visible_before, predicted_now);
        }
        server_turn_changed
    }
}

#[derive(Component)]
pub(super) struct BikeRoot;

#[derive(Component)]
struct RiderVisual;

#[derive(Component)]
pub(super) struct GunnerVisual;

#[derive(Component)]
pub(super) struct FollowCamera;

#[derive(Component)]
pub(super) struct HudText;

#[derive(Component)]
pub(super) struct Reticle;

#[derive(Component)]
pub(super) struct HelicopterVisual;

#[derive(Component)]
pub(super) struct RocketVisual;

#[derive(Component)]
pub(super) struct DangerZoneVisual;

#[derive(Component)]
pub(super) struct ExplosionVisual;

#[derive(Component)]
pub(super) struct PlayerMissileVisual;

#[derive(Component)]
pub(super) struct LockConeRing;

#[derive(Component)]
pub(super) struct CombatHudText;

type HelicopterVisualFilter = (
    With<HelicopterVisual>,
    Without<RocketVisual>,
    Without<DangerZoneVisual>,
    Without<ExplosionVisual>,
);
type RocketVisualFilter = (
    With<RocketVisual>,
    Without<HelicopterVisual>,
    Without<DangerZoneVisual>,
    Without<ExplosionVisual>,
);
type DangerZoneVisualFilter = (
    With<DangerZoneVisual>,
    Without<HelicopterVisual>,
    Without<RocketVisual>,
    Without<ExplosionVisual>,
);
type ExplosionVisualFilter = (
    With<ExplosionVisual>,
    Without<HelicopterVisual>,
    Without<RocketVisual>,
    Without<DangerZoneVisual>,
);
type PlayerMissileVisualFilter = (
    With<PlayerMissileVisual>,
    Without<HelicopterVisual>,
    Without<RocketVisual>,
    Without<DangerZoneVisual>,
    Without<ExplosionVisual>,
);
type LockConeRingFilter = (With<LockConeRing>, Without<Reticle>);

pub fn setup_scene(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(ClearColor(Color::srgb(0.62, 0.78, 0.79)));
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.88, 0.96, 0.95),
        brightness: 180.0,
        ..default()
    });

    let grass = materials.add(StandardMaterial {
        base_color: Color::srgb(0.23, 0.40, 0.31),
        perceptual_roughness: 0.96,
        ..default()
    });
    let rail_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.76, 0.82, 0.77),
        metallic: 0.32,
        perceptual_roughness: 0.46,
        ..default()
    });
    let lane_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.90, 0.91, 0.81),
        emissive: LinearRgba::rgb(0.025, 0.03, 0.02),
        ..default()
    });

    let course = riverside_course();
    let course_length = course.length_m();
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(600.0, course_length + 160.0))),
        MeshMaterial3d(grass),
        Transform::from_xyz(0.0, -0.23, BikeState::default().z - course_length * 0.5),
    ));

    let rail_mesh = meshes.add(Cuboid::new(0.12, 0.44, 8.0));
    let lane_mesh = meshes.add(Cuboid::new(0.055, 0.018, 8.0));
    let short_rail_mesh = meshes.add(Cuboid::new(0.12, 0.44, 1.0));
    // Give the rear-facing Gunner a short visual road behind the start line;
    // the playable course itself still begins at the default bike position.
    for segment in 1..=6 {
        let base = Transform::from_xyz(
            0.0,
            0.0,
            BikeState::default().z + ROAD_STRAIGHT_LENGTH_M * segment as f32,
        );
        commands.spawn((
            WorldAssetRoot(
                asset_server.load(GltfAssetLabel::Scene(0).from_asset("models/road_straight.glb")),
            ),
            base,
        ));
        for side in [-1.0, 1.0] {
            let mut rail_transform = base;
            rail_transform.translation += Vec3::new(side * 2.55, 0.34, -4.0);
            commands.spawn((
                Mesh3d(rail_mesh.clone()),
                MeshMaterial3d(rail_material.clone()),
                rail_transform,
            ));
            let mut lane_transform = base;
            lane_transform.translation += Vec3::new(side * 1.58, 0.015, -4.0);
            commands.spawn((
                Mesh3d(lane_mesh.clone()),
                MeshMaterial3d(lane_material.clone()),
                lane_transform,
            ));
        }
    }
    for piece in course.pieces() {
        let road_asset = match piece.kind {
            RoadPieceKind::Straight => "models/road_straight.glb",
            RoadPieceKind::CurveRight | RoadPieceKind::CurveLeft => "models/road_curve.glb",
        };
        let base = Transform::from_xyz(piece.root_x, 0.0, piece.root_z)
            .with_rotation(Quat::from_rotation_y(-piece.root_yaw));
        commands.spawn((
            WorldAssetRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(road_asset))),
            base,
        ));

        if piece.kind == RoadPieceKind::Straight {
            for side in [-1.0, 1.0] {
                let mut rail_transform = base;
                rail_transform.translation += base.rotation * Vec3::new(side * 2.55, 0.34, -4.0);
                commands.spawn((
                    Mesh3d(rail_mesh.clone()),
                    MeshMaterial3d(rail_material.clone()),
                    rail_transform,
                ));
                let mut lane_transform = base;
                lane_transform.translation += base.rotation * Vec3::new(side * 1.58, 0.015, -4.0);
                commands.spawn((
                    Mesh3d(lane_mesh.clone()),
                    MeshMaterial3d(lane_material.clone()),
                    lane_transform,
                ));
            }
        } else {
            let sections = (piece.length_m / 1.0).ceil() as usize;
            for index in 0..sections {
                let start = index as f32;
                let end = (start + 1.0).min(piece.length_m);
                let center = course.point_at(piece.start_distance_m + (start + end) * 0.5);
                let yaw = center.yaw;
                let rotation = Quat::from_rotation_y(-yaw);
                for side in [-1.0, 1.0] {
                    let right = Vec3::new(yaw.cos(), 0.0, yaw.sin());
                    commands.spawn((
                        Mesh3d(short_rail_mesh.clone()),
                        MeshMaterial3d(rail_material.clone()),
                        Transform::from_xyz(center.center_x, 0.34, center.center_z)
                            .with_translation(
                                Vec3::new(center.center_x, 0.34, center.center_z)
                                    + right * side * 2.55,
                            )
                            .with_rotation(rotation)
                            .with_scale(Vec3::new(1.0, 1.0, end - start)),
                    ));
                }
            }
        }
    }

    let landmark_spacing = 80.0;
    let mut landmark_distance = 28.0;
    let mut index = 0;
    while landmark_distance < course_length - 24.0 {
        let landmark = course.point_at(landmark_distance);
        let side = if index % 2 == 0 { 1.0 } else { -1.0 };
        let right = Vec3::new(landmark.yaw.cos(), 0.0, landmark.yaw.sin());
        let rotation = Quat::from_rotation_y(-landmark.yaw);
        commands.spawn((
            WorldAssetRoot(
                asset_server.load(GltfAssetLabel::Scene(0).from_asset("models/riverside_lamp.glb")),
            ),
            Transform::from_xyz(landmark.center_x, 0.0, landmark.center_z)
                .with_translation(
                    Vec3::new(landmark.center_x, 0.0, landmark.center_z) + right * side * 3.4,
                )
                .with_rotation(rotation),
        ));
        commands.spawn((
            WorldAssetRoot(
                asset_server.load(GltfAssetLabel::Scene(0).from_asset("models/riverside_tree.glb")),
            ),
            Transform::from_xyz(landmark.center_x, 0.0, landmark.center_z)
                .with_translation(
                    Vec3::new(landmark.center_x, 0.0, landmark.center_z) - right * side * 4.8,
                )
                .with_rotation(rotation),
        ));
        index += 1;
        landmark_distance += landmark_spacing;
    }

    let bike = commands
        .spawn((
            BikeRoot,
            WorldAssetRoot(
                asset_server.load(GltfAssetLabel::Scene(0).from_asset("models/bicycle.glb")),
            ),
            Transform::from_xyz(0.0, 0.0, BikeState::default().z),
        ))
        .id();
    commands.spawn((
        RiderVisual,
        ChildOf(bike),
        WorldAssetRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset("models/rider.glb"))),
        Transform::from_xyz(0.0, 1.08, 0.23),
    ));
    commands.spawn((
        GunnerVisual,
        ChildOf(bike),
        WorldAssetRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset("models/gunner.glb"))),
        Transform::from_translation(GUNNER_SEAT),
    ));

    for obstacle in OBSTACLES {
        let path = match obstacle.kind {
            ObstacleKind::Cone => "models/traffic_cone.glb",
            ObstacleKind::Bollard => "models/bollard.glb",
            ObstacleKind::Barrier => "models/concrete_barrier.glb",
        };
        let scale = if obstacle.kind == ObstacleKind::Barrier {
            Vec3::new(0.72, 1.0, 0.92)
        } else {
            Vec3::ONE
        };
        commands.spawn((
            WorldAssetRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(path))),
            Transform::from_xyz(obstacle.x, 0.0, obstacle.z).with_scale(scale),
        ));
    }

    commands.spawn((
        HelicopterVisual,
        WorldAssetRoot(
            asset_server.load(GltfAssetLabel::Scene(0).from_asset("models/helicopter.glb")),
        ),
        Transform::from_xyz(0.0, HELICOPTER_ALTITUDE_M, 16.0).with_scale(Vec3::splat(0.82)),
        Visibility::Hidden,
    ));
    commands.spawn((
        RocketVisual,
        WorldAssetRoot(
            asset_server.load(GltfAssetLabel::Scene(0).from_asset("models/missile.glb")),
        ),
        Transform::from_xyz(0.0, 0.0, 0.0).with_scale(Vec3::splat(2.0)),
        Visibility::Hidden,
    ));
    commands.spawn((
        PlayerMissileVisual,
        WorldAssetRoot(
            asset_server.load(GltfAssetLabel::Scene(0).from_asset("models/missile.glb")),
        ),
        Transform::from_xyz(0.0, 0.0, 0.0).with_scale(Vec3::splat(0.82)),
        Visibility::Hidden,
    ));

    let warning_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.98, 0.12, 0.035, 0.34),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    });
    let explosion_material = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.48, 0.08),
        emissive: LinearRgba::rgb(5.0, 1.15, 0.12),
        unlit: true,
        ..default()
    });
    let warning_mesh = meshes.add(Circle::new(1.0));
    commands.spawn((
        DangerZoneVisual,
        Mesh3d(warning_mesh.clone()),
        MeshMaterial3d(warning_material),
        Transform::from_xyz(0.0, 0.075, 0.0)
            .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        Visibility::Hidden,
    ));
    commands.spawn((
        ExplosionVisual,
        Mesh3d(warning_mesh),
        MeshMaterial3d(explosion_material),
        Transform::from_xyz(0.0, 0.09, 0.0)
            .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        Visibility::Hidden,
    ));

    commands.spawn((
        DirectionalLight {
            illuminance: 9_500.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.78, -0.38, 0.0)),
    ));

    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: std::f32::consts::PI / 3.0,
            ..default()
        }),
        FollowCamera,
        Transform::from_xyz(0.0, 3.8, 9.0).looking_at(Vec3::new(0.0, 1.2, 0.0), Vec3::Y),
    ));

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(18),
                top: px(18),
                width: px(450),
                padding: UiRect::all(px(16)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.035, 0.09, 0.085, 0.88)),
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new("RIVERSIDE ESCAPE   /   P4 ROUTE PROTOTYPE"),
                TextFont {
                    font_size: FontSize::Px(19.0),
                    ..default()
                },
                TextColor(Color::srgb(0.70, 0.93, 0.84)),
                HudText,
            ));
        });

    commands.spawn((
        Text::new("+"),
        TextFont {
            font_size: FontSize::Px(26.0),
            ..default()
        },
        TextColor(Color::srgba(0.94, 0.96, 0.87, 0.92)),
        Node {
            position_type: PositionType::Absolute,
            left: percent(50),
            top: percent(50),
            ..default()
        },
        Reticle,
    ));
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: percent(50),
            top: percent(50),
            width: px(120),
            height: px(120),
            margin: UiRect::all(px(-60)),
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        BorderColor::all(Color::srgba(0.96, 0.86, 0.48, 0.8)),
        Visibility::Hidden,
        LockConeRing,
    ));

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: px(18),
                bottom: px(18),
                width: px(460),
                padding: UiRect::all(px(14)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.12, 0.045, 0.025, 0.86)),
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::default(),
                TextFont {
                    font_size: FontSize::Px(20.0),
                    ..default()
                },
                TextColor(Color::srgb(1.0, 0.78, 0.48)),
                CombatHudText,
            ));
        });
}

#[allow(clippy::too_many_arguments)] // Bevy systems expose each resource/query as a separate parameter.
pub fn capture_input(
    mut keyboard: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    mut focus_events: MessageReader<WindowFocused>,
    mut windows: Query<&mut CursorOptions, With<PrimaryWindow>>,
    network: Res<NetworkClient>,
    mut session: ResMut<GameSession>,
) {
    let mut focus_lost = false;
    for event in focus_events.read() {
        focus_lost |= session.focused && !event.focused;
        session.focused = event.focused;
    }
    let browser_focused = browser_has_focus(session.focused);
    focus_lost |= session.focused && !browser_focused;
    session.focused = browser_focused;
    if focus_lost {
        keyboard.reset_all();
        mouse.reset_all();
        session.rider_input = RiderInput::default();
        session.gunner_input = GunnerInput::default();
        session.rider_free_look = false;
        session.cursor_captured = false;
    }
    if !session.focused {
        session.rider_input = RiderInput::default();
        session.gunner_input.aim_held = false;
        session.rider_free_look = false;
        session.cursor_captured = false;
        if let Ok(mut cursor) = windows.single_mut() {
            cursor.grab_mode = CursorGrabMode::None;
            cursor.visible = true;
        }
        return;
    }

    if keyboard.just_pressed(KeyCode::KeyV) {
        let next_seat = match session.local_seat {
            SeatKind::Rider => SeatKind::Gunner,
            SeatKind::Gunner => SeatKind::Rider,
        };
        if network.solo_mode {
            network.send(ClientMessage::SwitchSeat { seat: next_seat });
        } else if !network.connected {
            session.local_seat = next_seat;
        }
    }

    if keyboard.just_pressed(KeyCode::KeyR) && session.phase != GamePhase::Running {
        if network.solo_mode {
            network.send(ClientMessage::ForceStartSolo);
        } else if network.connected {
            session.ready = !session.ready;
            network.send(ClientMessage::SetReady {
                ready: session.ready,
            });
        }
    }
    if keyboard.just_pressed(KeyCode::F3) {
        session.show_diagnostics = !session.show_diagnostics;
    }

    if session.local_seat == SeatKind::Rider {
        let left = keyboard.pressed(KeyCode::KeyA);
        let right = keyboard.pressed(KeyCode::KeyD);
        session.rider_input = RiderInput {
            pedal_held: mouse.pressed(MouseButton::Left),
            brake_held: keyboard.pressed(KeyCode::Space),
            steer: (if right { 1.0 } else { 0.0 }) - (if left { 1.0 } else { 0.0 }),
        };

        if mouse.just_pressed(MouseButton::Right) {
            session.rider_free_look = true;
            session.rider_camera_yaw = session.bike.yaw;
            session.rider_camera_pitch = RIDER_CAMERA_PITCH;
        }
        if session.rider_free_look && mouse.pressed(MouseButton::Right) {
            session.rider_camera_yaw -= motion.delta.x * MOUSE_SENSITIVITY;
            session.rider_camera_pitch = (session.rider_camera_pitch
                - motion.delta.y * MOUSE_SENSITIVITY)
                .clamp(-0.72, 0.82);
        }
        if mouse.just_released(MouseButton::Right) {
            session.rider_free_look = false;
        }
        session.cursor_captured = mouse.pressed(MouseButton::Right);
    } else {
        session.rider_input = RiderInput::default();
        session.gunner_input.yaw = (session.gunner_input.yaw - motion.delta.x * MOUSE_SENSITIVITY
            + std::f32::consts::PI)
            .rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        session.gunner_input.pitch = (session.gunner_input.pitch
            - motion.delta.y * MOUSE_SENSITIVITY)
            .clamp(GUNNER_PITCH_MIN_RADIANS, GUNNER_PITCH_MAX_RADIANS);
        session.gunner_input.aim_held = mouse.pressed(MouseButton::Right);
        if mouse.just_pressed(MouseButton::Left) || mouse.just_pressed(MouseButton::Right) {
            session.cursor_captured = true;
        }
        if mouse.just_pressed(MouseButton::Left)
            && session.phase == GamePhase::Running
            && network.connected
        {
            // A fire request is validated against the most recently received
            // aim input. Send this frame's aim first so one-way network delay
            // cannot make a valid trigger pull race a stale 20 Hz input sample.
            session.input_sequence = session.input_sequence.wrapping_add(1).max(1);
            network.send(ClientMessage::GunnerInput {
                sequence: session.input_sequence,
                tick: session.server_tick,
                input: session.gunner_input,
            });
            session.fire_request_sequence = session.fire_request_sequence.saturating_add(1).max(1);
            network.send(ClientMessage::FireRequest {
                request_id: session.fire_request_sequence,
                tick: session.server_tick,
            });
        }
        if keyboard.just_pressed(KeyCode::Escape) {
            session.cursor_captured = false;
        }
    }

    if let Ok(mut cursor) = windows.single_mut() {
        let capture = session.cursor_captured;
        cursor.grab_mode = if capture {
            CursorGrabMode::Locked
        } else {
            CursorGrabMode::None
        };
        cursor.visible = !capture;
    }
}

fn browser_has_focus(fallback: bool) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.has_focus().ok())
            .unwrap_or(fallback)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        fallback
    }
}

pub fn simulate_bike(
    time: Res<Time>,
    network: Res<NetworkClient>,
    mut session: ResMut<GameSession>,
) {
    session.collision_flash = (session.collision_flash - time.delta_secs()).max(0.0);
    if session.phase != GamePhase::Running {
        return;
    }
    let delta_secs = time.delta_secs().max(0.0);
    session.max_update_delta_ms = session.max_update_delta_ms.max(delta_secs * 1000.0);
    session.prediction_accumulator += f64::from(delta_secs);

    while session.prediction_accumulator + 1.0e-10 >= FIXED_DT_SECONDS {
        session.prediction_accumulator =
            (session.prediction_accumulator - FIXED_DT_SECONDS).max(0.0);
        if session.local_seat == SeatKind::Rider || !network.connected {
            if !session.prediction_initialized {
                let tick = session.client_tick;
                let bike = session.bike;
                session.prediction.reset(tick, bike);
                session.prediction_initialized = true;
            }
            let input = session.rider_input;
            if session.prediction.step(input) {
                session.collision_flash = 0.28;
            }
            session.client_tick = session.prediction.current_tick();
        } else {
            session.client_tick = session.client_tick.saturating_add(1);
        }
    }

    while let Some(snapshot) = session.pending_authoritative_snapshots.pop_front() {
        session.receive_running_snapshot(
            snapshot.tick,
            snapshot.bike,
            snapshot.last_applied_rider_sequence,
        );
    }

    if network.connected {
        session.input_send_accumulator += f64::from(delta_secs);
        while session.input_send_accumulator + 1.0e-10 >= INPUT_SEND_INTERVAL_SECS {
            session.input_send_accumulator -= INPUT_SEND_INTERVAL_SECS;
            session.input_sequence = session.input_sequence.wrapping_add(1).max(1);
            let sequence = session.input_sequence;
            let tick = session.client_tick;
            match session.local_seat {
                SeatKind::Rider => {
                    let input = session.rider_input;
                    session.pending_rider_inputs.push_back(RiderInputSample {
                        sequence,
                        tick,
                        input,
                    });
                    session.max_pending_rider_inputs = session
                        .max_pending_rider_inputs
                        .max(session.pending_rider_inputs.len());
                    network.send(ClientMessage::RiderInput {
                        sequence,
                        tick,
                        input,
                    });
                }
                SeatKind::Gunner => network.send(ClientMessage::GunnerInput {
                    sequence,
                    tick,
                    input: session.gunner_input,
                }),
            }
        }
    }

    if network.connected && session.local_seat == SeatKind::Gunner {
        if !session.snapshot_received_this_frame {
            session
                .snapshot_interpolator
                .advance_time(f64::from(delta_secs));
        }
        if let Some(bike) = session.snapshot_interpolator.sample() {
            session.bike = bike;
        }
        session.gunner_interpolation_lag_ms = session
            .snapshot_interpolator
            .estimated_lag_secs()
            .unwrap_or_default() as f32
            * 1000.0;
    } else {
        let previous_offset = session.render_offset.advance(delta_secs);
        let predicted = session.prediction.bike();
        let before_correction = previous_offset.apply(predicted);
        let after_correction = session.render_offset.apply(predicted);
        session.reconcile_bike_step_m = (before_correction.x - after_correction.x)
            .hypot(before_correction.z - after_correction.z);
        session.max_reconcile_bike_step_m = session
            .max_reconcile_bike_step_m
            .max(session.reconcile_bike_step_m);
        session.bike = after_correction;
        if network.connected {
            session.snapshot_gap_m = (predicted.x - session.authoritative_bike.x)
                .hypot(predicted.z - session.authoritative_bike.z);
            session.max_snapshot_gap_m = session.max_snapshot_gap_m.max(session.snapshot_gap_m);
        }
    }
    session.snapshot_received_this_frame = false;
}

pub(super) fn apply_world_transforms(
    time: Res<Time>,
    mut session: ResMut<GameSession>,
    mut bikes: Query<&mut Transform, With<BikeRoot>>,
    mut gunners: Query<&mut Transform, (With<GunnerVisual>, Without<BikeRoot>)>,
    mut gunner_visibility: Query<&mut Visibility, With<GunnerVisual>>,
) {
    let Ok(mut transform) = bikes.single_mut() else {
        return;
    };
    transform.translation.x = session.bike.x;
    transform.translation.z = session.bike.z;
    transform.rotation = Quat::from_rotation_y(session.bike.yaw);

    if let Ok(mut gunner) = gunners.single_mut() {
        gunner.translation = GUNNER_SEAT;
        gunner.rotation = Quat::from_rotation_y(GUNNER_HOME_YAW + session.gunner_input.yaw);
    }
    if let Ok(mut visibility) = gunner_visibility.single_mut() {
        *visibility = if session.local_seat == SeatKind::Gunner {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
    }
    if session.rider_free_look {
        return;
    }
    session.rider_camera_yaw = approach_angle(
        session.rider_camera_yaw,
        session.bike.yaw,
        (time.delta_secs() * CAMERA_RETURN_SPEED).clamp(0.0, 1.0),
    );
    session.rider_camera_pitch += (RIDER_CAMERA_PITCH - session.rider_camera_pitch)
        * (time.delta_secs() * CAMERA_RETURN_SPEED).clamp(0.0, 1.0);
}

pub(super) fn apply_steering_visual(
    session: Res<GameSession>,
    mut steering_nodes: Query<(&Name, &mut Transform)>,
) {
    for (name, mut transform) in &mut steering_nodes {
        if name.as_str().ends_with(".Steering") {
            transform.rotation = Quat::from_rotation_y(session.bike.steering_angle);
        }
    }
}

#[allow(clippy::too_many_arguments)] // Bevy systems expose each visual query separately.
pub(super) fn update_combat_visuals(
    time: Res<Time>,
    mut session: ResMut<GameSession>,
    mut helicopters: Query<(&mut Transform, &mut Visibility), HelicopterVisualFilter>,
    mut rockets: Query<(&mut Transform, &mut Visibility), RocketVisualFilter>,
    mut player_missiles: Query<(&mut Transform, &mut Visibility), PlayerMissileVisualFilter>,
    mut danger_zones: Query<(&mut Transform, &mut Visibility), DangerZoneVisualFilter>,
    mut explosions: Query<(&mut Transform, &mut Visibility), ExplosionVisualFilter>,
    mut combat_hud: Query<&mut Text, With<CombatHudText>>,
) {
    session.combat_notice_secs = (session.combat_notice_secs - time.delta_secs()).max(0.0);

    if let Ok((mut transform, mut visibility)) = helicopters.single_mut() {
        if let Some(helicopter) = session
            .helicopter
            .filter(|_| session.phase == GamePhase::Running)
        {
            transform.translation = world_position(helicopter.position);
            transform.rotation = Quat::from_rotation_y(helicopter.yaw);
            *visibility = Visibility::Visible;
        } else {
            *visibility = Visibility::Hidden;
        }
    }

    if let Ok((mut transform, mut visibility)) = rockets.single_mut() {
        if let Some(rocket) = session
            .rocket
            .filter(|_| session.phase == GamePhase::Running)
        {
            let direction =
                world_position(rocket.impact_position) - world_position(rocket.launch_position);
            let mut rocket_transform = Transform::from_translation(world_position(rocket.position))
                .with_scale(Vec3::splat(2.0));
            if direction.length_squared() > f32::EPSILON {
                rocket_transform = rocket_transform.looking_to(direction.normalize(), Vec3::Y);
            }
            *transform = rocket_transform;
            *visibility = Visibility::Visible;
        } else {
            *visibility = Visibility::Hidden;
        }
    }

    if let Ok((mut transform, mut visibility)) = player_missiles.single_mut() {
        if let Some(missile) = session
            .player_missile
            .filter(|_| session.phase == GamePhase::Running)
        {
            let mut missile_transform =
                Transform::from_translation(world_position(missile.position))
                    .with_scale(Vec3::splat(0.82));
            if let Some(helicopter) = session.helicopter {
                let direction = world_position(helicopter.position) - missile_transform.translation;
                if direction.length_squared() > f32::EPSILON {
                    missile_transform =
                        missile_transform.looking_to(direction.normalize(), Vec3::Y);
                }
            }
            *transform = missile_transform;
            *visibility = Visibility::Visible;
        } else {
            *visibility = Visibility::Hidden;
        }
    }

    let warning = session
        .helicopter
        .filter(|_| session.phase == GamePhase::Running)
        .and_then(|helicopter| helicopter.warning);
    if let Ok((mut transform, mut visibility)) = danger_zones.single_mut() {
        if let Some(warning) = warning {
            *transform =
                Transform::from_xyz(warning.impact_position.x, 0.075, warning.impact_position.z)
                    .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
                    .with_scale(Vec3::new(
                        warning.blast_radius_m,
                        warning.blast_radius_m,
                        1.0,
                    ));
            *visibility = Visibility::Visible;
        } else {
            *visibility = Visibility::Hidden;
        }
    }

    let explosion_visible = session.phase == GamePhase::Running
        && session.combat_notice_secs > 0.0
        && session.explosion_position.is_some();
    if let Ok((mut transform, mut visibility)) = explosions.single_mut() {
        if let Some(position) = session.explosion_position.filter(|_| explosion_visible) {
            *transform = Transform::from_xyz(position.x, 0.09, position.z)
                .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
                .with_scale(Vec3::splat(ROCKET_BLAST_RADIUS_M));
            *visibility = Visibility::Visible;
        } else {
            *visibility = Visibility::Hidden;
        }
    }

    if let Ok(mut text) = combat_hud.single_mut() {
        text.0 = combat_status(&session);
    }
}

fn combat_status(session: &GameSession) -> String {
    if session.combat_notice_secs > 0.0
        && let Some(event) = session.last_combat_event
    {
        match event.kind {
            CombatEventKind::RocketExploded { hit, hit_count } => {
                return if hit {
                    format!("ROCKET HIT  |  Confirmed hits: {hit_count}")
                } else {
                    format!("ROCKET MISSED  |  Confirmed hits: {hit_count}")
                };
            }
            CombatEventKind::BikeDamaged { health_remaining } => {
                return format!("BIKE HIT  |  Health: {health_remaining}/100");
            }
            CombatEventKind::GunnerFireAccepted { ammo_remaining, .. } => {
                return format!("MISSILE AWAY  |  Ammo: {ammo_remaining}");
            }
            CombatEventKind::GunnerFireRejected { reason, .. } => {
                return format!("CANNOT FIRE  |  {}", fire_rejection_label(reason));
            }
            CombatEventKind::PlayerMissileImpacted {
                hit,
                helicopter_health,
            } => {
                return if hit {
                    format!("MISSILE HIT  |  Helicopter: {helicopter_health}/100")
                } else {
                    "MISSILE MISSED  |  Target not reached".to_owned()
                };
            }
            CombatEventKind::MatchFinished { result } => {
                return match result {
                    MatchResult::RidersVictory => "VICTORY  |  Helicopter destroyed".into(),
                    MatchResult::RidersDefeat => "DEFEAT  |  Bike disabled".into(),
                };
            }
            _ => {}
        }
    }
    if session.phase == GamePhase::Finished {
        return match session.match_result {
            Some(MatchResult::RidersVictory) => "VICTORY  |  Press R to ready again".into(),
            Some(MatchResult::RidersDefeat) => "DEFEAT  |  Press R to ready again".into(),
            None => String::new(),
        };
    }
    if session.phase != GamePhase::Running {
        return String::new();
    }

    if let Some(helicopter) = session.helicopter {
        if let Some(warning) = helicopter.warning {
            let seconds_left = warning.impact_tick.saturating_sub(session.server_tick) as f32
                / SIMULATION_HZ as f32;
            return match session.local_seat {
                SeatKind::Rider => format!(
                    "ROCKET WARNING  |  Fixed blast zone shown  |  Impact in {seconds_left:.1}s  |  Hits {}",
                    session.rocket_hit_count,
                ),
                SeatKind::Gunner => format!(
                    "INCOMING ROCKET: {}  |  Impact in {seconds_left:.1}s  |  Hits {}",
                    threat_direction(session, helicopter),
                    session.rocket_hit_count,
                ),
            };
        }
        if session.rocket.is_some() {
            return format!(
                "ROCKET IN FLIGHT: {}  |  Confirmed hits: {}",
                threat_direction(session, helicopter),
                session.rocket_hit_count,
            );
        }
        if session.player_missile.is_some() {
            return format!(
                "PLAYER MISSILE IN FLIGHT  |  Heli {} HP",
                session.helicopter_health,
            );
        }
        return format!(
            "HELICOPTER {:?}  |  Confirmed rocket hits: {}",
            helicopter.phase, session.rocket_hit_count,
        );
    }

    String::new()
}

fn fire_rejection_label(reason: FireRejectionReason) -> &'static str {
    match reason {
        FireRejectionReason::NotRunning => "round is not active",
        FireRejectionReason::StaleInput => "aim input expired",
        FireRejectionReason::NotAiming => "hold right mouse to aim",
        FireRejectionReason::NoTarget => "helicopter unavailable",
        FireRejectionReason::TargetOutsideCone => "helicopter outside the lock ring",
        FireRejectionReason::OutOfRange => "target beyond 120 m",
        FireRejectionReason::Obstructed => "target is obstructed",
        FireRejectionReason::NotLocked => "keep the target in the ring for 2 s",
        FireRejectionReason::Cooldown => "launcher cooling down",
        FireRejectionReason::NoAmmo => "no missiles remaining",
        FireRejectionReason::MissileInFlight => "missile still in flight",
    }
}

fn threat_direction(session: &GameSession, helicopter: HelicopterSnapshot) -> &'static str {
    let camera_yaw = match session.local_seat {
        SeatKind::Rider => session.rider_camera_yaw,
        SeatKind::Gunner => session.bike.yaw + GUNNER_HOME_YAW + session.gunner_input.yaw,
    };
    let delta_x = helicopter.position.x - session.bike.x;
    let delta_z = helicopter.position.z - session.bike.z;
    let target_yaw = delta_x.atan2(-delta_z);
    let relative = shortest_angle_delta(camera_yaw, target_yaw);
    if relative.abs() < 0.4 {
        "CENTER"
    } else if relative.abs() > 2.5 {
        "BEHIND"
    } else if relative > 0.0 {
        "RIGHT"
    } else {
        "LEFT"
    }
}

fn world_position(position: WorldPosition) -> Vec3 {
    Vec3::new(position.x, position.y, position.z)
}

pub(super) fn update_camera(
    session: Res<GameSession>,
    mut cameras: Query<&mut Transform, With<FollowCamera>>,
) {
    let Ok(mut camera) = cameras.single_mut() else {
        return;
    };
    let bike_position = Vec3::new(session.bike.x, 0.0, session.bike.z);
    match session.local_seat {
        SeatKind::Rider => {
            let camera_yaw = Quat::from_rotation_y(session.rider_camera_yaw);
            let eye = bike_position + camera_yaw * Vec3::new(0.0, 3.4, 7.4);
            let direction =
                camera_yaw * Quat::from_rotation_x(session.rider_camera_pitch) * Vec3::NEG_Z;
            *camera = Transform::from_translation(eye).looking_to(direction, Vec3::Y);
        }
        SeatKind::Gunner => {
            *camera = gunner_camera_transform(session.bike, session.gunner_input);
        }
    }
}

fn gunner_camera_transform(bike: BikeState, input: GunnerInput) -> Transform {
    let bike_position = Vec3::new(bike.x, 0.0, bike.z);
    let bike_rotation = Quat::from_rotation_y(bike.yaw);
    let chair_rotation = Quat::from_rotation_y(GUNNER_HOME_YAW + input.yaw);
    let eye = bike_position
        + bike_rotation * GUNNER_SEAT
        + bike_rotation * (chair_rotation * GUNNER_CAMERA_AIM);
    Transform {
        translation: eye,
        rotation: bike_rotation * chair_rotation * Quat::from_rotation_x(input.pitch),
        ..default()
    }
}

pub(super) fn update_hud(
    time: Res<Time>,
    mut session: ResMut<GameSession>,
    network: Res<NetworkClient>,
    mut hud: Query<&mut Text, (With<HudText>, Without<LockConeRing>)>,
    mut reticle: Query<&mut Visibility, (With<Reticle>, Without<LockConeRing>)>,
    mut lock_ring: Query<(&mut Node, &mut BorderColor, &mut Visibility), LockConeRingFilter>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    update_predicted_lock(&mut session, time.delta_secs());
    let status = if network.solo_unavailable {
        "Solo mode disabled | start server with RIVERSIDE_DEV_SOLO=1".to_owned()
    } else if network.room_full {
        "Room full | close one of the other clients".to_owned()
    } else if !session.is_connected {
        "Local preview | server not connected".to_owned()
    } else {
        match session.phase {
            GamePhase::WaitingForPlayers => "Waiting for the other seat".to_owned(),
            GamePhase::ReadyCheck if session.solo_mode => {
                "Solo mode | press R to ready both seats and start".to_owned()
            }
            GamePhase::ReadyCheck => "Ready check | press R when prepared".to_owned(),
            GamePhase::Countdown => {
                format!("Starting in {}...", session.countdown_seconds.unwrap_or(5))
            }
            GamePhase::Running => "Keep moving together".to_owned(),
            GamePhase::Finished => match session.match_result {
                Some(MatchResult::RidersVictory) => {
                    "VICTORY | both players press R to replay".into()
                }
                Some(MatchResult::RidersDefeat) => "DEFEAT | both players press R to replay".into(),
                None => "Finished | press R to ready for another round".into(),
            },
        }
    };
    let players = format!(
        "Rider {}  |  Gunner {}",
        slot_label(session.seats.get(&SeatKind::Rider)),
        slot_label(session.seats.get(&SeatKind::Gunner)),
    );
    let controls = match (session.local_seat, session.solo_mode, session.is_connected) {
        (SeatKind::Rider, true, _) => {
            "LMB pedal · Space brake · A/D steer\nR ready both · V switch role · F3 diagnostics"
        }
        (SeatKind::Gunner, true, _) => {
            "Mouse aim · hold RMB to lock · LMB fire\nR ready both · V switch role · F3 diagnostics"
        }
        (SeatKind::Rider, false, false) => {
            "LMB pedal · Space brake · A/D steer\nRMB free look · F3 diagnostics"
        }
        (SeatKind::Rider, false, true) => {
            "LMB pedal · Space brake · A/D steer\nRMB free look · R ready · F3 diagnostics"
        }
        (SeatKind::Gunner, false, _) => {
            "Mouse aim · hold RMB to lock · LMB fire\nR ready · Esc release pointer · F3 diagnostics"
        }
    };
    let lock_progress = session.gunner_lock_progress_ticks as f32 / GUNNER_LOCK_TICKS as f32;
    let predicted_progress =
        session.predicted_lock_progress_ticks as f32 / GUNNER_LOCK_TICKS as f32;
    let combat = if session.local_seat == SeatKind::Gunner {
        let server_lock = if session.gunner_locked {
            "LOCKED"
        } else if session.gunner_lock_grace_ticks_remaining > 0 {
            "PAUSED"
        } else {
            "locking"
        };
        format!(
            "Bike {:>3}/100  Heli {:>3}/100\nAmmo {}/{} · Server {server_lock} ({:.0}%) · {}",
            session.bike_health,
            session.helicopter_health,
            session.gunner_ammo,
            GUNNER_STARTING_AMMO,
            lock_progress * 100.0,
            if session.gunner_can_fire {
                "READY TO FIRE".to_owned()
            } else if session.fire_cooldown_ticks_remaining > 0 {
                format!(
                    "cooldown {:.1}s",
                    session.fire_cooldown_ticks_remaining as f32 / SIMULATION_HZ as f32
                )
            } else {
                format!("local aim estimate {:.0}%", predicted_progress * 100.0)
            },
        )
    } else {
        format!(
            "Bike {:>3}/100  Heli {:>3}/100\nConfirmed rocket hits {}",
            session.bike_health, session.helicopter_health, session.rocket_hit_count
        )
    };
    let collision = if session.collision_flash > 0.0 {
        "\nObstacle hit - steer around it"
    } else {
        ""
    };
    let diagnostics = if session.show_diagnostics && session.is_connected {
        format!(
            "\n\n[F3] Server {} / Client {} · Rider ack {} · pending {} (max {})\
             \nSame-tick error {:.3} m / {:.2}° (max {:.3} m / {:.2}°)\
             \nReplayed {} ticks ({} unacked) · history misses {}\
             \nPred↔snapshot {:.2} m (max {:.2}; ticks differ)\
             \nRender correction/frame {:.3} m (max {:.3}) · Gunner lag {:.1} ms\
             \nMax app frame {:.1} ms · predicted lock {:.0}%",
            session.server_tick,
            session.client_tick,
            session.last_applied_rider_sequence,
            session.pending_rider_inputs.len(),
            session.max_pending_rider_inputs,
            session.same_tick_error_m,
            session.same_tick_yaw_error_deg,
            session.max_same_tick_error_m,
            session.max_same_tick_yaw_error_deg,
            session.replayed_ticks_last_snapshot,
            session.replayed_unacknowledged_inputs_last_snapshot,
            session.history_miss_count,
            session.snapshot_gap_m,
            session.max_snapshot_gap_m,
            session.reconcile_bike_step_m,
            session.max_reconcile_bike_step_m,
            session.gunner_interpolation_lag_ms,
            session.max_update_delta_ms,
            predicted_progress * 100.0,
        )
    } else {
        String::new()
    };
    if let Ok(mut text) = hud.single_mut() {
        text.0 = format!(
            "RIVERSIDE ESCAPE  /  P4\n{}  |  {}\n{}\nSpeed {:>4.1} m/s\n{}\n{}{}{}",
            session.local_seat.label(),
            status,
            players,
            session.bike.speed,
            combat,
            controls,
            collision,
            diagnostics,
        );
    }
    if let Ok(mut visibility) = reticle.single_mut() {
        *visibility = if session.local_seat == SeatKind::Gunner {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    if let Ok((mut node, mut border_color, mut visibility)) = lock_ring.single_mut() {
        let (radius, focused) = windows.single().map_or((60.0, false), |window| {
            let half_height = window.height() * 0.5;
            let cone_radius = half_height
                * (GUNNER_LOCK_CONE_HALF_ANGLE_DEGREES.to_radians().tan())
                / (std::f32::consts::PI / 6.0).tan();
            (
                cone_radius.clamp(36.0, 120.0),
                session.local_seat == SeatKind::Gunner,
            )
        });
        node.width = px(radius * 2.0);
        node.height = px(radius * 2.0);
        node.margin = UiRect::all(px(-radius));
        if session.gunner_locked {
            *border_color = BorderColor::all(Color::srgb(0.35, 1.0, 0.54));
        } else if session.predicted_lock_progress_ticks > 0 {
            *border_color = BorderColor::all(Color::srgb(1.0, 0.78, 0.30));
        } else {
            *border_color = BorderColor::all(Color::srgba(0.96, 0.86, 0.48, 0.8));
        }
        *visibility = if focused && session.phase == GamePhase::Running {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

fn update_predicted_lock(session: &mut GameSession, delta_secs: f32) {
    if session.local_seat != SeatKind::Gunner
        || session.phase != GamePhase::Running
        || !session.gunner_input.aim_held
        || session.helicopter_health == 0
    {
        session.predicted_lock_progress_ticks = 0;
        session.predicted_lock_outside_secs = 0.0;
        return;
    }
    let Some(helicopter) = session.helicopter else {
        session.predicted_lock_progress_ticks = 0;
        session.predicted_lock_outside_secs = 0.0;
        return;
    };
    let aim = evaluate_gunner_aim(session.bike, session.gunner_input, helicopter.position);
    if !aim.in_range || aim.obstructed {
        session.predicted_lock_progress_ticks = 0;
        session.predicted_lock_outside_secs = 0.0;
    } else if aim.in_cone {
        session.predicted_lock_outside_secs = 0.0;
        let added_ticks = (delta_secs.max(0.0) * SIMULATION_HZ as f32).ceil() as u16;
        session.predicted_lock_progress_ticks = session
            .predicted_lock_progress_ticks
            .saturating_add(added_ticks)
            .min(GUNNER_LOCK_TICKS as u16);
    } else if session.predicted_lock_progress_ticks > 0 {
        session.predicted_lock_outside_secs += delta_secs.max(0.0);
        if session.predicted_lock_outside_secs > 0.2 {
            session.predicted_lock_progress_ticks = 0;
            session.predicted_lock_outside_secs = 0.0;
        }
    }
}

fn slot_label(status: Option<&SeatStatus>) -> &'static str {
    let Some(status) = status else {
        return "not required";
    };
    if !status.connected {
        "open"
    } else if status.ready {
        "ready"
    } else {
        "connected"
    }
}

fn approach_angle(current: f32, target: f32, amount: f32) -> f32 {
    current + shortest_angle_delta(current, target) * amount
}

fn shortest_angle_delta(current: f32, target: f32) -> f32 {
    (target - current + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
        - std::f32::consts::PI
}

#[cfg(test)]
mod tests {
    use super::{GameSession, update_combat_visuals, update_hud};
    use bevy::prelude::{App, Time, Update};
    use riverside_common::{CombatEvent, CombatEventKind, WorldPosition};

    use crate::network::NetworkClient;

    #[test]
    fn combat_visual_system_has_no_conflicting_queries_at_startup() {
        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(GameSession::default());
        app.add_systems(Update, update_combat_visuals);
        app.update();
    }

    #[test]
    fn aim_hud_system_has_no_conflicting_queries_at_startup() {
        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(GameSession::default());
        app.insert_resource(NetworkClient::default());
        app.add_systems(Update, update_hud);
        app.update();
    }

    #[test]
    fn repeated_snapshot_event_id_only_triggers_one_client_effect() {
        let event = CombatEvent {
            event_id: 17,
            attack_id: 4,
            tick: 350,
            position: WorldPosition {
                x: 0.5,
                y: 0.0,
                z: -12.0,
            },
            kind: CombatEventKind::RocketExploded {
                hit: true,
                hit_count: 1,
            },
        };
        let mut session = GameSession::default();
        assert!(session.observe_combat_event(event));
        session.combat_notice_secs = 0.4;
        assert!(!session.observe_combat_event(event));
        assert_eq!(session.combat_event_order.len(), 1);
        assert_eq!(session.combat_notice_secs, 0.4);
    }
}
