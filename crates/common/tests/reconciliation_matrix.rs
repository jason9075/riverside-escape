use std::collections::VecDeque;

use riverside_common::{
    BIKE_RENDER_CORRECTION_TAU_SECS, BikeRenderOffset, BikeState, FIXED_DT,
    INTERPOLATION_DELAY_TICKS, MAX_INPUT_TICK_AGE, MAX_INPUT_TICK_FUTURE, RiderInput,
    RiderInputSample, RiderPrediction, SIMULATION_HZ, SNAPSHOT_HZ, SnapshotInterpolator,
};

#[derive(Clone, Copy)]
struct InputPacket {
    deliver_at_tick: f64,
    sequence: u32,
    tick: u64,
    input: RiderInput,
}

#[derive(Clone, Copy)]
struct SnapshotPacket {
    deliver_at_tick: f64,
    tick: u64,
    bike: BikeState,
    ack: u32,
}

#[derive(Debug)]
struct MatrixResult {
    fps: u32,
    rtt_ms: u32,
    max_same_tick_error_m: f32,
    final_same_tick_error_m: f32,
    max_same_tick_yaw_error_deg: f32,
    final_same_tick_yaw_error_deg: f32,
    convergence_ms: Option<f64>,
    max_unacked: usize,
    sent: usize,
    acknowledged: usize,
    remaining_pending: usize,
    pending_sequences: Vec<u32>,
    final_server_ack: u32,
    rejected_input_count: usize,
    last_processed_snapshot_tick: u64,
    history_misses: usize,
    max_remote_lag_ms: f64,
    mean_remote_lag_ms: f64,
    max_client_frame_delta_ms: f64,
}

fn input_for_tick(tick: u64) -> RiderInput {
    if (60..90).contains(&tick) {
        RiderInput {
            pedal_held: true,
            steer: 1.0,
            ..default()
        }
    } else if (90..120).contains(&tick) {
        RiderInput {
            pedal_held: true,
            steer: -1.0,
            ..default()
        }
    } else if tick < 180 {
        RiderInput {
            pedal_held: true,
            ..default()
        }
    } else if tick < 300 {
        RiderInput {
            brake_held: true,
            ..default()
        }
    } else {
        RiderInput::default()
    }
}

fn valid_input_tick(server_tick: u64, input_tick: u64) -> bool {
    input_tick.saturating_add(MAX_INPUT_TICK_AGE) >= server_tick
        && input_tick <= server_tick.saturating_add(MAX_INPUT_TICK_FUTURE)
}

