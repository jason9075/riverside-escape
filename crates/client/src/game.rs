use std::collections::{BTreeMap, BTreeSet, VecDeque};

use bevy::{
    gltf::GltfAssetLabel,
    input::mouse::AccumulatedMouseMotion,
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow, WindowFocused},
    world_serialization::WorldAssetRoot,
};
use riverside_common::{
    BikeRenderOffset, BikeState, ClientMessage, CombatEvent, CombatEventKind, FIXED_DT_SECONDS,
    GamePhase, GunnerInput, HELICOPTER_ALTITUDE_M, HelicopterSnapshot, OBSTACLES, ObstacleKind,
    REQUIRED_SEATS, ROCKET_BLAST_RADIUS_M, RiderInput, RiderInputSample, RiderPrediction,
    RocketSnapshot, SIMULATION_HZ, SeatKind, SeatStatus, SnapshotInterpolator, WorldPosition,
    WorldSnapshot,
};

use crate::network::NetworkClient;

const CAMERA_RETURN_SPEED: f32 = 6.0;
const MOUSE_SENSITIVITY: f32 = 0.004;
const GUNNER_YAW_LIMIT: f32 = 1.92;
const GUNNER_PITCH_MIN: f32 = -0.55;
const GUNNER_PITCH_MAX: f32 = 0.45;
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
    pub rocket_hit_count: u32,
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
    input_sequence: u32,
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
            rocket_hit_count: 0,
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
            input_sequence: 0,
        }
    }
}

