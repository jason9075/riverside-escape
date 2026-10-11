use std::sync::{Mutex, mpsc};

use bevy::prelude::*;
use riverside_common::{ClientMessage, GamePhase, SeatKind, ServerMessage};

use crate::GameSession;

#[derive(Resource)]
pub struct NetworkClient {
    outgoing_tx: mpsc::Sender<ClientMessage>,
    #[allow(dead_code)]
    outgoing_rx: Mutex<mpsc::Receiver<ClientMessage>>,
    #[allow(dead_code)]
    incoming_tx: mpsc::Sender<ServerMessage>,
    incoming_rx: Mutex<mpsc::Receiver<ServerMessage>>,
    pub connected: bool,
    pub room_full: bool,
    pub solo_unavailable: bool,
    pub solo_mode: bool,
    pub player_id: Option<u64>,
    pub seat: Option<SeatKind>,
}

impl Default for NetworkClient {
    fn default() -> Self {
        let (outgoing_tx, outgoing_rx) = mpsc::channel();
        let (incoming_tx, incoming_rx) = mpsc::channel();
        Self {
            outgoing_tx,
            outgoing_rx: Mutex::new(outgoing_rx),
            incoming_tx,
            incoming_rx: Mutex::new(incoming_rx),
            connected: false,
            room_full: false,
            solo_unavailable: false,
            solo_mode: false,
            player_id: None,
            seat: None,
        }
    }
}

impl NetworkClient {
    pub fn send(&self, message: ClientMessage) {
        let _ = self.outgoing_tx.send(message);
    }
}

pub fn connect_browser(network: Res<NetworkClient>) {
    #[cfg(target_arch = "wasm32")]
    browser_socket::connect(network.incoming_tx.clone());

    #[cfg(not(target_arch = "wasm32"))]
    let _ = network;
}

pub fn pump_network(mut network: ResMut<NetworkClient>, mut session: ResMut<GameSession>) {
    #[cfg(target_arch = "wasm32")]
    {
        if browser_socket::ready_state().is_some_and(|state| state != web_sys::WebSocket::OPEN) {
            network.connected = false;
        }
    }

    let messages = network
        .incoming_rx
        .lock()
        .map(|receiver| receiver.try_iter().collect::<Vec<_>>())
        .unwrap_or_default();
    for message in messages {
        match message {
            ServerMessage::Welcome {
                player_id,
                seat,
                solo_mode,
            } => {
                network.connected = true;
                network.player_id = Some(player_id);
                network.seat = Some(seat);
                network.solo_mode = solo_mode;
                session.change_seat(seat);
                session.solo_mode = solo_mode;
                session.is_connected = true;
                session.ready = false;
                session.pending_rider_inputs.clear();
                session.last_applied_rider_sequence = 0;
            }
            ServerMessage::SeatChanged { seat } => {
                network.seat = Some(seat);
                session.change_seat(seat);
                session.rider_input = Default::default();
                session.gunner_input.aim_held = false;
                session.cursor_captured = false;
                session.pending_rider_inputs.clear();
            }
            ServerMessage::Snapshot(snapshot) => {
                let snapshot = *snapshot;
                let entering_running = snapshot.phase == GamePhase::Running
                    && (session.phase != GamePhase::Running || !session.prediction_initialized);
                session.is_connected = true;
                session.phase = snapshot.phase;
                session.apply_combat_snapshot(&snapshot);
                session.seats = snapshot.seats.clone();
                if let Some(seat) = network.seat {
                    session.ready = snapshot.seats.get(&seat).is_some_and(|status| status.ready);
                }
                session.countdown_seconds = snapshot.countdown_seconds;
                if entering_running {
                    session.begin_running(
                        snapshot.tick,
                        snapshot.bike,
                        snapshot.last_applied_rider_sequence,
                    );
                } else if snapshot.phase == GamePhase::Running {
                    session.queue_running_snapshot(snapshot);
                } else {
                    session.pending_rider_inputs.clear();
                    session.receive_prestart_snapshot(snapshot.tick, snapshot.bike);
                }
            }
            ServerMessage::RoomFull => {
                network.room_full = true;
                network.connected = false;
            }
            ServerMessage::SoloUnavailable => {
                network.solo_unavailable = true;
                network.connected = false;
            }
        }
    }
    if !network.connected {
        session.is_connected = false;
    }
}

