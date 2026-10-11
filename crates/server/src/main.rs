use std::{
    collections::{BTreeMap, VecDeque},
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Router,
    extract::{OriginalUri, State, ws::WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
};
use bevy_app::{App, ScheduleRunnerPlugin, Update};
use bevy_ecs::prelude::*;
use futures_util::{SinkExt, StreamExt};
use riverside_common::{
    BikeState, COMBAT_EVENT_RETENTION_TICKS, COUNTDOWN_SECONDS, ClientMessage, CombatEvent,
    CombatEventKind, GamePhase, GunnerInput, HELICOPTER_ALTITUDE_M, HELICOPTER_CHASE_DISTANCE_M,
    HELICOPTER_CHASE_TICKS, HELICOPTER_REPOSITION_OFFSET_M, HELICOPTER_REPOSITION_TICKS,
    HELICOPTER_SPEED_MPS, HelicopterPhase, HelicopterSnapshot, MAX_INPUT_TICK_AGE,
    MAX_INPUT_TICK_FUTURE, REQUIRED_SEATS, ROCKET_AIM_LEAD_SECONDS, ROCKET_BLAST_RADIUS_M,
    ROCKET_FLIGHT_TICKS, ROCKET_WARNING_TICKS, RiderInput, RocketSnapshot, RocketWarning,
    SIMULATION_HZ, SNAPSHOT_HZ, SeatKind, SeatStatus, ServerMessage, WorldPosition, WorldSnapshot,
};
use tokio::{net::TcpListener, runtime::Builder, sync::broadcast};

#[derive(Resource, Clone)]
struct ServerRuntime {
    room: Arc<Mutex<Room>>,
    snapshots: broadcast::Sender<WorldSnapshot>,
    dev_solo_enabled: bool,
}

struct Room {
    seats: BTreeMap<SeatKind, SeatSlot>,
    phase: GamePhase,
    solo_player_id: Option<u64>,
    solo_seat: Option<SeatKind>,
    countdown_deadline: Option<Instant>,
    tick: u64,
    bike: BikeState,
    rider_input: RiderInput,
    last_rider_input_at: Option<Instant>,
    last_applied_rider_sequence: u32,
    gunner_input: GunnerInput,
    helicopter: Option<HelicopterSnapshot>,
    rocket: Option<RocketSnapshot>,
    helicopter_phase_deadline_tick: u64,
    next_attack_id: u64,
    next_event_id: u64,
    recent_combat_events: VecDeque<CombatEvent>,
    rocket_hit_count: u32,
}

#[derive(Default)]
struct SeatSlot {
    player_id: Option<u64>,
    ready: bool,
    last_sequence: u32,
}

impl Room {
    fn new() -> Self {
        Self {
            seats: REQUIRED_SEATS
                .into_iter()
                .map(|seat| (seat, SeatSlot::default()))
                .collect(),
            phase: GamePhase::WaitingForPlayers,
            solo_player_id: None,
            solo_seat: None,
            countdown_deadline: None,
            tick: 0,
            bike: BikeState::default(),
            rider_input: RiderInput::default(),
            last_rider_input_at: None,
            last_applied_rider_sequence: 0,
            gunner_input: GunnerInput::default(),
            helicopter: None,
            rocket: None,
            helicopter_phase_deadline_tick: 0,
            next_attack_id: 1,
            next_event_id: 1,
            recent_combat_events: VecDeque::new(),
            rocket_hit_count: 0,
        }
    }

    fn snapshot(&self, now: Instant) -> WorldSnapshot {
        let countdown_seconds = (self.phase == GamePhase::Countdown).then(|| {
            self.countdown_deadline
                .map(|deadline| deadline.saturating_duration_since(now).as_secs_f32().ceil() as u8)
                .unwrap_or(COUNTDOWN_SECONDS)
        });
        WorldSnapshot {
            tick: self.tick,
            phase: self.phase,
            seats: self
                .seats
                .iter()
                .map(|(kind, slot)| {
                    (
                        *kind,
                        SeatStatus {
                            connected: slot.player_id.is_some(),
                            ready: slot.ready,
                        },
                    )
                })
                .collect(),
            countdown_seconds,
            bike: self.bike,
            last_applied_rider_sequence: self.last_applied_rider_sequence,
            helicopter: self.helicopter,
            rocket: self.rocket,
            combat_events: self.recent_combat_events.iter().copied().collect(),
            rocket_hit_count: self.rocket_hit_count,
        }
    }

    fn reserve_seat(&mut self, player_id: u64, solo_mode: bool) -> Option<(SeatKind, bool)> {
        if solo_mode {
            if self.seats.values().any(|slot| slot.player_id.is_some()) {
                return None;
            }
            self.solo_player_id = Some(player_id);
            self.solo_seat = Some(SeatKind::Rider);
            for slot in self.seats.values_mut() {
                slot.player_id = Some(player_id);
                slot.ready = false;
                slot.last_sequence = 0;
            }
            self.phase = GamePhase::ReadyCheck;
            return Some((SeatKind::Rider, true));
        }

        let seat = REQUIRED_SEATS.iter().copied().find(|seat| {
            self.seats
                .get(seat)
                .is_some_and(|slot| slot.player_id.is_none())
        })?;
        let slot = self.seats.get_mut(&seat)?;
        slot.player_id = Some(player_id);
        slot.ready = false;
        slot.last_sequence = 0;
        if self.phase != GamePhase::Running {
            self.phase = if self.all_required_connected() {
                GamePhase::ReadyCheck
            } else {
                GamePhase::WaitingForPlayers
            };
        }
        Some((seat, false))
    }

    fn release_seat(&mut self, player_id: u64, seat: SeatKind) {
        if self.solo_player_id == Some(player_id) {
            for slot in self.seats.values_mut() {
                slot.player_id = None;
                slot.ready = false;
                slot.last_sequence = 0;
            }
            self.solo_player_id = None;
            self.solo_seat = None;
            self.reset_match();
            return;
        }

        let Some(slot) = self.seats.get_mut(&seat) else {
            return;
        };
        if slot.player_id != Some(player_id) {
            return;
        }
        slot.player_id = None;
        slot.ready = false;
        slot.last_sequence = 0;
        self.reset_match();
        self.phase = if self.all_required_connected() {
            GamePhase::ReadyCheck
        } else {
            GamePhase::WaitingForPlayers
        };
    }

    fn reset_match(&mut self) {
        self.countdown_deadline = None;
        self.phase = if self.all_required_connected() {
            GamePhase::ReadyCheck
        } else {
            GamePhase::WaitingForPlayers
        };
        self.rider_input = RiderInput::default();
        self.last_rider_input_at = None;
        self.last_applied_rider_sequence = 0;
        self.gunner_input = GunnerInput::default();
        self.bike = BikeState::default();
        self.helicopter = None;
        self.rocket = None;
        self.helicopter_phase_deadline_tick = 0;
        self.recent_combat_events.clear();
        self.rocket_hit_count = 0;
    }

    fn begin_running(&mut self) {
        self.helicopter = Some(HelicopterSnapshot {
            position: WorldPosition {
                x: self.bike.x,
                y: HELICOPTER_ALTITUDE_M,
                z: self.bike.z + HELICOPTER_CHASE_DISTANCE_M,
            },
            yaw: 0.0,
            phase: HelicopterPhase::Chasing,
            warning: None,
        });
        self.rocket = None;
        self.rocket_hit_count = 0;
        self.helicopter_phase_deadline_tick = self.tick + HELICOPTER_CHASE_TICKS;
    }

    fn advance_tick(&mut self, now: Instant) -> Option<WorldSnapshot> {
        self.tick += 1;
        let both_connected = self.all_required_connected();
        let both_ready = self.all_required_ready();
        let was_running = self.phase == GamePhase::Running;

        if !both_connected {
            if was_running {
                self.reset_match();
            }
            self.phase = GamePhase::WaitingForPlayers;
            self.countdown_deadline = None;
        } else if self.phase != GamePhase::Running {
            if both_ready {
                if self.phase != GamePhase::Countdown {
                    self.phase = GamePhase::Countdown;
                    self.countdown_deadline =
                        Some(now + Duration::from_secs(u64::from(COUNTDOWN_SECONDS)));
                } else if self
                    .countdown_deadline
                    .is_some_and(|deadline| now >= deadline)
                {
                    self.phase = GamePhase::Running;
                    self.countdown_deadline = None;
                    self.rider_input = RiderInput::default();
                    self.begin_running();
                }
            } else {
                self.phase = GamePhase::ReadyCheck;
                self.countdown_deadline = None;
            }
        }

        if self.phase == GamePhase::Running {
            if self.last_rider_input_at.is_some_and(|last_input| {
                now.duration_since(last_input) > Duration::from_millis(200)
            }) {
                self.rider_input = RiderInput::default();
                self.last_rider_input_at = None;
            }
            self.bike.step(self.rider_input, 1.0 / SIMULATION_HZ as f32);
            self.last_applied_rider_sequence = self
                .seats
                .get(&SeatKind::Rider)
                .map_or(0, |slot| slot.last_sequence);
            self.step_helicopter();
        }

        while self
            .recent_combat_events
            .front()
            .is_some_and(|event| event.tick + COMBAT_EVENT_RETENTION_TICKS < self.tick)
        {
            self.recent_combat_events.pop_front();
        }

        self.tick
            .is_multiple_of((SIMULATION_HZ / SNAPSHOT_HZ) as u64)
            .then(|| self.snapshot(now))
    }

    fn step_helicopter(&mut self) {
        let Some(mut helicopter) = self.helicopter else {
            return;
        };
        let bike_target = WorldPosition {
            x: self.bike.x,
            y: 0.0,
            z: self.bike.z,
        };

        match helicopter.phase {
            HelicopterPhase::Chasing => {
                let chase_target = WorldPosition {
                    x: self.bike.x,
                    y: HELICOPTER_ALTITUDE_M,
                    z: self.bike.z + HELICOPTER_CHASE_DISTANCE_M,
                };
                move_toward(
                    &mut helicopter.position,
                    chase_target,
                    HELICOPTER_SPEED_MPS / SIMULATION_HZ as f32,
                );
                helicopter.yaw = heading_toward(helicopter.position, bike_target);
                if self.tick >= self.helicopter_phase_deadline_tick {
                    let attack_id = self.next_attack_id;
                    self.next_attack_id = self.next_attack_id.saturating_add(1);
                    let lead = self.bike.speed * ROCKET_AIM_LEAD_SECONDS;
                    let impact_position = WorldPosition {
                        x: (self.bike.x + self.bike.yaw.sin() * lead).clamp(-2.0, 2.0),
                        y: 0.0,
                        z: self.bike.z - self.bike.yaw.cos() * lead,
                    };
                    let impact_tick = self.tick + ROCKET_WARNING_TICKS + ROCKET_FLIGHT_TICKS;
                    helicopter.phase = HelicopterPhase::Telegraph;
                    helicopter.warning = Some(RocketWarning {
                        attack_id,
                        impact_position,
                        impact_tick,
                        blast_radius_m: ROCKET_BLAST_RADIUS_M,
                    });
                    self.helicopter_phase_deadline_tick = self.tick + ROCKET_WARNING_TICKS;
                    self.push_combat_event(
                        attack_id,
                        CombatEventKind::RocketWarning,
                        impact_position,
                    );
                }
            }
            HelicopterPhase::Telegraph => {
                if self.tick >= self.helicopter_phase_deadline_tick
                    && let Some(warning) = helicopter.warning
                {
                    let launch_position = WorldPosition {
                        x: helicopter.position.x,
                        y: helicopter.position.y - 0.55,
                        z: helicopter.position.z - 1.82,
                    };
                    self.rocket = Some(RocketSnapshot {
                        attack_id: warning.attack_id,
                        launch_position,
                        position: launch_position,
                        impact_position: warning.impact_position,
                        launch_tick: self.tick,
                        impact_tick: warning.impact_tick,
                    });
                    helicopter.phase = HelicopterPhase::RocketInFlight;
                    self.helicopter_phase_deadline_tick = warning.impact_tick;
                    self.push_combat_event(
                        warning.attack_id,
                        CombatEventKind::RocketLaunched,
                        launch_position,
                    );
                }
            }
            HelicopterPhase::RocketInFlight => {
                if let Some(mut rocket) = self.rocket {
                    let flight_ticks = rocket.impact_tick.saturating_sub(rocket.launch_tick).max(1);
                    let progress =
                        self.tick.saturating_sub(rocket.launch_tick) as f32 / flight_ticks as f32;
                    rocket.position = interpolate_position(
                        rocket.launch_position,
                        rocket.impact_position,
                        progress.clamp(0.0, 1.0),
                    );
                    if self.tick >= rocket.impact_tick {
                        let dx = self.bike.x - rocket.impact_position.x;
                        let dz = self.bike.z - rocket.impact_position.z;
                        let hit = dx.hypot(dz) <= ROCKET_BLAST_RADIUS_M;
                        if hit {
                            self.rocket_hit_count = self.rocket_hit_count.saturating_add(1);
                        }
                        self.rocket = None;
                        helicopter.phase = HelicopterPhase::Repositioning;
                        helicopter.warning = None;
                        self.helicopter_phase_deadline_tick =
                            self.tick + HELICOPTER_REPOSITION_TICKS;
                        self.push_combat_event(
                            rocket.attack_id,
                            CombatEventKind::RocketExploded {
                                hit,
                                hit_count: self.rocket_hit_count,
                            },
                            rocket.impact_position,
                        );
                    } else {
                        self.rocket = Some(rocket);
                    }
                }
            }
            HelicopterPhase::Repositioning => {
                let reposition_target = WorldPosition {
                    x: self.bike.x + HELICOPTER_REPOSITION_OFFSET_M,
                    y: HELICOPTER_ALTITUDE_M + 1.5,
                    z: self.bike.z + HELICOPTER_CHASE_DISTANCE_M + 5.0,
                };
                move_toward(
                    &mut helicopter.position,
                    reposition_target,
                    HELICOPTER_SPEED_MPS / SIMULATION_HZ as f32,
                );
                helicopter.yaw = heading_toward(helicopter.position, bike_target);
                if self.tick >= self.helicopter_phase_deadline_tick {
                    helicopter.phase = HelicopterPhase::Chasing;
                    self.helicopter_phase_deadline_tick = self.tick + HELICOPTER_CHASE_TICKS;
                }
            }
        }

        self.helicopter = Some(helicopter);
    }

    fn push_combat_event(
        &mut self,
        attack_id: u64,
        kind: CombatEventKind,
        position: WorldPosition,
    ) {
        let event = CombatEvent {
            event_id: self.next_event_id,
            attack_id,
            tick: self.tick,
            position,
            kind,
        };
        self.next_event_id = self.next_event_id.saturating_add(1);
        self.recent_combat_events.push_back(event);
    }

    fn switch_solo_seat(&mut self, player_id: u64, seat: SeatKind) -> bool {
        if self.solo_player_id != Some(player_id) {
            return false;
        }
        self.solo_seat = Some(seat);
        self.rider_input = RiderInput::default();
        self.last_rider_input_at = None;
        true
    }

    fn receive(&mut self, player_id: u64, seat: SeatKind, message: ClientMessage) {
        let phase = self.phase;
        if matches!(&message, ClientMessage::ForceStartSolo) {
            if self.solo_player_id == Some(player_id) && phase != GamePhase::Running {
                for slot in self.seats.values_mut() {
                    slot.ready = true;
                }
            }
            return;
        }
        if self.solo_player_id == Some(player_id) && self.solo_seat != Some(seat) {
            return;
        }
        let Some(slot) = self.seats.get_mut(&seat) else {
            return;
        };
        if slot.player_id != Some(player_id) {
            return;
        }
        match message {
            ClientMessage::SetReady { ready } => {
                if phase != GamePhase::Running {
                    slot.ready = ready;
                }
            }
            ClientMessage::RiderInput {
                sequence,
                tick,
                mut input,
            } if seat == SeatKind::Rider && phase == GamePhase::Running => {
                if sequence > slot.last_sequence && input_tick_is_valid(self.tick, tick) {
                    input.steer = if input.steer.is_finite() {
                        input.steer.clamp(-1.0, 1.0)
                    } else {
                        0.0
                    };
                    self.rider_input = input;
                    self.last_rider_input_at = Some(Instant::now());
                    slot.last_sequence = sequence;
                }
            }
            ClientMessage::GunnerInput {
                sequence,
                tick,
                mut input,
            } if seat == SeatKind::Gunner && phase == GamePhase::Running => {
                if sequence > slot.last_sequence && input_tick_is_valid(self.tick, tick) {
                    input.yaw = if input.yaw.is_finite() {
                        input.yaw.clamp(-1.92, 1.92)
                    } else {
                        0.0
                    };
                    input.pitch = if input.pitch.is_finite() {
                        input.pitch.clamp(-0.55, 0.45)
                    } else {
                        0.0
                    };
                    self.gunner_input = input;
                    slot.last_sequence = sequence;
                }
            }
            ClientMessage::ForceStartSolo
            | ClientMessage::SwitchSeat { .. }
            | ClientMessage::RiderInput { .. }
            | ClientMessage::GunnerInput { .. } => {}
        }
    }

    fn all_required_connected(&self) -> bool {
        REQUIRED_SEATS.iter().all(|seat| {
            self.seats
                .get(seat)
                .is_some_and(|slot| slot.player_id.is_some())
        })
    }

    fn all_required_ready(&self) -> bool {
        self.all_required_connected()
            && REQUIRED_SEATS
                .iter()
                .all(|seat| self.seats.get(seat).is_some_and(|slot| slot.ready))
    }
}

fn input_tick_is_valid(server_tick: u64, input_tick: u64) -> bool {
    input_tick.saturating_add(MAX_INPUT_TICK_AGE) >= server_tick
        && input_tick <= server_tick.saturating_add(MAX_INPUT_TICK_FUTURE)
}

fn move_toward(current: &mut WorldPosition, target: WorldPosition, max_distance: f32) {
    let dx = target.x - current.x;
    let dy = target.y - current.y;
    let dz = target.z - current.z;
    let distance = (dx * dx + dy * dy + dz * dz).sqrt();
    if distance <= max_distance || distance <= f32::EPSILON {
        *current = target;
        return;
    }
    let amount = max_distance / distance;
    current.x += dx * amount;
    current.y += dy * amount;
    current.z += dz * amount;
}

fn heading_toward(from: WorldPosition, to: WorldPosition) -> f32 {
    (to.x - from.x).atan2(from.z - to.z)
}

fn interpolate_position(from: WorldPosition, to: WorldPosition, amount: f32) -> WorldPosition {
    WorldPosition {
        x: from.x + (to.x - from.x) * amount,
        y: from.y + (to.y - from.y) * amount,
        z: from.z + (to.z - from.z) * amount,
    }
}

fn main() {
    let (snapshots, _) = broadcast::channel(8);
    let runtime = ServerRuntime {
        room: Arc::new(Mutex::new(Room::new())),
        snapshots,
        dev_solo_enabled: std::env::var("RIVERSIDE_DEV_SOLO").is_ok_and(|value| value == "1"),
    };

    let http_runtime = runtime.clone();
    std::thread::Builder::new()
        .name("riverside-websocket".to_owned())
        .spawn(move || {
            let result = Builder::new_multi_thread().enable_all().build();
            match result {
                Ok(runtime) => runtime.block_on(run_http(http_runtime)),
                Err(error) => eprintln!("Failed to start networking runtime: {error}"),
            }
        })
        .expect("failed to create networking thread");

    let loop_period = Duration::from_secs_f64(1.0 / f64::from(SIMULATION_HZ));
    App::new()
        .insert_resource(runtime)
        .add_plugins(ScheduleRunnerPlugin::run_loop(loop_period))
        .add_systems(Update, tick_room)
        .run();
}

async fn run_http(runtime: ServerRuntime) {
    let address = std::env::var("RIVERSIDE_BIND")
        .unwrap_or_else(|_| "0.0.0.0:3001".to_owned())
        .parse::<SocketAddr>();
    let address = match address {
        Ok(address) => address,
        Err(error) => {
            eprintln!("Invalid RIVERSIDE_BIND address: {error}");
            return;
        }
    };
    let listener = match TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("Could not bind game server to {address}: {error}");
            return;
        }
    };
    println!("Riverside Escape WebSocket server listening on ws://{address}/ws");

    let app = Router::new()
        .route("/ws", get(websocket_upgrade))
        .route("/health", get(|| async { "ok" }))
        .with_state(runtime);
    if let Err(error) = axum::serve(listener, app).await {
        eprintln!("WebSocket server stopped: {error}");
    }
}

