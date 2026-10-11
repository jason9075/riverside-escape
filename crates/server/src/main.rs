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
    BIKE_ROCKET_DAMAGE, BIKE_STARTING_HEALTH, BikeState, COMBAT_EVENT_RETENTION_TICKS,
    COUNTDOWN_SECONDS, ClientMessage, CombatEvent, CombatEventKind, FireRejectionReason,
    GUNNER_AIM_ORIGIN_HEIGHT_M, GUNNER_FIRE_COOLDOWN_TICKS, GUNNER_LOCK_GRACE_TICKS,
    GUNNER_LOCK_TICKS, GUNNER_STARTING_AMMO, GamePhase, GunnerInput, HELICOPTER_ALTITUDE_M,
    HELICOPTER_CHASE_DISTANCE_M, HELICOPTER_CHASE_TICKS, HELICOPTER_REPOSITION_OFFSET_M,
    HELICOPTER_REPOSITION_TICKS, HELICOPTER_SPEED_MPS, HELICOPTER_STARTING_HEALTH, HelicopterPhase,
    HelicopterSnapshot, MAX_INPUT_TICK_AGE, MAX_INPUT_TICK_FUTURE, MatchResult, MatchVictoryReason,
    PLAYER_MISSILE_DAMAGE, PLAYER_MISSILE_HIT_RADIUS_M, PLAYER_MISSILE_LIFETIME_TICKS,
    PLAYER_MISSILE_SPEED_MPS, PlayerMissileSnapshot, REQUIRED_SEATS, ROCKET_AIM_LEAD_SECONDS,
    ROCKET_BLAST_RADIUS_M, ROCKET_FLIGHT_TICKS, ROCKET_WARNING_TICKS, RiderInput, RoadSection,
    RocketSnapshot, RocketWarning, SIMULATION_HZ, SNAPSHOT_HZ, SeatKind, SeatStatus, ServerMessage,
    WorldPosition, WorldSnapshot, evaluate_gunner_aim, first_cover_intersection,
    line_of_sight_blocked, riverside_course,
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
    last_gunner_input_at: Option<Instant>,
    gunner_lock_progress_ticks: u64,
    gunner_lock_outside_since_tick: Option<u64>,
    player_missile: Option<PlayerMissileSnapshot>,
    bike_health: u16,
    helicopter_health: u16,
    gunner_ammo: u8,
    fire_cooldown_until_tick: u64,
    match_result: Option<MatchResult>,
    match_victory_reason: Option<MatchVictoryReason>,
    route_section: RoadSection,
    route_distance_m: f32,
    route_checkpoints_passed: usize,
    escape_finish_crossed: bool,
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
    last_fire_request_id: u64,
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
            last_gunner_input_at: None,
            gunner_lock_progress_ticks: 0,
            gunner_lock_outside_since_tick: None,
            player_missile: None,
            bike_health: BIKE_STARTING_HEALTH,
            helicopter_health: HELICOPTER_STARTING_HEALTH,
            gunner_ammo: GUNNER_STARTING_AMMO,
            fire_cooldown_until_tick: 0,
            match_result: None,
            match_victory_reason: None,
            route_section: RoadSection::OpenRiverside,
            route_distance_m: 0.0,
            route_checkpoints_passed: 0,
            escape_finish_crossed: false,
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
            route_section: self.route_section,
            route_distance_m: self.route_distance_m,
            route_checkpoints_passed: self.route_checkpoints_passed as u8,
            route_checkpoints_total: riverside_course().checkpoints_m().len() as u8,
            route_length_m: riverside_course().length_m(),
            last_applied_rider_sequence: self.last_applied_rider_sequence,
            helicopter: self.helicopter,
            rocket: self.rocket,
            combat_events: self.recent_combat_events.iter().copied().collect(),
            rocket_hit_count: self.rocket_hit_count,
            player_missile: self.player_missile,
            bike_health: self.bike_health,
            helicopter_health: self.helicopter_health,
            gunner_ammo: self.gunner_ammo,
            gunner_lock_progress_ticks: self.gunner_lock_progress_ticks.min(u64::from(u16::MAX))
                as u16,
            gunner_lock_grace_ticks_remaining: self.gunner_lock_outside_since_tick.map_or(
                0,
                |started| {
                    GUNNER_LOCK_GRACE_TICKS
                        .saturating_sub(self.tick.saturating_sub(started))
                        .min(u64::from(u16::MAX)) as u16
                },
            ),
            gunner_locked: self.gunner_lock_progress_ticks >= GUNNER_LOCK_TICKS
                && self.gunner_lock_outside_since_tick.is_none(),
            gunner_can_fire: self.gunner_can_fire(now),
            fire_cooldown_ticks_remaining: self
                .fire_cooldown_until_tick
                .saturating_sub(self.tick)
                .min(u64::from(u16::MAX)) as u16,
            match_result: self.match_result,
            match_victory_reason: self.match_victory_reason,
        }
    }

    fn reset_route_progress(&mut self) {
        self.route_section = RoadSection::OpenRiverside;
        self.route_distance_m = 0.0;
        self.route_checkpoints_passed = 0;
        self.escape_finish_crossed = false;
    }

    fn update_route_progress(&mut self, previous_bike: BikeState) {
        let course = riverside_course();
        let projection = course.project(self.bike.x, self.bike.z);
        self.route_section = projection.section;
        self.route_distance_m = projection.distance_m;

        while let Some(&checkpoint_m) = course.checkpoints_m().get(self.route_checkpoints_passed) {
            if course.crosses_forward_gate(
                previous_bike.x,
                previous_bike.z,
                self.bike.x,
                self.bike.z,
                checkpoint_m,
            ) {
                self.route_checkpoints_passed += 1;
            } else {
                break;
            }
        }
        if self.route_checkpoints_passed == course.checkpoints_m().len()
            && course.crosses_forward_gate(
                previous_bike.x,
                previous_bike.z,
                self.bike.x,
                self.bike.z,
                course.finish_distance_m(),
            )
        {
            self.escape_finish_crossed = true;
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
                slot.last_fire_request_id = 0;
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
        slot.last_fire_request_id = 0;
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
                slot.last_fire_request_id = 0;
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
        slot.last_fire_request_id = 0;
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
        self.last_gunner_input_at = None;
        self.clear_gunner_lock();
        self.player_missile = None;
        self.bike_health = BIKE_STARTING_HEALTH;
        self.helicopter_health = HELICOPTER_STARTING_HEALTH;
        self.gunner_ammo = GUNNER_STARTING_AMMO;
        self.fire_cooldown_until_tick = 0;
        self.match_result = None;
        self.match_victory_reason = None;
        self.bike = BikeState::default();
        self.reset_route_progress();
        self.helicopter = None;
        self.rocket = None;
        self.helicopter_phase_deadline_tick = 0;
        self.recent_combat_events.clear();
        self.rocket_hit_count = 0;
    }

    fn begin_running(&mut self) {
        self.reset_route_progress();
        self.bike_health = BIKE_STARTING_HEALTH;
        self.helicopter_health = HELICOPTER_STARTING_HEALTH;
        self.gunner_ammo = GUNNER_STARTING_AMMO;
        self.fire_cooldown_until_tick = self.tick;
        self.player_missile = None;
        self.match_result = None;
        self.match_victory_reason = None;
        self.rocket_hit_count = 0;
        self.clear_gunner_lock();
        self.gunner_input = GunnerInput::default();
        self.last_gunner_input_at = None;
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
        } else if self.phase == GamePhase::Finished {
            if both_ready {
                self.reset_match();
                self.phase = GamePhase::Countdown;
                self.countdown_deadline =
                    Some(now + Duration::from_secs(u64::from(COUNTDOWN_SECONDS)));
            }
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
            if self.last_gunner_input_at.is_some_and(|last_input| {
                now.duration_since(last_input) > Duration::from_millis(200)
            }) {
                self.gunner_input = GunnerInput::default();
                self.last_gunner_input_at = None;
            }
            let previous_bike = self.bike;
            self.bike.step(self.rider_input, 1.0 / SIMULATION_HZ as f32);
            self.update_route_progress(previous_bike);
            self.last_applied_rider_sequence = self
                .seats
                .get(&SeatKind::Rider)
                .map_or(0, |slot| slot.last_sequence);
            self.update_gunner_lock();
            self.step_helicopter();
            self.step_player_missile();
            self.finish_if_needed();
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
            y: GUNNER_AIM_ORIGIN_HEIGHT_M,
            z: self.bike.z,
        };

        match helicopter.phase {
            HelicopterPhase::Chasing => {
                if line_of_sight_blocked(helicopter.position, bike_target) {
                    // Stay in a pursuit/reposition loop while bridge cover
                    // blocks the bike. Do not telegraph a shot through the deck.
                    helicopter.phase = HelicopterPhase::Repositioning;
                    helicopter.warning = None;
                    self.helicopter_phase_deadline_tick = self.tick + HELICOPTER_REPOSITION_TICKS;
                } else {
                    let chase_target = WorldPosition {
                        x: self.bike.x,
                        y: HELICOPTER_ALTITUDE_M,
                        z: self.bike.z + HELICOPTER_CHASE_DISTANCE_M,
                    };
                    move_helicopter_toward(
                        &mut helicopter.position,
                        chase_target,
                        HELICOPTER_SPEED_MPS / SIMULATION_HZ as f32,
                    );
                    helicopter.yaw = heading_toward(helicopter.position, bike_target);
                }
                if helicopter.phase == HelicopterPhase::Chasing
                    && self.tick >= self.helicopter_phase_deadline_tick
                    && !line_of_sight_blocked(helicopter.position, bike_target)
                {
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
                if line_of_sight_blocked(helicopter.position, bike_target) {
                    helicopter.phase = HelicopterPhase::Repositioning;
                    helicopter.warning = None;
                    self.helicopter_phase_deadline_tick = self.tick + HELICOPTER_REPOSITION_TICKS;
                } else if self.tick >= self.helicopter_phase_deadline_tick
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
                    let next_position = interpolate_position(
                        rocket.launch_position,
                        rocket.impact_position,
                        progress.clamp(0.0, 1.0),
                    );
                    if let Some(fraction) = first_cover_intersection(rocket.position, next_position)
                    {
                        let impact_position =
                            interpolate_position(rocket.position, next_position, fraction);
                        self.rocket = None;
                        helicopter.phase = HelicopterPhase::Repositioning;
                        helicopter.warning = None;
                        self.helicopter_phase_deadline_tick =
                            self.tick + HELICOPTER_REPOSITION_TICKS;
                        self.push_combat_event(
                            rocket.attack_id,
                            CombatEventKind::RocketExploded {
                                hit: false,
                                hit_count: self.rocket_hit_count,
                            },
                            impact_position,
                        );
                    } else if self.tick >= rocket.impact_tick {
                        let dx = self.bike.x - rocket.impact_position.x;
                        let dz = self.bike.z - rocket.impact_position.z;
                        let hit = dx.hypot(dz) <= ROCKET_BLAST_RADIUS_M;
                        if hit {
                            self.rocket_hit_count = self.rocket_hit_count.saturating_add(1);
                            self.bike_health = self.bike_health.saturating_sub(BIKE_ROCKET_DAMAGE);
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
                        if hit {
                            self.push_combat_event(
                                rocket.attack_id,
                                CombatEventKind::BikeDamaged {
                                    health_remaining: self.bike_health,
                                },
                                rocket.impact_position,
                            );
                        }
                    } else {
                        rocket.position = next_position;
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
                move_helicopter_toward(
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

    fn update_gunner_lock(&mut self) {
        if !self.gunner_input.aim_held
            || self.last_gunner_input_at.is_none()
            || self.helicopter_health == 0
        {
            self.clear_gunner_lock();
            return;
        }
        let Some(helicopter) = self.helicopter else {
            self.clear_gunner_lock();
            return;
        };
        let aim = evaluate_gunner_aim(self.bike, self.gunner_input, helicopter.position);
        if !aim.in_range || aim.obstructed {
            self.clear_gunner_lock();
            return;
        }
        if aim.in_cone {
            self.gunner_lock_outside_since_tick = None;
            self.gunner_lock_progress_ticks =
                (self.gunner_lock_progress_ticks + 1).min(GUNNER_LOCK_TICKS);
        } else if self.gunner_lock_progress_ticks > 0 {
            let outside_since = *self.gunner_lock_outside_since_tick.get_or_insert(self.tick);
            if self.tick.saturating_sub(outside_since) > GUNNER_LOCK_GRACE_TICKS {
                self.clear_gunner_lock();
            }
        }
    }

    fn clear_gunner_lock(&mut self) {
        self.gunner_lock_progress_ticks = 0;
        self.gunner_lock_outside_since_tick = None;
    }

    fn gunner_can_fire(&self, now: Instant) -> bool {
        self.fire_rejection_reason(self.tick, now).is_none()
    }

    fn fire_rejection_reason(
        &self,
        request_tick: u64,
        now: Instant,
    ) -> Option<FireRejectionReason> {
        if self.phase != GamePhase::Running {
            return Some(FireRejectionReason::NotRunning);
        }
        if !input_tick_is_valid(self.tick, request_tick)
            || self
                .last_gunner_input_at
                .is_none_or(|last| now.saturating_duration_since(last) > Duration::from_millis(200))
        {
            return Some(FireRejectionReason::StaleInput);
        }
        if !self.gunner_input.aim_held {
            return Some(FireRejectionReason::NotAiming);
        }
        let Some(helicopter) = self.helicopter.filter(|_| self.helicopter_health > 0) else {
            return Some(FireRejectionReason::NoTarget);
        };
        let aim = evaluate_gunner_aim(self.bike, self.gunner_input, helicopter.position);
        if !aim.in_range {
            return Some(FireRejectionReason::OutOfRange);
        }
        if aim.obstructed {
            return Some(FireRejectionReason::Obstructed);
        }
        if !aim.in_cone {
            return Some(FireRejectionReason::TargetOutsideCone);
        }
        if self.tick < self.fire_cooldown_until_tick {
            return Some(FireRejectionReason::Cooldown);
        }
        if self.gunner_ammo == 0 {
            return Some(FireRejectionReason::NoAmmo);
        }
        if self.player_missile.is_some() {
            return Some(FireRejectionReason::MissileInFlight);
        }
        if self.gunner_lock_progress_ticks < GUNNER_LOCK_TICKS
            || self.gunner_lock_outside_since_tick.is_some()
        {
            return Some(FireRejectionReason::NotLocked);
        }
        None
    }

    fn step_player_missile(&mut self) {
        let Some(mut missile) = self.player_missile else {
            return;
        };
        let Some(helicopter) = self.helicopter.filter(|_| self.helicopter_health > 0) else {
            self.player_missile = None;
            return;
        };
        let target = helicopter.position;
        let dx = target.x - missile.position.x;
        let dy = target.y - missile.position.y;
        let dz = target.z - missile.position.z;
        let distance = (dx * dx + dy * dy + dz * dz).sqrt();
        let travel = PLAYER_MISSILE_SPEED_MPS / SIMULATION_HZ as f32;
        let reaches_target = distance <= travel + PLAYER_MISSILE_HIT_RADIUS_M;
        let next_position = if reaches_target {
            target
        } else {
            WorldPosition {
                x: missile.position.x + dx / distance * travel,
                y: missile.position.y + dy / distance * travel,
                z: missile.position.z + dz / distance * travel,
            }
        };
        if let Some(fraction) = first_cover_intersection(missile.position, next_position) {
            let impact_position = interpolate_position(missile.position, next_position, fraction);
            self.player_missile = None;
            self.push_combat_event(
                missile.attack_id,
                CombatEventKind::PlayerMissileImpacted {
                    hit: false,
                    helicopter_health: self.helicopter_health,
                },
                impact_position,
            );
            return;
        }
        if distance <= travel + PLAYER_MISSILE_HIT_RADIUS_M {
            self.helicopter_health = self.helicopter_health.saturating_sub(PLAYER_MISSILE_DAMAGE);
            self.player_missile = None;
            self.push_combat_event(
                missile.attack_id,
                CombatEventKind::PlayerMissileImpacted {
                    hit: true,
                    helicopter_health: self.helicopter_health,
                },
                target,
            );
        } else if self.tick >= missile.expires_tick {
            self.player_missile = None;
            self.push_combat_event(
                missile.attack_id,
                CombatEventKind::PlayerMissileImpacted {
                    hit: false,
                    helicopter_health: self.helicopter_health,
                },
                missile.position,
            );
        } else {
            missile.position = next_position;
            self.player_missile = Some(missile);
        }
    }

    fn finish_if_needed(&mut self) {
        if self.phase != GamePhase::Running {
            return;
        }
        let (result, victory_reason) = if self.bike_health == 0 {
            (Some(MatchResult::RidersDefeat), None)
        } else if self.helicopter_health == 0 {
            (
                Some(MatchResult::RidersVictory),
                Some(MatchVictoryReason::HelicopterDestroyed),
            )
        } else if self.escape_finish_crossed {
            (
                Some(MatchResult::RidersVictory),
                Some(MatchVictoryReason::Escaped),
            )
        } else {
            (None, None)
        };
        let Some(result) = result else {
            return;
        };
        self.phase = GamePhase::Finished;
        self.match_result = Some(result);
        self.match_victory_reason = victory_reason;
        self.countdown_deadline = None;
        for slot in self.seats.values_mut() {
            slot.ready = false;
        }
        self.rider_input = RiderInput::default();
        self.last_rider_input_at = None;
        self.gunner_input = GunnerInput::default();
        self.last_gunner_input_at = None;
        self.clear_gunner_lock();
        self.player_missile = None;
        self.rocket = None;
        self.helicopter = None;
        self.push_combat_event(
            0,
            CombatEventKind::MatchFinished {
                result,
                victory_reason,
            },
            WorldPosition {
                x: self.bike.x,
                y: 0.0,
                z: self.bike.z,
            },
        );
    }

    fn handle_fire_request(&mut self, request_id: u64, request_tick: u64, now: Instant) {
        if request_id == 0 {
            return;
        }
        let slot = self
            .seats
            .get_mut(&SeatKind::Gunner)
            .expect("Gunner seat is part of the required room");
        if request_id <= slot.last_fire_request_id {
            return;
        }
        slot.last_fire_request_id = request_id;
        if let Some(reason) = self.fire_rejection_reason(request_tick, now) {
            self.push_combat_event(
                0,
                CombatEventKind::GunnerFireRejected { request_id, reason },
                WorldPosition {
                    x: self.bike.x,
                    y: 0.0,
                    z: self.bike.z,
                },
            );
            return;
        }

        let attack_id = self.next_attack_id;
        self.next_attack_id = self.next_attack_id.saturating_add(1);
        let yaw = self.bike.yaw + std::f32::consts::PI;
        let launch_position = WorldPosition {
            x: self.bike.x - yaw.sin() * riverside_common::GUNNER_AIM_ORIGIN_REAR_M,
            y: riverside_common::GUNNER_AIM_ORIGIN_HEIGHT_M,
            z: self.bike.z - yaw.cos() * riverside_common::GUNNER_AIM_ORIGIN_REAR_M,
        };
        self.gunner_ammo = self.gunner_ammo.saturating_sub(1);
        self.fire_cooldown_until_tick = self.tick + GUNNER_FIRE_COOLDOWN_TICKS;
        self.player_missile = Some(PlayerMissileSnapshot {
            attack_id,
            launch_position,
            position: launch_position,
            launch_tick: self.tick,
            expires_tick: self.tick + PLAYER_MISSILE_LIFETIME_TICKS,
        });
        self.clear_gunner_lock();
        self.push_combat_event(
            attack_id,
            CombatEventKind::GunnerFireAccepted {
                request_id,
                ammo_remaining: self.gunner_ammo,
            },
            launch_position,
        );
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
        self.gunner_input = GunnerInput::default();
        self.last_gunner_input_at = None;
        self.clear_gunner_lock();
        true
    }

    fn receive(&mut self, player_id: u64, seat: SeatKind, message: ClientMessage) {
        let phase = self.phase;
        if let ClientMessage::FireRequest { request_id, tick } = &message {
            let authorized = seat == SeatKind::Gunner
                && self
                    .solo_player_id
                    .is_none_or(|_| self.solo_seat == Some(seat))
                && self
                    .seats
                    .get(&SeatKind::Gunner)
                    .is_some_and(|slot| slot.player_id == Some(player_id));
            if authorized {
                self.handle_fire_request(*request_id, *tick, Instant::now());
            }
            return;
        }
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
                        normalize_yaw(input.yaw)
                    } else {
                        0.0
                    };
                    input.pitch = if input.pitch.is_finite() {
                        input.pitch.clamp(
                            riverside_common::GUNNER_PITCH_MIN_RADIANS,
                            riverside_common::GUNNER_PITCH_MAX_RADIANS,
                        )
                    } else {
                        0.0
                    };
                    self.gunner_input = input;
                    self.last_gunner_input_at = Some(Instant::now());
                    slot.last_sequence = sequence;
                    if !input.aim_held {
                        self.clear_gunner_lock();
                    }
                }
            }
            ClientMessage::ForceStartSolo
            | ClientMessage::SwitchSeat { .. }
            | ClientMessage::RiderInput { .. }
            | ClientMessage::GunnerInput { .. }
            | ClientMessage::FireRequest { .. } => {}
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

fn normalize_yaw(yaw: f32) -> f32 {
    (yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
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

fn move_helicopter_toward(current: &mut WorldPosition, target: WorldPosition, max_distance: f32) {
    let mut next = *current;
    move_toward(&mut next, target, max_distance);
    if first_cover_intersection(*current, next).is_none() {
        *current = next;
    }
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
    use super::{Room, input_tick_is_valid, move_helicopter_toward};
    use riverside_common::{
        BIKE_ROCKET_DAMAGE, BIKE_STARTING_HEALTH, BRIDGE_DECK_BOTTOM_M, BikeState, ClientMessage,
        CombatEventKind, CoverKind, FireRejectionReason, GUNNER_AIM_ORIGIN_HEIGHT_M,
        GUNNER_AIM_ORIGIN_REAR_M, GUNNER_LOCK_GRACE_TICKS, GUNNER_LOCK_TICKS, GUNNER_STARTING_AMMO,
        GamePhase, GunnerInput, HELICOPTER_CHASE_TICKS, HELICOPTER_STARTING_HEALTH,
        HelicopterPhase, MatchResult, MatchVictoryReason, PLAYER_MISSILE_DAMAGE,
        PlayerMissileSnapshot, REQUIRED_SEATS, ROCKET_BLAST_RADIUS_M, ROCKET_FLIGHT_TICKS,
        ROCKET_WARNING_TICKS, RiderInput, RocketSnapshot, RocketWarning, SIMULATION_HZ,
        WorldPosition, riverside_course,
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
        if room.gunner_input.aim_held {
            room.last_gunner_input_at = Some(*now);
        }
        let _ = room.advance_tick(*now);
    }

    fn bike_position_near_gate(distance_m: f32, along_m: f32) -> (f32, f32) {
        let gate = riverside_course().point_at(distance_m);
        (
            gate.center_x + gate.yaw.sin() * along_m,
            gate.center_z - gate.yaw.cos() * along_m,
        )
    }

    fn cross_room_gate(room: &mut Room, distance_m: f32) {
        let before = bike_position_near_gate(distance_m, -0.3);
        let after = bike_position_near_gate(distance_m, 0.3);
        room.bike.x = before.0;
        room.bike.z = before.1;
        let previous_bike = room.bike;
        room.bike.x = after.0;
        room.bike.z = after.1;
        room.update_route_progress(previous_bike);
    }

    fn route_angle_delta(from: f32, to: f32) -> f32 {
        (to - from + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
    }

    #[test]
    fn escape_requires_forward_ordered_checkpoints_and_crossing_the_finish() {
        let course = riverside_course();
        let [first_checkpoint, second_checkpoint] = course.checkpoints_m() else {
            panic!("the course must have two ordered checkpoints");
        };
        let mut room = running_room();

        // A later checkpoint or the finish cannot substitute for the first.
        cross_room_gate(&mut room, *second_checkpoint);
        assert_eq!(room.route_checkpoints_passed, 0);
        cross_room_gate(&mut room, course.finish_distance_m());
        assert!(!room.escape_finish_crossed);

        // Crossing checkpoint one in reverse does not advance route state.
        let after = bike_position_near_gate(*first_checkpoint, 0.3);
        let before = bike_position_near_gate(*first_checkpoint, -0.3);
        room.bike.x = after.0;
        room.bike.z = after.1;
        let previous_bike = room.bike;
        room.bike.x = before.0;
        room.bike.z = before.1;
        room.update_route_progress(previous_bike);
        assert_eq!(room.route_checkpoints_passed, 0);

        cross_room_gate(&mut room, *first_checkpoint);
        assert_eq!(room.route_checkpoints_passed, 1);
        cross_room_gate(&mut room, *second_checkpoint);
        assert_eq!(room.route_checkpoints_passed, 2);

        let finish_before = bike_position_near_gate(course.finish_distance_m(), -0.3);
        room.bike.x = finish_before.0;
        room.bike.z = finish_before.1;
        let previous_bike = room.bike;
        room.update_route_progress(previous_bike);
        room.finish_if_needed();
        assert_eq!(room.phase, GamePhase::Running);

        cross_room_gate(&mut room, course.finish_distance_m());
        room.finish_if_needed();
        assert_eq!(room.phase, GamePhase::Finished);
        assert_eq!(room.match_result, Some(MatchResult::RidersVictory));
        assert_eq!(room.match_victory_reason, Some(MatchVictoryReason::Escaped));
        assert!(room.recent_combat_events.iter().any(|event| matches!(
            event.kind,
            CombatEventKind::MatchFinished {
                result: MatchResult::RidersVictory,
                victory_reason: Some(MatchVictoryReason::Escaped)
            }
        )));
    }

    #[test]
    fn controlled_authoritative_route_completes_an_escape() {
        let course = riverside_course();
        let mut room = running_room();
        room.helicopter = None;
        let mut now = Instant::now();
        for _ in 0..SIMULATION_HZ * 180 {
            let projection = course.project(room.bike.x, room.bike.z);
            if room.phase == GamePhase::Finished {
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
            let desired_yaw = (target_x - room.bike.x).atan2(room.bike.z - target_z);
            let heading_error = route_angle_delta(room.bike.yaw, desired_yaw);
            let desired_curvature = course.curvature_at(projection.distance_m + 2.0);
            let steer = ((desired_curvature + heading_error * 0.65) / 0.148).clamp(-1.0, 1.0);
            step_room(
                &mut room,
                &mut now,
                RiderInput {
                    pedal_held: true,
                    steer,
                    ..RiderInput::default()
                },
            );
        }

        assert_eq!(room.phase, GamePhase::Finished);
        assert_eq!(room.route_checkpoints_passed, 2);
        assert_eq!(room.match_result, Some(MatchResult::RidersVictory));
        assert_eq!(room.match_victory_reason, Some(MatchVictoryReason::Escaped));
        eprintln!(
            "authoritative escape finished at {:.1} m in {:.1} s ({} ticks)",
            room.route_distance_m,
            room.tick as f32 / SIMULATION_HZ as f32,
            room.tick
        );
    }

    #[test]
    fn delayed_twenty_hz_rider_control_completes_authoritative_escape() {
        let course = riverside_course();
        let mut room = running_room();
        room.helicopter = None;
        let mut now = Instant::now();
        let mut history = vec![room.bike];
        let mut delayed = std::collections::VecDeque::new();
        let mut input = RiderInput::default();
        for elapsed_tick in 0..SIMULATION_HZ * 180 {
            if elapsed_tick.is_multiple_of(3) {
                // Observe a 20 Hz snapshot after one 50 ms transit, calculate
                // steering, then apply the next input after its 50 ms transit.
                let observed_index = elapsed_tick.saturating_sub(3) as usize;
                let observed_bike = history[observed_index];
                let projection = course.project(observed_bike.x, observed_bike.z);
                let lookahead =
                    course.point_at((projection.distance_m + 6.0).min(course.length_m()));
                let detour = if (30.0..40.0).contains(&projection.distance_m) {
                    0.75
                } else {
                    0.0
                };
                let target_x = lookahead.center_x + lookahead.yaw.cos() * detour;
                let target_z = lookahead.center_z + lookahead.yaw.sin() * detour;
                let desired_yaw = (target_x - observed_bike.x).atan2(observed_bike.z - target_z);
                let heading_error = route_angle_delta(observed_bike.yaw, desired_yaw);
                let curvature = course.curvature_at(projection.distance_m + 2.0);
                let steer = ((curvature + heading_error * 0.65) / 0.148).clamp(-1.0, 1.0);
                delayed.push_back((
                    elapsed_tick + 3,
                    RiderInput {
                        pedal_held: true,
                        steer,
                        ..RiderInput::default()
                    },
                ));
            }
            if delayed
                .front()
                .is_some_and(|(ready_tick, _)| *ready_tick <= elapsed_tick)
            {
                input = delayed.pop_front().unwrap().1;
            }
            step_room(&mut room, &mut now, input);
            history.push(room.bike);
            if room.phase == GamePhase::Finished {
                break;
            }
        }

        assert_eq!(room.phase, GamePhase::Finished);
        assert_eq!(room.route_checkpoints_passed, 2);
        assert_eq!(room.match_result, Some(MatchResult::RidersVictory));
        assert_eq!(room.match_victory_reason, Some(MatchVictoryReason::Escaped));
        eprintln!(
            "20 Hz Rider control with 100 ms input/feedback delay escaped {:.1} m in {:.1} s ({} ticks)",
            room.route_distance_m,
            room.tick as f32 / SIMULATION_HZ as f32,
            room.tick
        );
    }

    fn gunner_input_for_helicopter(room: &Room, aim_held: bool) -> GunnerInput {
        let target = room.helicopter.expect("running helicopter").position;
        let bike = room.bike;
        let home_yaw = bike.yaw + std::f32::consts::PI;
        let origin = WorldPosition {
            x: bike.x - home_yaw.sin() * GUNNER_AIM_ORIGIN_REAR_M,
            y: GUNNER_AIM_ORIGIN_HEIGHT_M,
            z: bike.z - home_yaw.cos() * GUNNER_AIM_ORIGIN_REAR_M,
        };
        let dx = target.x - origin.x;
        let dy = target.y - origin.y;
        let dz = target.z - origin.z;
        let target_yaw = (-dx).atan2(-dz);
        GunnerInput {
            yaw: target_yaw - bike.yaw - std::f32::consts::PI,
            pitch: dy.atan2(dx.hypot(dz)),
            aim_held,
        }
    }

    fn force_locked_aim(room: &mut Room) {
        room.gunner_input = gunner_input_for_helicopter(room, true);
        room.last_gunner_input_at = Some(Instant::now());
        room.gunner_lock_progress_ticks = GUNNER_LOCK_TICKS;
        room.gunner_lock_outside_since_tick = None;
    }

    fn run_missile_until_impact(room: &mut Room) {
        for _ in 0..super::PLAYER_MISSILE_LIFETIME_TICKS {
            if room.player_missile.is_none() {
                break;
            }
            room.tick += 1;
            room.step_player_missile();
        }
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
        assert_eq!(room.recent_combat_events.len(), 4);
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

    #[test]
    fn server_lock_needs_two_seconds_and_applies_the_short_grace_window() {
        let mut now = Instant::now();
        let mut room = running_room();
        room.gunner_input = gunner_input_for_helicopter(&room, true);
        for _ in 0..GUNNER_LOCK_TICKS {
            step_room(&mut room, &mut now, RiderInput::default());
        }
        assert_eq!(room.gunner_lock_progress_ticks, GUNNER_LOCK_TICKS);
        assert!(room.gunner_can_fire(now));

        room.gunner_input.yaw += 0.2;
        for _ in 0..GUNNER_LOCK_GRACE_TICKS {
            step_room(&mut room, &mut now, RiderInput::default());
        }
        assert_eq!(room.gunner_lock_progress_ticks, GUNNER_LOCK_TICKS);
        assert!(room.gunner_lock_outside_since_tick.is_some());
        assert!(!room.gunner_can_fire(now));

        room.gunner_input = gunner_input_for_helicopter(&room, true);
        step_room(&mut room, &mut now, RiderInput::default());
        assert_eq!(room.gunner_lock_progress_ticks, GUNNER_LOCK_TICKS);
        assert!(room.gunner_lock_outside_since_tick.is_none());

        room.gunner_input.yaw += 0.2;
        for _ in 0..(GUNNER_LOCK_GRACE_TICKS + 2) {
            step_room(&mut room, &mut now, RiderInput::default());
        }
        assert_eq!(room.gunner_lock_progress_ticks, 0);
        assert!(room.gunner_lock_outside_since_tick.is_none());
    }

    #[test]
    fn releasing_aim_clears_authoritative_lock_immediately() {
        let mut room = running_room();
        force_locked_aim(&mut room);
        room.receive(
            2,
            super::SeatKind::Gunner,
            ClientMessage::GunnerInput {
                sequence: 1,
                tick: room.tick,
                input: gunner_input_for_helicopter(&room, false),
            },
        );
        assert_eq!(room.gunner_lock_progress_ticks, 0);
        assert!(room.gunner_lock_outside_since_tick.is_none());
    }

    #[test]
    fn fire_request_is_authorized_deduplicated_and_only_success_spends_ammo() {
        let mut room = running_room();
        force_locked_aim(&mut room);
        let tick = room.tick;
        room.receive(
            2,
            super::SeatKind::Gunner,
            ClientMessage::FireRequest {
                request_id: 1,
                tick,
            },
        );
        assert_eq!(room.gunner_ammo, GUNNER_STARTING_AMMO - 1);
        assert_eq!(room.player_missile.unwrap().attack_id, 1);
        assert_eq!(room.gunner_lock_progress_ticks, 0);
        assert!(room.recent_combat_events.iter().any(|event| matches!(
            event.kind,
            CombatEventKind::GunnerFireAccepted {
                request_id: 1,
                ammo_remaining: 5
            }
        )));

        let event_count = room.recent_combat_events.len();
        room.receive(
            2,
            super::SeatKind::Gunner,
            ClientMessage::FireRequest {
                request_id: 1,
                tick,
            },
        );
        assert_eq!(room.gunner_ammo, GUNNER_STARTING_AMMO - 1);
        assert_eq!(room.next_attack_id, 2);
        assert_eq!(room.recent_combat_events.len(), event_count);

        room.receive(
            2,
            super::SeatKind::Gunner,
            ClientMessage::FireRequest {
                request_id: 2,
                tick: room.tick,
            },
        );
        assert!(room.recent_combat_events.iter().any(|event| matches!(
            event.kind,
            CombatEventKind::GunnerFireRejected {
                request_id: 2,
                reason: FireRejectionReason::Cooldown
            }
        )));
        assert_eq!(room.gunner_ammo, GUNNER_STARTING_AMMO - 1);

        room.player_missile = None;
        room.fire_cooldown_until_tick = room.tick;
        room.gunner_ammo = 0;
        force_locked_aim(&mut room);
        room.receive(
            2,
            super::SeatKind::Gunner,
            ClientMessage::FireRequest {
                request_id: 3,
                tick: room.tick,
            },
        );
        assert!(room.recent_combat_events.iter().any(|event| matches!(
            event.kind,
            CombatEventKind::GunnerFireRejected {
                request_id: 3,
                reason: FireRejectionReason::NoAmmo
            }
        )));
        assert_eq!(room.player_missile, None);
    }

    #[test]
    fn untrusted_fire_requests_cannot_bypass_seat_phase_lock_or_tick_checks() {
        let mut room = running_room();
        let tick = room.tick;
        room.receive(
            1,
            super::SeatKind::Rider,
            ClientMessage::FireRequest {
                request_id: 40,
                tick,
            },
        );
        assert!(room.recent_combat_events.is_empty());
        assert_eq!(room.gunner_ammo, GUNNER_STARTING_AMMO);

        room.tick = 1_000;
        force_locked_aim(&mut room);
        room.receive(
            2,
            super::SeatKind::Gunner,
            ClientMessage::FireRequest {
                request_id: 1,
                tick: room.tick.saturating_sub(super::MAX_INPUT_TICK_AGE + 1),
            },
        );
        assert!(room.recent_combat_events.iter().any(|event| matches!(
            event.kind,
            CombatEventKind::GunnerFireRejected {
                request_id: 1,
                reason: FireRejectionReason::StaleInput
            }
        )));
        assert_eq!(room.gunner_ammo, GUNNER_STARTING_AMMO);

        room.gunner_input = gunner_input_for_helicopter(&room, false);
        room.last_gunner_input_at = Some(Instant::now());
        room.receive(
            2,
            super::SeatKind::Gunner,
            ClientMessage::FireRequest {
                request_id: 2,
                tick: room.tick,
            },
        );
        assert!(room.recent_combat_events.iter().any(|event| matches!(
            event.kind,
            CombatEventKind::GunnerFireRejected {
                request_id: 2,
                reason: FireRejectionReason::NotAiming
            }
        )));

        room.phase = GamePhase::Finished;
        room.match_result = Some(MatchResult::RidersDefeat);
        room.receive(
            2,
            super::SeatKind::Gunner,
            ClientMessage::FireRequest {
                request_id: 3,
                tick: room.tick,
            },
        );
        assert!(room.recent_combat_events.iter().any(|event| matches!(
            event.kind,
            CombatEventKind::GunnerFireRejected {
                request_id: 3,
                reason: FireRejectionReason::NotRunning
            }
        )));
        assert_eq!(room.gunner_ammo, GUNNER_STARTING_AMMO);
    }

    #[test]
    fn fire_rejects_out_of_cone_out_of_range_and_obstructed_targets() {
        let mut room = running_room();
        force_locked_aim(&mut room);
        room.gunner_lock_progress_ticks = 0;
        room.gunner_input.yaw += 0.2;
        room.receive(
            2,
            super::SeatKind::Gunner,
            ClientMessage::FireRequest {
                request_id: 1,
                tick: room.tick,
            },
        );
        assert!(room.recent_combat_events.iter().any(|event| matches!(
            event.kind,
            CombatEventKind::GunnerFireRejected {
                request_id: 1,
                reason: FireRejectionReason::TargetOutsideCone
            }
        )));

        let mut helicopter = room.helicopter.unwrap();
        helicopter.position.z = room.bike.z + 160.0;
        room.helicopter = Some(helicopter);
        room.gunner_input = gunner_input_for_helicopter(&room, true);
        room.last_gunner_input_at = Some(Instant::now());
        room.receive(
            2,
            super::SeatKind::Gunner,
            ClientMessage::FireRequest {
                request_id: 2,
                tick: room.tick,
            },
        );
        assert!(room.recent_combat_events.iter().any(|event| matches!(
            event.kind,
            CombatEventKind::GunnerFireRejected {
                request_id: 2,
                reason: FireRejectionReason::OutOfRange
            }
        )));

        room.bike.z = -44.0;
        helicopter.position = WorldPosition {
            x: -0.55,
            y: 0.5,
            z: -30.0,
        };
        room.helicopter = Some(helicopter);
        room.gunner_input = gunner_input_for_helicopter(&room, true);
        room.last_gunner_input_at = Some(Instant::now());
        room.receive(
            2,
            super::SeatKind::Gunner,
            ClientMessage::FireRequest {
                request_id: 3,
                tick: room.tick,
            },
        );
        assert!(room.recent_combat_events.iter().any(|event| matches!(
            event.kind,
            CombatEventKind::GunnerFireRejected {
                request_id: 3,
                reason: FireRejectionReason::Obstructed
            }
        )));
        assert_eq!(room.gunner_ammo, GUNNER_STARTING_AMMO);
        assert_eq!(room.player_missile, None);
    }

    #[test]
    fn bridge_deck_clears_gunner_lock_cancels_warning_and_blocks_fire() {
        let mut room = running_room();
        let deck = riverside_course()
            .cover_volumes()
            .iter()
            .find(|cover| cover.kind == CoverKind::BridgeDeck)
            .copied()
            .unwrap();
        room.bike = BikeState {
            x: deck.center_x,
            z: deck.center_z,
            ..BikeState::default()
        };
        let mut helicopter = room.helicopter.unwrap();
        helicopter.position = WorldPosition {
            x: deck.center_x,
            y: 8.0,
            z: deck.center_z,
        };
        room.helicopter = Some(helicopter);
        room.gunner_input = gunner_input_for_helicopter(&room, true);
        room.last_gunner_input_at = Some(Instant::now());
        room.gunner_lock_progress_ticks = GUNNER_LOCK_TICKS;
        room.update_gunner_lock();
        assert_eq!(room.gunner_lock_progress_ticks, 0);
        assert_eq!(
            room.fire_rejection_reason(room.tick, Instant::now()),
            Some(FireRejectionReason::Obstructed)
        );
        room.handle_fire_request(1, room.tick, Instant::now());
        assert!(room.recent_combat_events.iter().any(|event| matches!(
            event.kind,
            CombatEventKind::GunnerFireRejected {
                request_id: 1,
                reason: FireRejectionReason::Obstructed
            }
        )));

        helicopter.phase = HelicopterPhase::Telegraph;
        helicopter.warning = Some(RocketWarning {
            attack_id: 500,
            impact_position: WorldPosition {
                x: room.bike.x,
                y: 0.0,
                z: room.bike.z,
            },
            impact_tick: room.tick + ROCKET_WARNING_TICKS + ROCKET_FLIGHT_TICKS,
            blast_radius_m: ROCKET_BLAST_RADIUS_M,
        });
        room.helicopter = Some(helicopter);
        room.helicopter_phase_deadline_tick = room.tick;
        room.step_helicopter();
        let helicopter = room.helicopter.unwrap();
        assert_eq!(helicopter.phase, HelicopterPhase::Repositioning);
        assert!(helicopter.warning.is_none());
        assert!(room.rocket.is_none());
    }

    #[test]
    fn projectiles_hit_the_bridge_deck_before_the_bike_or_helicopter() {
        let deck = riverside_course()
            .cover_volumes()
            .iter()
            .find(|cover| cover.kind == CoverKind::BridgeDeck)
            .copied()
            .unwrap();
        let mut room = running_room();
        room.bike = BikeState {
            x: deck.center_x,
            z: deck.center_z,
            ..BikeState::default()
        };
        let mut helicopter = room.helicopter.unwrap();
        helicopter.position = WorldPosition {
            x: deck.center_x,
            y: 8.0,
            z: deck.center_z,
        };
        helicopter.phase = HelicopterPhase::RocketInFlight;
        room.helicopter = Some(helicopter);
        room.rocket = Some(RocketSnapshot {
            attack_id: 501,
            launch_position: helicopter.position,
            position: helicopter.position,
            impact_position: WorldPosition {
                x: deck.center_x,
                y: 0.0,
                z: deck.center_z,
            },
            launch_tick: room.tick,
            impact_tick: room.tick + 60,
        });
        for _ in 0..60 {
            room.tick += 1;
            room.step_helicopter();
            if room.rocket.is_none() {
                break;
            }
        }
        assert!(room.rocket.is_none());
        assert_eq!(room.bike_health, BIKE_STARTING_HEALTH);
        assert_eq!(room.rocket_hit_count, 0);
        assert!(room.recent_combat_events.iter().any(|event| matches!(
            event.kind,
            CombatEventKind::RocketExploded {
                hit: false,
                hit_count: 0
            }
        )));

        room.player_missile = Some(PlayerMissileSnapshot {
            attack_id: 502,
            launch_position: WorldPosition {
                x: deck.center_x,
                y: BRIDGE_DECK_BOTTOM_M - 0.5,
                z: deck.center_z,
            },
            position: WorldPosition {
                x: deck.center_x,
                y: BRIDGE_DECK_BOTTOM_M - 0.5,
                z: deck.center_z,
            },
            launch_tick: room.tick,
            expires_tick: room.tick + 90,
        });
        for _ in 0..90 {
            room.tick += 1;
            room.step_player_missile();
            if room.player_missile.is_none() {
                break;
            }
        }
        assert!(room.player_missile.is_none());
        assert_eq!(room.helicopter_health, HELICOPTER_STARTING_HEALTH);
        assert!(room.recent_combat_events.iter().any(|event| matches!(
            event.kind,
            CombatEventKind::PlayerMissileImpacted {
                hit: false,
                helicopter_health: HELICOPTER_STARTING_HEALTH
            }
        )));
    }

    #[test]
    fn helicopter_does_not_move_through_bridge_geometry() {
        let deck = riverside_course()
            .cover_volumes()
            .iter()
            .find(|cover| cover.kind == CoverKind::BridgeDeck)
            .copied()
            .unwrap();
        let start = WorldPosition {
            x: deck.center_x + 10.0,
            y: (deck.min_y + deck.max_y) * 0.5,
            z: deck.center_z,
        };
        let mut helicopter = start;
        move_helicopter_toward(
            &mut helicopter,
            WorldPosition {
                x: deck.center_x - 10.0,
                ..start
            },
            30.0,
        );
        assert_eq!(helicopter, start);
    }

    #[test]
    fn player_missiles_apply_exact_damage_and_three_hits_win_once() {
        let mut room = running_room();
        for request_id in 1..=3 {
            force_locked_aim(&mut room);
            room.fire_cooldown_until_tick = room.tick;
            room.handle_fire_request(request_id, room.tick, Instant::now());
            run_missile_until_impact(&mut room);
            assert_eq!(
                room.helicopter_health,
                HELICOPTER_STARTING_HEALTH
                    .saturating_sub(request_id as u16 * PLAYER_MISSILE_DAMAGE)
            );
            if request_id < 3 {
                assert_eq!(room.phase, GamePhase::Running);
            }
        }
        room.finish_if_needed();
        assert_eq!(room.phase, GamePhase::Finished);
        assert_eq!(room.match_result, Some(MatchResult::RidersVictory));
        assert_eq!(room.gunner_ammo, GUNNER_STARTING_AMMO - 3);
        assert_eq!(room.player_missile, None);
        assert!(room.seats.values().all(|slot| !slot.ready));
        assert_eq!(
            room.recent_combat_events
                .iter()
                .filter(|event| matches!(
                    event.kind,
                    CombatEventKind::MatchFinished {
                        result: MatchResult::RidersVictory,
                        victory_reason: Some(MatchVictoryReason::HelicopterDestroyed),
                    }
                ))
                .count(),
            1
        );
    }

    #[test]
    fn rocket_damage_and_simultaneous_finish_use_defeat_priority() {
        let mut room = running_room();
        room.bike_health = BIKE_ROCKET_DAMAGE;
        room.helicopter_health = PLAYER_MISSILE_DAMAGE;
        room.route_checkpoints_passed = riverside_course().checkpoints_m().len();
        room.escape_finish_crossed = true;
        let helicopter = room.helicopter.unwrap();
        room.helicopter = Some(super::HelicopterSnapshot {
            phase: HelicopterPhase::RocketInFlight,
            ..helicopter
        });
        room.rocket = Some(super::RocketSnapshot {
            attack_id: 77,
            launch_position: helicopter.position,
            position: helicopter.position,
            impact_position: WorldPosition {
                x: room.bike.x,
                y: 0.0,
                z: room.bike.z,
            },
            launch_tick: room.tick,
            impact_tick: room.tick + 1,
        });
        room.player_missile = Some(super::PlayerMissileSnapshot {
            attack_id: 78,
            launch_position: helicopter.position,
            position: helicopter.position,
            launch_tick: room.tick,
            expires_tick: room.tick + 60,
        });
        let now = Instant::now();
        room.advance_tick(now);

        assert_eq!(room.phase, GamePhase::Finished);
        assert_eq!(room.bike_health, 0);
        assert_eq!(room.helicopter_health, 0);
        assert_eq!(room.match_result, Some(MatchResult::RidersDefeat));
        assert_eq!(room.match_victory_reason, None);
        assert!(room.rocket.is_none());
        assert!(room.player_missile.is_none());
        assert!(room.helicopter.is_none());
        assert!(room.recent_combat_events.iter().any(|event| matches!(
            event.kind,
            CombatEventKind::BikeDamaged {
                health_remaining: 0
            }
        )));
    }

    #[test]
    fn both_players_ready_after_finished_resets_battle_and_starts_a_five_second_countdown() {
        let mut room = running_room();
        room.phase = GamePhase::Finished;
        room.match_result = Some(MatchResult::RidersVictory);
        room.match_victory_reason = Some(MatchVictoryReason::Escaped);
        room.route_distance_m = 850.0;
        room.route_checkpoints_passed = 2;
        room.escape_finish_crossed = true;
        room.bike_health = 0;
        room.helicopter_health = 0;
        room.gunner_ammo = 1;
        room.gunner_lock_progress_ticks = GUNNER_LOCK_TICKS;
        room.rocket = Some(super::RocketSnapshot {
            attack_id: 10,
            launch_position: WorldPosition::default(),
            position: WorldPosition::default(),
            impact_position: WorldPosition::default(),
            launch_tick: room.tick,
            impact_tick: room.tick + 1,
        });
        room.player_missile = Some(super::PlayerMissileSnapshot {
            attack_id: 11,
            launch_position: WorldPosition::default(),
            position: WorldPosition::default(),
            launch_tick: room.tick,
            expires_tick: room.tick + 1,
        });
        for seat in REQUIRED_SEATS {
            room.seats.get_mut(&seat).unwrap().ready = false;
        }
        let now = Instant::now();
        room.receive(
            1,
            super::SeatKind::Rider,
            ClientMessage::SetReady { ready: true },
        );
        room.receive(
            2,
            super::SeatKind::Gunner,
            ClientMessage::SetReady { ready: true },
        );
        room.advance_tick(now);
        assert_eq!(room.phase, GamePhase::Countdown);
        assert_eq!(room.countdown_deadline, Some(now + Duration::from_secs(5)));
        assert_eq!(room.bike, BikeState::default());
        assert_eq!(room.bike_health, BIKE_STARTING_HEALTH);
        assert_eq!(room.helicopter_health, HELICOPTER_STARTING_HEALTH);
        assert_eq!(room.gunner_ammo, GUNNER_STARTING_AMMO);
        assert_eq!(room.gunner_lock_progress_ticks, 0);
        assert_eq!(room.rocket, None);
        assert_eq!(room.player_missile, None);
        assert_eq!(room.match_result, None);
        assert_eq!(room.match_victory_reason, None);
        assert_eq!(room.route_distance_m, 0.0);
        assert_eq!(room.route_checkpoints_passed, 0);
        assert!(!room.escape_finish_crossed);
        assert!(room.recent_combat_events.is_empty());
        assert!(REQUIRED_SEATS.iter().all(|seat| room.seats[seat].ready));

        room.advance_tick(now + Duration::from_secs(5));
        assert_eq!(room.phase, GamePhase::Running);
        assert_eq!(room.helicopter_health, HELICOPTER_STARTING_HEALTH);
        assert!(room.helicopter.is_some());
    }

    #[test]
    fn solo_role_change_clears_lock_without_refilling_health_or_ammo() {
        let mut room = running_room();
        room.solo_player_id = Some(42);
        room.solo_seat = Some(super::SeatKind::Gunner);
        for slot in room.seats.values_mut() {
            slot.player_id = Some(42);
        }
        room.bike_health = 50;
        room.helicopter_health = 66;
        room.gunner_ammo = 4;
        force_locked_aim(&mut room);

        assert!(room.switch_solo_seat(42, super::SeatKind::Rider));
        assert_eq!(room.gunner_lock_progress_ticks, 0);
        assert_eq!(room.gunner_input, GunnerInput::default());
        assert_eq!(room.bike_health, 50);
        assert_eq!(room.helicopter_health, 66);
        assert_eq!(room.gunner_ammo, 4);
    }
}