pub fn flush_outgoing(network: Res<NetworkClient>) {
    #[cfg(target_arch = "wasm32")]
    browser_socket::flush(&network);

    #[cfg(not(target_arch = "wasm32"))]
    let _ = network;
}

#[cfg(target_arch = "wasm32")]
mod browser_socket {
    use std::{cell::RefCell, sync::mpsc};

    use bevy::prelude::*;
    use riverside_common::ServerMessage;
    use wasm_bindgen::{JsCast, closure::Closure};
    use web_sys::{MessageEvent, WebSocket};

    use super::NetworkClient;

    thread_local! {
        static SOCKET: RefCell<Option<WebSocket>> = const { RefCell::new(None) };
        static MESSAGE_CALLBACK: RefCell<Option<Closure<dyn FnMut(MessageEvent)>>> = const { RefCell::new(None) };
    }

    pub(super) fn connect(sender: mpsc::Sender<ServerMessage>) {
        let Some(window) = web_sys::window() else {
            error!("Browser window is unavailable; running in local preview mode.");
            return;
        };
        let location = window.location();
        let custom = location
            .search()
            .ok()
            .and_then(|search| web_sys::UrlSearchParams::new_with_str(&search).ok())
            .and_then(|params| params.get("server"));
        let requested_solo = location
            .search()
            .ok()
            .and_then(|search| web_sys::UrlSearchParams::new_with_str(&search).ok())
            .and_then(|params| params.get("solo"))
            .is_some_and(|value| value == "1");
        let mut address = custom.unwrap_or_else(|| {
            let scheme = if location.protocol().as_deref() == Ok("https:") {
                "wss"
            } else {
                "ws"
            };
            let host = location
                .hostname()
                .unwrap_or_else(|_| "127.0.0.1".to_owned());
            format!("{scheme}://{host}:3001/ws")
        });
        if requested_solo {
            address.push(if address.contains('?') { '&' } else { '?' });
            address.push_str("solo=1");
        }

        let socket = match WebSocket::new(&address) {
            Ok(socket) => socket,
            Err(_) => {
                error!("Could not open {address}; running in local preview mode.");
                return;
            }
        };
        let callback = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
            let Some(text) = event.data().as_string() else {
                return;
            };
            match serde_json::from_str::<ServerMessage>(&text) {
                Ok(message) => {
                    let _ = sender.send(message);
                }
                Err(error) => warn!("Ignoring malformed server message: {error}"),
            }
        });
        socket.set_onmessage(Some(callback.as_ref().unchecked_ref()));
        SOCKET.with(|current| *current.borrow_mut() = Some(socket));
        MESSAGE_CALLBACK.with(|current| *current.borrow_mut() = Some(callback));
        info!("Connecting to {address}");
    }

    pub(super) fn ready_state() -> Option<u16> {
        SOCKET.with(|socket| socket.borrow().as_ref().map(WebSocket::ready_state))
    }

    pub(super) fn flush(network: &NetworkClient) {
        let messages = network
            .outgoing_rx
            .lock()
            .map(|receiver| receiver.try_iter().collect::<Vec<_>>())
            .unwrap_or_default();
        if messages.is_empty() {
            return;
        }
        SOCKET.with(|current| {
            let current = current.borrow();
            let Some(socket) = current.as_ref() else {
                return;
            };
            if socket.ready_state() != WebSocket::OPEN {
                return;
            }
            for message in messages {
                if let Ok(text) = serde_json::to_string(&message) {
                    let _ = socket.send_with_str(&text);
                }
            }
        });
    }
}