fn run_matrix_case(fps: u32, rtt_ms: u32, stall: Option<(u64, u64)>) -> MatrixResult {
    let frame_ticks = (SIMULATION_HZ / fps) as u64;
    let one_way_ticks = f64::from(rtt_ms) * f64::from(SIMULATION_HZ) / 2_000.0;
    let mut server_bike = BikeState::default();
    let mut server_input = RiderInput::default();
    let mut server_ack = 0;
    let mut last_input_received_tick = None;
    let mut client = RiderPrediction::new(0, BikeState::default());
    let mut pending_inputs = VecDeque::<RiderInputSample>::new();
    let mut input_queue = VecDeque::<InputPacket>::new();
    let mut snapshot_queue = VecDeque::<SnapshotPacket>::new();
    let mut gunner_interpolator = SnapshotInterpolator::default();
    let mut latest_snapshot_delivery = None;
    let mut interpolator_age_ticks = 0.0_f64;
    let mut sequence = 0_u32;
    let mut client_input_send_accumulator_ticks = 0.0_f64;
    let mut previous_client_update_tick = 0_u64;
    let mut sent = 0_usize;
    let mut acknowledged = 0_usize;
    let mut rejected_input_count = 0_usize;
    let mut last_processed_snapshot_tick = 0_u64;
    let mut max_unacked = 0_usize;
    let mut max_same_tick_error_m = 0.0_f32;
    let mut final_same_tick_error_m = f32::INFINITY;
    let mut max_same_tick_yaw_error_deg = 0.0_f32;
    let mut final_same_tick_yaw_error_deg = f32::INFINITY;
    let mut history_misses = 0_usize;
    let mut release_sequence = None;
    let mut release_applied_tick = None;
    let mut stable_after_release = 0_u8;
    let mut convergence_ms = None;
    let mut remote_lag_sum_ms = 0.0;
    let mut remote_lag_samples = 0_usize;
    let mut max_remote_lag_ms = 0.0_f64;
    let mut previous_render_tick = 0_u64;
    let mut max_client_frame_delta_ms = frame_ticks as f64 * 1_000.0 / f64::from(SIMULATION_HZ);

    // 14 seconds of fixed-step wall time includes a two-second settled drain
    // after input sending stops so every sequence can be acknowledged.
    let end_tick = SIMULATION_HZ as u64 * 14;
    for wall_tick in 1..=end_tick {
        let now = wall_tick as f64;
        while input_queue
            .front()
            .is_some_and(|packet| packet.deliver_at_tick <= now)
        {
            let packet = input_queue.pop_front().expect("front was present");
            if packet.sequence > server_ack {
                if valid_input_tick(wall_tick, packet.tick) {
                    server_ack = packet.sequence;
                    server_input = packet.input;
                    last_input_received_tick = Some(wall_tick);
                    if packet.tick >= 300 && packet.input == RiderInput::default() {
                        release_sequence.get_or_insert(packet.sequence);
                        release_applied_tick.get_or_insert(wall_tick);
                    }
                } else {
                    rejected_input_count += 1;
                }
            }
        }
        if last_input_received_tick.is_some_and(|received| wall_tick - received > 12) {
            server_input = RiderInput::default();
            last_input_received_tick = None;
        }
        server_bike.step(server_input, FIXED_DT);

        if wall_tick % (SIMULATION_HZ / SNAPSHOT_HZ) as u64 == 0 {
            snapshot_queue.push_back(SnapshotPacket {
                deliver_at_tick: now + one_way_ticks,
                tick: wall_tick,
                bike: server_bike,
                ack: server_ack,
            });
        }

        let in_stall = stall.is_some_and(|(start, end)| (start..end).contains(&wall_tick));
        if wall_tick % frame_ticks != 0 || in_stall {
            continue;
        }
        max_client_frame_delta_ms = max_client_frame_delta_ms
            .max((wall_tick - previous_render_tick) as f64 * 1_000.0 / f64::from(SIMULATION_HZ));
        previous_render_tick = wall_tick;

        let mut latest_delivery = None;
        let mut arrived_snapshots = Vec::new();
        while snapshot_queue
            .front()
            .is_some_and(|packet| packet.deliver_at_tick <= now)
        {
            let packet = snapshot_queue.pop_front().expect("front was present");
            latest_delivery = Some(packet.deliver_at_tick);
            gunner_interpolator.push(packet.tick, packet.bike);
            arrived_snapshots.push(packet);
        }

        let frame_input = input_for_tick(wall_tick);
        while client.current_tick() < wall_tick {
            let collision = client.step(frame_input);
            assert!(!collision, "controlled route unexpectedly hit an obstacle");
        }

        for packet in arrived_snapshots {
            last_processed_snapshot_tick = packet.tick;
            let outcome = client.reconcile(packet.tick, packet.bike, packet.ack, &pending_inputs);
            if let Some(error) = outcome.same_tick_position_error_m {
                final_same_tick_error_m = error;
                max_same_tick_error_m = max_same_tick_error_m.max(error);
                if packet.tick >= 300
                    && release_sequence.is_some_and(|released| packet.ack >= released)
                    && error <= 0.02
                {
                    stable_after_release = stable_after_release.saturating_add(1);
                    if stable_after_release >= 3 && convergence_ms.is_none() {
                        convergence_ms = Some(
                            (wall_tick.saturating_sub(release_applied_tick.unwrap_or(wall_tick)))
                                as f64
                                * 1_000.0
                                / f64::from(SIMULATION_HZ),
                        );
                    }
                } else {
                    stable_after_release = 0;
                }
            } else if packet.tick < client.current_tick() {
                history_misses += 1;
            }
            if let Some(error) = outcome.same_tick_yaw_error_deg {
                final_same_tick_yaw_error_deg = error;
                max_same_tick_yaw_error_deg = max_same_tick_yaw_error_deg.max(error);
            }

            let acked_here = pending_inputs
                .iter()
                .filter(|sample| sample.sequence <= packet.ack)
                .count();
            acknowledged += acked_here;
            pending_inputs.retain(|sample| sample.sequence > packet.ack);
        }
        if let Some(delivery) = latest_delivery {
            latest_snapshot_delivery = Some(delivery);
            interpolator_age_ticks = 0.0;
        }

        client_input_send_accumulator_ticks += (wall_tick - previous_client_update_tick) as f64;
        previous_client_update_tick = wall_tick;
        while client_input_send_accumulator_ticks + 1.0e-9 >= f64::from(SIMULATION_HZ / SNAPSHOT_HZ)
            && client.current_tick() <= 720
        {
            client_input_send_accumulator_ticks -= f64::from(SIMULATION_HZ / SNAPSHOT_HZ);
            sequence += 1;
            sent += 1;
            pending_inputs.push_back(RiderInputSample {
                sequence,
                tick: client.current_tick(),
                input: frame_input,
            });
            max_unacked = max_unacked.max(pending_inputs.len());
            input_queue.push_back(InputPacket {
                deliver_at_tick: now + one_way_ticks,
                sequence,
                tick: client.current_tick(),
                input: frame_input,
            });
        }

        if let Some(delivery_tick) = latest_snapshot_delivery {
            let age_ticks = now - delivery_tick;
            gunner_interpolator
                .advance_time((age_ticks - interpolator_age_ticks) / f64::from(SIMULATION_HZ));
            interpolator_age_ticks = age_ticks;
            if let Some(render_tick) = gunner_interpolator.render_tick() {
                let lag_ms = (now - render_tick) * 1_000.0 / f64::from(SIMULATION_HZ);
                remote_lag_sum_ms += lag_ms;
                remote_lag_samples += 1;
                max_remote_lag_ms = max_remote_lag_ms.max(lag_ms);
            }
        }
    }

    assert_eq!(client.current_tick(), end_tick);
    let mean_remote_lag_ms = if remote_lag_samples == 0 {
        0.0
    } else {
        remote_lag_sum_ms / remote_lag_samples as f64
    };
    MatrixResult {
        fps,
        rtt_ms,
        max_same_tick_error_m,
        final_same_tick_error_m,
        max_same_tick_yaw_error_deg,
        final_same_tick_yaw_error_deg,
        convergence_ms,
        max_unacked,
        sent,
        acknowledged,
        remaining_pending: pending_inputs.len(),
        pending_sequences: pending_inputs
            .iter()
            .map(|sample| sample.sequence)
            .collect(),
        final_server_ack: server_ack,
        rejected_input_count,
        last_processed_snapshot_tick,
        history_misses,
        max_remote_lag_ms,
        mean_remote_lag_ms,
        max_client_frame_delta_ms,
    }
}