async fn websocket_upgrade(
    State(runtime): State<ServerRuntime>,
    OriginalUri(uri): OriginalUri,
    websocket: WebSocketUpgrade,
) -> impl IntoResponse {
    let requested_solo = uri
        .query()
        .is_some_and(|query| query.split('&').any(|part| part == "solo=1"));
    websocket
        .max_message_size(16 * 1024)
        .on_upgrade(move |socket| handle_socket(socket, runtime, requested_solo))
}

async fn handle_socket(
    mut socket: axum::extract::ws::WebSocket,
    runtime: ServerRuntime,
    requested_solo: bool,
) {
    static NEXT_PLAYER_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let player_id = NEXT_PLAYER_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if requested_solo && !runtime.dev_solo_enabled {
        let response = serde_json::to_string(&ServerMessage::SoloUnavailable)
            .expect("serializing SoloUnavailable");
        let _ = socket
            .send(axum::extract::ws::Message::Text(response.into()))
            .await;
        let _ = socket.close().await;
        return;
    }
    let reservation = match runtime.room.lock() {
        Ok(mut room) => room.reserve_seat(player_id, requested_solo),
        Err(error) => {
            eprintln!("Room state lock was poisoned: {error}");
            return;
        }
    };
    let Some((mut seat, solo_mode)) = reservation else {
        let full = serde_json::to_string(&ServerMessage::RoomFull).expect("serializing RoomFull");
        let _ = socket
            .send(axum::extract::ws::Message::Text(full.into()))
            .await;
        let _ = socket.close().await;
        return;
    };

    let welcome = ServerMessage::Welcome {
        player_id,
        seat,
        solo_mode,
    };
    let encoded = serde_json::to_string(&welcome).expect("serializing welcome message");
    if socket
        .send(axum::extract::ws::Message::Text(encoded.into()))
        .await
        .is_err()
    {
        disconnect(&runtime, player_id, seat);
        return;
    }

    let mut updates = runtime.snapshots.subscribe();
    let (mut sender, mut receiver) = socket.split();
    loop {
        tokio::select! {
            incoming = receiver.next() => {
                let Some(Ok(message)) = incoming else { break };
                match message {
                    axum::extract::ws::Message::Text(text) => {
                        if let Ok(message) = serde_json::from_str::<ClientMessage>(text.as_str()) {
                            match message {
                                ClientMessage::SwitchSeat { seat: next_seat } => {
                                    let accepted = match runtime.room.lock() {
                                        Ok(mut room) => room.switch_solo_seat(player_id, next_seat),
                                        Err(error) => {
                                            eprintln!("Room state lock was poisoned: {error}");
                                            false
                                        }
                                    };
                                    if accepted {
                                        seat = next_seat;
                                        let response = serde_json::to_string(
                                            &ServerMessage::SeatChanged { seat },
                                        ).expect("serializing SeatChanged");
                                        if sender.send(axum::extract::ws::Message::Text(response.into())).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                                message => {
                                    if let Ok(mut room) = runtime.room.lock() {
                                        room.receive(player_id, seat, message);
                                    }
                                }
                            }
                        }
                    }
                    axum::extract::ws::Message::Close(_) => break,
                    axum::extract::ws::Message::Ping(payload) => {
                        if sender.send(axum::extract::ws::Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    axum::extract::ws::Message::Binary(_) | axum::extract::ws::Message::Pong(_) => {}
                }
            }
            update = updates.recv() => {
                let snapshot = match update {
                    Ok(snapshot) => snapshot,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                let encoded = serde_json::to_string(&ServerMessage::Snapshot(Box::new(snapshot)))
                    .expect("serializing room snapshot");
                if sender.send(axum::extract::ws::Message::Text(encoded.into())).await.is_err() {
                    break;
                }
            }
        }
    }
    disconnect(&runtime, player_id, seat);
}

fn disconnect(runtime: &ServerRuntime, player_id: u64, seat: SeatKind) {
    if let Ok(mut room) = runtime.room.lock() {
        room.release_seat(player_id, seat);
    }
}

fn tick_room(runtime: Res<ServerRuntime>) {
    let now = Instant::now();
    let mut room = match runtime.room.lock() {
        Ok(room) => room,
        Err(error) => {
            eprintln!("Room state lock was poisoned: {error}");
            return;
        }
    };
    let snapshot = room.advance_tick(now);
    drop(room);
    if let Some(snapshot) = snapshot {
        let _ = runtime.snapshots.send(snapshot);
    }
}

#[cfg(test)]
mod tests {
    use super::{Room, input_tick_is_valid};
    use riverside_common::{
        ClientMessage, CombatEventKind, GamePhase, GunnerInput, HELICOPTER_CHASE_TICKS,
        HelicopterPhase, REQUIRED_SEATS, ROCKET_BLAST_RADIUS_M, ROCKET_FLIGHT_TICKS,
        ROCKET_WARNING_TICKS, RiderInput, SIMULATION_HZ,
    };
    use std::{
        collections::BTreeSet,
        time::{Duration, Instant},
    };

    #[test]
    fn input_tick_validation_allows_network_delay_but_rejects_stale_or_future_values() {
        assert!(input_tick_is_valid(1_000, 700));
        assert!(input_tick_is_valid(1_000, 1_003));
        assert!(!input_tick_is_valid(1_000, 699));
        assert!(!input_tick_is_valid(1_000, 1_004));
    }

    #[test]
    fn running_inputs_remain_limited_to_the_authorized_seat() {
        let mut room = Room::new();
        connect_and_ready_both_seats(&mut room);
        room.phase = GamePhase::Running;
        let pedal = RiderInput {
            pedal_held: true,
            ..RiderInput::default()
        };

        room.receive(
            2,
            super::SeatKind::Gunner,
            ClientMessage::RiderInput {
                sequence: 1,
                tick: room.tick,
                input: pedal,
            },
        );
        room.receive(
            1,
            super::SeatKind::Rider,
            ClientMessage::GunnerInput {
                sequence: 1,
                tick: room.tick,
                input: GunnerInput {
                    yaw: 0.5,
                    pitch: 0.2,
                    aim_held: true,
                },
            },
        );
        assert_eq!(room.rider_input, RiderInput::default());
        assert_eq!(room.gunner_input, GunnerInput::default());

        room.receive(
            1,
            super::SeatKind::Rider,
            ClientMessage::RiderInput {
                sequence: 1,
                tick: room.tick,
                input: pedal,
            },
        );
        assert_eq!(room.rider_input, pedal);
        assert_eq!(room.seats[&super::SeatKind::Rider].last_sequence, 1);
    }

    fn connect_and_ready_both_seats(room: &mut Room) {
        for (index, seat) in REQUIRED_SEATS.into_iter().enumerate() {
            let slot = room.seats.get_mut(&seat).expect("required seat exists");
            slot.player_id = Some(index as u64 + 1);
            slot.ready = true;
        }
    }

    fn running_room() -> Room {
        let mut room = Room::new();
        connect_and_ready_both_seats(&mut room);
        room.phase = GamePhase::Running;
        room.begin_running();
        room.countdown_deadline = None;
        room
    }

    fn step_room(room: &mut Room, now: &mut Instant, input: RiderInput) {
        *now += Duration::from_secs_f64(1.0 / f64::from(super::SIMULATION_HZ));
        room.rider_input = input;
        room.last_rider_input_at = Some(*now);
        let _ = room.advance_tick(*now);
    }

    fn warning_event(room: &Room) -> super::CombatEvent {
        *room
            .recent_combat_events
            .iter()
            .find(|event| matches!(event.kind, CombatEventKind::RocketWarning))
            .expect("rocket warning event exists")
    }

    #[test]
    fn helicopter_and_rocket_are_absent_until_countdown_finishes() {
        let start = Instant::now();
        let deadline = start + Duration::from_secs(5);
        let mut room = Room::new();
        connect_and_ready_both_seats(&mut room);
        room.phase = GamePhase::Countdown;
        room.countdown_deadline = Some(deadline);

        for tick in 0..u64::from(super::SIMULATION_HZ) * 5 - 1 {
            let now =
                start + Duration::from_secs_f64(tick as f64 / f64::from(super::SIMULATION_HZ));
            room.advance_tick(now);
            assert_eq!(room.phase, GamePhase::Countdown);
            assert!(room.helicopter.is_none());
            assert!(room.rocket.is_none());
            assert!(room.recent_combat_events.is_empty());
        }

        room.advance_tick(deadline);
        assert_eq!(room.phase, GamePhase::Running);
        assert_eq!(room.helicopter.unwrap().phase, HelicopterPhase::Chasing);
        assert!(room.rocket.is_none());
        assert!(room.recent_combat_events.is_empty());
    }

    #[test]
    fn staying_in_the_fixed_zone_is_hit_once_with_unique_events() {
        let mut now = Instant::now();
        let mut room = running_room();
        let total_ticks = HELICOPTER_CHASE_TICKS + ROCKET_WARNING_TICKS + ROCKET_FLIGHT_TICKS;
        for _ in 0..total_ticks {
            step_room(&mut room, &mut now, RiderInput::default());
        }

        assert_eq!(room.rocket_hit_count, 1);
        assert_eq!(
            room.helicopter.unwrap().phase,
            HelicopterPhase::Repositioning
        );
        let explosions = room
            .recent_combat_events
            .iter()
            .filter(|event| {
                matches!(
                    event.kind,
                    CombatEventKind::RocketExploded { hit: true, .. }
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(explosions.len(), 1);
        let event_ids = room
            .recent_combat_events
            .iter()
            .map(|event| event.event_id)
            .collect::<BTreeSet<_>>();
        assert_eq!(event_ids.len(), room.recent_combat_events.len());
        let attack_ids = room
            .recent_combat_events
            .iter()
            .map(|event| event.attack_id)
            .collect::<BTreeSet<_>>();
        assert_eq!(attack_ids, BTreeSet::from([1]));

        let ids_in_first_snapshot = room
            .snapshot(now)
            .combat_events
            .into_iter()
            .map(|event| event.event_id)
            .collect::<Vec<_>>();
        let ids_in_next_snapshot = room
            .snapshot(now + Duration::from_millis(50))
            .combat_events
            .into_iter()
            .map(|event| event.event_id)
            .collect::<Vec<_>>();
        assert_eq!(ids_in_first_snapshot, ids_in_next_snapshot);

        for _ in 0..30 {
            step_room(&mut room, &mut now, RiderInput::default());
        }
        assert_eq!(room.rocket_hit_count, 1);
        for _ in 0..SIMULATION_HZ {
            step_room(&mut room, &mut now, RiderInput::default());
        }
        assert_eq!(room.rocket_hit_count, 1);
        assert_eq!(room.recent_combat_events.len(), 3);
        assert_eq!(
            room.recent_combat_events
                .iter()
                .filter(|event| matches!(event.kind, CombatEventKind::RocketExploded { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn moving_out_of_the_fixed_zone_avoids_the_rocket() {
        let mut now = Instant::now();
        let mut room = running_room();
        for _ in 0..HELICOPTER_CHASE_TICKS {
            step_room(&mut room, &mut now, RiderInput::default());
        }
        assert_eq!(room.helicopter.unwrap().phase, HelicopterPhase::Telegraph);
        let impact_position = room.helicopter.unwrap().warning.unwrap().impact_position;
        let mut target_stayed_fixed = true;

        for _ in 0..ROCKET_WARNING_TICKS + ROCKET_FLIGHT_TICKS {
            let input = RiderInput {
                pedal_held: true,
                ..RiderInput::default()
            };
            step_room(&mut room, &mut now, input);
            if let Some(warning) = room.helicopter.and_then(|state| state.warning) {
                target_stayed_fixed &= warning.impact_position == impact_position;
            }
        }

        let distance = (room.bike.x - impact_position.x).hypot(room.bike.z - impact_position.z);
        assert!(target_stayed_fixed);
        assert!(distance > ROCKET_BLAST_RADIUS_M);
        assert_eq!(room.rocket_hit_count, 0);
        assert!(room.recent_combat_events.iter().any(|event| matches!(
            event.kind,
            CombatEventKind::RocketExploded {
                hit: false,
                hit_count: 0
            }
        )));
    }

    #[test]
    fn reset_clears_combat_state_without_reusing_event_ids() {
        let mut now = Instant::now();
        let mut room = running_room();
        for _ in 0..HELICOPTER_CHASE_TICKS {
            step_room(&mut room, &mut now, RiderInput::default());
        }
        let warning_id = warning_event(&room).event_id;
        let next_event_id = room.next_event_id;

        room.reset_match();
        assert!(room.helicopter.is_none());
        assert!(room.rocket.is_none());
        assert!(room.recent_combat_events.is_empty());
        assert_eq!(room.rocket_hit_count, 0);
        assert_eq!(room.next_event_id, next_event_id);
        assert!(next_event_id > warning_id);

        room.phase = GamePhase::Running;
        room.begin_running();
        for _ in 0..HELICOPTER_CHASE_TICKS {
            step_room(&mut room, &mut now, RiderInput::default());
        }
        assert!(warning_event(&room).event_id > warning_id);
    }
}