impl GameSession {
    pub fn change_seat(&mut self, seat: SeatKind) {
        self.local_seat = seat;
        self.pending_rider_inputs.clear();
        self.pending_authoritative_snapshots.clear();
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
        if snapshot.phase != GamePhase::Running {
            self.helicopter = None;
            self.rocket = None;
            self.rocket_hit_count = 0;
            self.combat_notice_secs = 0.0;
            self.last_combat_event = None;
            self.explosion_position = None;
            return;
        }

        self.helicopter = snapshot.helicopter;
        self.rocket = snapshot.rocket;
        self.rocket_hit_count = snapshot.rocket_hit_count;
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
        if matches!(event.kind, CombatEventKind::RocketExploded { .. }) {
            self.explosion_position = Some(event.position);
            self.combat_notice_secs = 1.25;
            self.last_combat_event = Some(event);
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

    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(400.0, 400.0))),
        MeshMaterial3d(grass),
        Transform::from_xyz(0.0, -0.23, -35.0),
    ));

    // Extend the road behind the starting position as well, so the Gunner's
    // rear-facing first-person view still has a path and landmarks to see.
    for segment in -6..15 {
        let z = -(segment as f32) * 8.0;
        commands.spawn((
            WorldAssetRoot(
                asset_server.load(GltfAssetLabel::Scene(0).from_asset("models/road_straight.glb")),
            ),
            Transform::from_xyz(0.0, 0.0, z),
        ));

        for side in [-1.0, 1.0] {
            commands.spawn((
                Mesh3d(meshes.add(Cuboid::new(0.12, 0.44, 8.0))),
                MeshMaterial3d(rail_material.clone()),
                Transform::from_xyz(side * 2.55, 0.34, z - 4.0),
            ));
            commands.spawn((
                Mesh3d(meshes.add(Cuboid::new(0.055, 0.018, 8.0))),
                MeshMaterial3d(lane_material.clone()),
                Transform::from_xyz(side * 1.58, 0.015, z - 4.0),
            ));
        }
    }

    for (index, z) in [-13.0, -39.0, -69.0, -95.0, 13.0, 39.0]
        .into_iter()
        .enumerate()
    {
        let side = if index % 2 == 0 { 1.0 } else { -1.0 };
        commands.spawn((
            WorldAssetRoot(
                asset_server.load(GltfAssetLabel::Scene(0).from_asset("models/riverside_lamp.glb")),
            ),
            Transform::from_xyz(side * 3.4, 0.0, z),
        ));
        commands.spawn((
            WorldAssetRoot(
                asset_server.load(GltfAssetLabel::Scene(0).from_asset("models/riverside_tree.glb")),
            ),
            Transform::from_xyz(-side * 4.8, 0.0, z - 5.0),
        ));
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
                Text::new("RIVERSIDE ESCAPE   /   P2 ROCKET PROTOTYPE"),
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
        session.gunner_input.yaw = (session.gunner_input.yaw - motion.delta.x * MOUSE_SENSITIVITY)
            .clamp(-GUNNER_YAW_LIMIT, GUNNER_YAW_LIMIT);
        session.gunner_input.pitch = (session.gunner_input.pitch
            - motion.delta.y * MOUSE_SENSITIVITY)
            .clamp(GUNNER_PITCH_MIN, GUNNER_PITCH_MAX);
        session.gunner_input.aim_held = mouse.pressed(MouseButton::Right);
        if mouse.just_pressed(MouseButton::Left) || mouse.just_pressed(MouseButton::Right) {
            session.cursor_captured = true;
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
    if session.phase != GamePhase::Running {
        return String::new();
    }

    if session.combat_notice_secs > 0.0
        && let Some(event) = session.last_combat_event
        && let CombatEventKind::RocketExploded { hit, hit_count } = event.kind
    {
        return if hit {
            format!("ROCKET HIT  |  Confirmed hits: {hit_count}")
        } else {
            format!("ROCKET MISSED  |  Confirmed hits: {hit_count}")
        };
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
        return format!(
            "HELICOPTER {:?}  |  Confirmed rocket hits: {}",
            helicopter.phase, session.rocket_hit_count,
        );
    }

    String::new()
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
    session: Res<GameSession>,
    network: Res<NetworkClient>,
    mut hud: Query<&mut Text, With<HudText>>,
    mut reticle: Query<&mut Visibility, With<Reticle>>,
) {
    if let Ok(mut text) = hud.single_mut() {
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
                GamePhase::Running => "Ride!  Keep moving together".to_owned(),
            }
        };
        let players = format!(
            "Room  Rider {}  |  Gunner {}\n",
            slot_label(session.seats.get(&SeatKind::Rider)),
            slot_label(session.seats.get(&SeatKind::Gunner)),
        );
        let controls = match (session.local_seat, session.solo_mode, session.is_connected) {
            (SeatKind::Rider, true, _) => {
                "LEFT MOUSE  pedal     SPACE  brake\nA / D  steer (temporary)     RIGHT MOUSE  free look\nR  force-ready both seats     V  switch role"
            }
            (SeatKind::Gunner, true, _) => {
                "MOUSE  turn the rear seat     RIGHT MOUSE  aim\nR  force-ready both seats     ESC  release pointer     V  switch role"
            }
            (SeatKind::Rider, false, false) => {
                "LEFT MOUSE  pedal     SPACE  brake\nA / D  steer (temporary)     RIGHT MOUSE  free look\nV  local camera preview"
            }
            (SeatKind::Rider, false, true) => {
                "LEFT MOUSE  pedal     SPACE  brake\nA / D  steer (temporary)     RIGHT MOUSE  free look\nR  toggle Ready     V  solo mode only"
            }
            (SeatKind::Gunner, false, _) => {
                "MOUSE  turn the rear seat     RIGHT MOUSE  aim\nR  toggle Ready     ESC  release pointer     V  solo mode only"
            }
        };
        let collision = if session.collision_flash > 0.0 {
            "\nObstacle hit - steer around it"
        } else {
            ""
        };
        let correction = if session.is_connected {
            format!(
                "\nServer tick {} | Client tick {} | Rider ack {} | pending {} (max {})\
                 \nSame-tick error {:.3} m / {:.2}° (max {:.3} m / {:.2}°)\
                 \nReplayed {} ticks ({} unacked samples) | history misses {}\
                 \nPred↔snapshot {:.2} m (max {:.2}; ticks differ)\
                 \nRender correction/frame {:.3} m (max {:.3}); Gunner interp lag {:.1} ms\
                 \nMax app frame Δ {:.1} ms",
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
            )
        } else {
            String::new()
        };
        text.0 = format!(
            "RIVERSIDE ESCAPE  /  P1 + P2\n{}  |  {}\n{}Speed  {:>4.1} m/s\n\n{}{}{}",
            session.local_seat.label(),
            status,
            players,
            session.bike.speed,
            controls,
            collision,
            correction,
        );
    }
    if let Ok(mut visibility) = reticle.single_mut() {
        *visibility = if session.local_seat == SeatKind::Gunner {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
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
    use super::{GameSession, update_combat_visuals};
    use bevy::prelude::{App, Time, Update};
    use riverside_common::{CombatEvent, CombatEventKind, WorldPosition};

    #[test]
    fn combat_visual_system_has_no_conflicting_queries_at_startup() {
        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(GameSession::default());
        app.add_systems(Update, update_combat_visuals);
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