#[test]
fn fixed_step_prediction_and_remote_interpolation_matrix() {
    println!(
        "FPS RTT  max same-tick position/yaw error  converge after release  max unacked  Acked  Gunner lag avg/max  max frame delta"
    );
    for fps in [15, 30, 60] {
        for rtt in [0, 50, 100, 200] {
            let result = run_matrix_case(fps, rtt, None);
            println!(
                "{:>2} {:>3}  {:>7.3} m / {:>6.2}°                {:>6?} ms              {:>2}       {}/{}   {:>6.1}/{:>6.1} ms      {:>6.1} ms",
                result.fps,
                result.rtt_ms,
                result.max_same_tick_error_m,
                result.max_same_tick_yaw_error_deg,
                result.convergence_ms.map(|ms| ms.round() as u64),
                result.max_unacked,
                result.acknowledged,
                result.sent,
                result.mean_remote_lag_ms,
                result.max_remote_lag_ms,
                result.max_client_frame_delta_ms,
            );
            assert_eq!(result.acknowledged, result.sent, "{result:?}");
            assert_eq!(result.remaining_pending, 0, "{result:?}");
            assert!(result.pending_sequences.is_empty(), "{result:?}");
            assert_eq!(result.final_server_ack, result.sent as u32, "{result:?}");
            assert_eq!(result.rejected_input_count, 0, "{result:?}");
            assert_eq!(result.history_misses, 0, "{result:?}");
            assert!(result.last_processed_snapshot_tick >= 720, "{result:?}");
            assert!(result.final_same_tick_error_m < 0.01, "{result:?}");
            assert!(result.final_same_tick_yaw_error_deg < 0.01, "{result:?}");
            assert!(result.convergence_ms.is_some(), "{result:?}");
            assert!(result.max_unacked > 0, "{result:?}");
            let expected_min_lag = f64::from(rtt) / 2.0 + 49.0;
            assert!(result.mean_remote_lag_ms >= expected_min_lag, "{result:?}");
            assert!(
                result.mean_remote_lag_ms <= expected_min_lag + 8.0,
                "{result:?}"
            );
        }
    }
}

#[test]
fn a_short_render_stall_catches_up_with_history_and_ack_intact() {
    let result = run_matrix_case(30, 100, Some((360, 372)));
    println!("short stall at 30 FPS / 100 ms RTT: {result:?}");
    assert_eq!(result.acknowledged, result.sent);
    assert_eq!(result.remaining_pending, 0);
    assert!(result.pending_sequences.is_empty());
    assert_eq!(result.final_server_ack, result.sent as u32);
    assert_eq!(result.rejected_input_count, 0);
    assert_eq!(result.history_misses, 0);
    assert!(result.final_same_tick_error_m < 0.01);
    assert!(result.max_client_frame_delta_ms >= 220.0);
    assert!(result.convergence_ms.is_some());
}

#[test]
fn reconciliation_restores_same_tick_then_replays_unconfirmed_held_input() {
    let start = BikeState::default();
    let pedal = RiderInput {
        pedal_held: true,
        steer: 0.5,
        ..default()
    };
    let mut predicted = RiderPrediction::new(100, start);
    for _ in 0..9 {
        predicted.step(pedal);
    }

    let mut authority_at_104 = start;
    for _ in 0..4 {
        authority_at_104.step(pedal, FIXED_DT);
    }
    let corrected_input = RiderInput {
        pedal_held: true,
        steer: -0.5,
        ..default()
    };
    let pending = VecDeque::from([RiderInputSample {
        sequence: 3,
        tick: 107,
        input: corrected_input,
    }]);
    let result = predicted.reconcile(104, authority_at_104, 2, &pending);
    assert!(result.same_tick_position_error_m.unwrap() < 0.001);
    assert_eq!(result.replayed_ticks, 5);
    assert_eq!(result.replayed_unacknowledged_inputs, 1);
    assert_eq!(predicted.current_tick(), 109);

    let mut expected = authority_at_104;
    for tick in 105..=109 {
        expected.step(if tick == 107 { corrected_input } else { pedal }, FIXED_DT);
    }
    assert_eq!(predicted.bike(), expected);
}

#[test]
fn stationary_steer_moves_the_fork_not_the_bicycle_body() {
    let mut bike = BikeState::default();
    let initial = bike;
    let turn_left = RiderInput {
        steer: -1.0,
        ..default()
    };
    for _ in 0..30 {
        bike.step(turn_left, FIXED_DT);
    }
    assert_eq!(bike.x, initial.x);
    assert_eq!(bike.z, initial.z);
    assert_eq!(bike.yaw, initial.yaw);
    assert!(bike.steering_angle < 0.0);
    assert!(bike.steering_angle < -0.4);

    let previous_yaw = bike.yaw;
    let pedal_turn = RiderInput {
        pedal_held: true,
        steer: -1.0,
        ..default()
    };
    for _ in 0..60 {
        bike.step(pedal_turn, FIXED_DT);
    }
    assert_ne!(bike.yaw, previous_yaw);
    assert!(bike.x.abs() > 0.01 || (bike.z - initial.z).abs() > 0.01);
}

#[test]
fn snapshot_interpolation_tracks_wall_time_with_fixed_buffer_delay() {
    let mut interpolation = SnapshotInterpolator::default();
    interpolation.push(
        120,
        BikeState {
            x: 0.0,
            ..default()
        },
    );
    interpolation.push(
        123,
        BikeState {
            x: 3.0,
            ..default()
        },
    );
    assert_eq!(interpolation.sample().unwrap().x, 0.0);
    assert_eq!(
        interpolation.estimated_lag_secs().unwrap(),
        INTERPOLATION_DELAY_TICKS as f64 / f64::from(SIMULATION_HZ)
    );
    interpolation.advance_time(0.025);
    assert!((interpolation.sample().unwrap().x - 1.5).abs() < 0.001);
    assert!((interpolation.estimated_lag_secs().unwrap() - 0.05).abs() < 1.0e-9);
}

#[test]
fn render_correction_converges_by_elapsed_time_at_each_frame_rate_and_after_a_stall() {
    let mut convergence_times = Vec::new();
    for fps in [15, 30, 60] {
        let mut offset = BikeRenderOffset {
            x: 1.0,
            z: -0.5,
            yaw: 0.7,
            speed: 1.0,
        };
        let frame_dt = 1.0 / fps as f32;
        let mut elapsed = 0.0_f32;
        while offset.x.abs() > 0.05 && elapsed < 1.0 {
            offset.advance(frame_dt);
            elapsed += frame_dt;
        }
        convergence_times.push(elapsed);
        assert!(offset.x.abs() <= 0.05);
        assert!(offset.z.abs() <= 0.025);
        assert!(offset.yaw.abs() <= 0.035);
    }
    assert!(
        convergence_times
            .iter()
            .all(|elapsed| (0.25..=0.29).contains(elapsed))
    );

    let mut after_stall = BikeRenderOffset {
        x: 1.0,
        ..default()
    };
    after_stall.advance(0.2);
    let expected_after_stall = (-0.2 / BIKE_RENDER_CORRECTION_TAU_SECS).exp();
    assert!((after_stall.x - expected_after_stall).abs() < 1.0e-6);
}

fn default<T: Default>() -> T {
    T::default()
}
