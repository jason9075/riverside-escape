use std::sync::{Mutex, mpsc};

use bevy::prelude::*;
use riverside_common::{
    ClientMessage, GamePhase, SeatChoiceRejection, SeatKind, SeatSwapCancellationReason,
    SeatSwapRejection, ServerMessage,
};

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
    pub room_limit_reached: bool,
    pub room_not_found: bool,
    pub room_selection_required: bool,
    pub invalid_room_code: bool,
    pub solo_unavailable: bool,
    pub reconnect_unavailable: bool,
    pub reconnect_in_use: bool,
    pub connection_lost: bool,
    pub reconnect_seconds_remaining: Option<u8>,
    pub reconnect_token: Option<String>,
    pub solo_mode: bool,
    pub player_id: Option<u64>,
    pub room_code: Option<String>,
    pub seat: Option<SeatKind>,
    pub seat_choice_error: Option<SeatChoiceRejection>,
    pub pending_seat_swap: Option<PendingSeatSwap>,
    pub seat_swap_status: Option<String>,
    pub unassigned_players: u8,
}

#[derive(Clone)]
#[allow(dead_code)] // Some fields are consumed only by the WASM Lobby renderer.
pub struct PendingSeatSwap {
    pub request_id: u64,
    pub requester_id: u64,
    pub target_id: u64,
    pub from_seat: SeatKind,
    pub to_seat: SeatKind,
    pub expires_in_seconds: u8,
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
            room_limit_reached: false,
            room_not_found: false,
            room_selection_required: false,
            invalid_room_code: false,
            solo_unavailable: false,
            reconnect_unavailable: false,
            reconnect_in_use: false,
            connection_lost: false,
            reconnect_seconds_remaining: None,
            reconnect_token: None,
            solo_mode: false,
            player_id: None,
            room_code: None,
            seat: None,
            seat_choice_error: None,
            pending_seat_swap: None,
            seat_swap_status: None,
            unassigned_players: 0,
        }
    }
}

impl NetworkClient {
    pub fn send(&self, message: ClientMessage) {
        let _ = self.outgoing_tx.send(message);
    }
}

fn seat_swap_rejection_text(reason: SeatSwapRejection) -> &'static str {
    match reason {
        SeatSwapRejection::SeatsNotOccupied => "兩個角色都有人選擇後才能交換。",
        SeatSwapRejection::AlreadyPending => "已有交換請求等待回覆。",
        SeatSwapRejection::MatchLocked => "倒數或對局中不能交換角色。",
        SeatSwapRejection::NoPendingRequest => "交換請求已失效。",
        SeatSwapRejection::NotSwapTarget => "只有收到請求的隊友可以回覆。",
    }
}

fn seat_swap_cancellation_text(reason: SeatSwapCancellationReason) -> &'static str {
    match reason {
        SeatSwapCancellationReason::Rejected => "隊友拒絕了角色交換。",
        SeatSwapCancellationReason::Expired => "角色交換請求已逾時。",
        SeatSwapCancellationReason::MemberLeft => "玩家離開，角色交換已取消。",
        SeatSwapCancellationReason::SeatChanged => "角色狀態已變更，交換請求已取消。",
    }
}

pub fn connect_browser(network: Res<NetworkClient>) {
    #[cfg(target_arch = "wasm32")]
    browser_socket::connect(network.incoming_tx.clone(), network.outgoing_tx.clone());

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
                room_code,
                seat,
                solo_mode,
                reconnect_token,
            } => {
                network.connected = true;
                network.connection_lost = false;
                network.reconnect_unavailable = false;
                network.reconnect_in_use = false;
                network.player_id = Some(player_id);
                network.room_code = Some(room_code);
                network.reconnect_token = reconnect_token;
                network.seat = seat;
                network.solo_mode = solo_mode;
                if let Some(seat) = seat {
                    session.change_seat(seat);
                }
                session.solo_mode = solo_mode;
                session.is_connected = true;
                session.ready = false;
                session.pending_rider_inputs.clear();
                session.last_applied_rider_sequence = 0;
            }
            ServerMessage::SeatAssigned { seat } => {
                network.seat = Some(seat);
                network.seat_choice_error = None;
                network.pending_seat_swap = None;
                network.seat_swap_status = None;
                session.change_seat(seat);
                session.rider_input = Default::default();
                session.gunner_input.aim_held = false;
            }
            ServerMessage::SeatChoiceRejected { seat: _, reason } => {
                network.seat_choice_error = Some(reason);
            }
            ServerMessage::SeatSwapProposed {
                request_id,
                requester_id,
                target_id,
                from_seat,
                to_seat,
                expires_in_seconds,
            } => {
                network.pending_seat_swap = Some(PendingSeatSwap {
                    request_id,
                    requester_id,
                    target_id,
                    from_seat,
                    to_seat,
                    expires_in_seconds,
                });
                network.seat_swap_status = Some(if network.player_id == Some(requester_id) {
                    format!("已提出交換：對方接受後會改選 {}。", to_seat.label())
                } else {
                    format!("隊友提出交換：接受後你會改選 {}。", from_seat.label())
                });
            }
            ServerMessage::SeatSwapProposalRejected { reason } => {
                network.seat_swap_status = Some(seat_swap_rejection_text(reason).to_owned());
            }
            ServerMessage::SeatSwapCancelled { request_id, reason } => {
                if network
                    .pending_seat_swap
                    .as_ref()
                    .is_some_and(|pending| pending.request_id == request_id)
                {
                    network.pending_seat_swap = None;
                    network.seat_swap_status = Some(seat_swap_cancellation_text(reason).to_owned());
                }
            }
            ServerMessage::SeatSwapCompleted {
                request_id: _,
                seat,
            } => {
                network.seat = Some(seat);
                network.pending_seat_swap = None;
                network.seat_swap_status = Some("角色已交換，請重新準備。".to_owned());
                session.change_seat(seat);
                session.rider_input = Default::default();
                session.gunner_input.aim_held = false;
                session.cursor_captured = false;
                session.pending_rider_inputs.clear();
                session.prediction_initialized = false;
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
                network.unassigned_players = snapshot.unassigned_players;
                network.reconnect_seconds_remaining = snapshot.reconnect_seconds_remaining;
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
            ServerMessage::RoomLimitReached => {
                network.room_limit_reached = true;
                network.connected = false;
            }
            ServerMessage::RoomNotFound => {
                network.room_not_found = true;
                network.connected = false;
            }
            ServerMessage::RoomSelectionRequired => {
                network.room_selection_required = true;
                network.connected = false;
            }
            ServerMessage::InvalidRoomCode => {
                network.invalid_room_code = true;
                network.connected = false;
            }
            ServerMessage::SoloUnavailable => {
                network.solo_unavailable = true;
                network.connected = false;
            }
            ServerMessage::ReconnectUnavailable => {
                network.reconnect_unavailable = true;
                network.connection_lost = false;
                network.connected = false;
            }
            ServerMessage::ReconnectInUse => {
                network.reconnect_in_use = true;
                network.connection_lost = false;
                network.connected = false;
            }
            ServerMessage::RoomLeft => {
                network.connected = false;
                network.connection_lost = false;
                network.reconnect_token = None;
                network.room_code = None;
            }
            ServerMessage::ConnectionLost => {
                network.connected = false;
                network.connection_lost = network.room_code.is_some();
                session.is_connected = false;
                if network.connection_lost {
                    session.phase = GamePhase::Paused;
                }
            }
        }
    }
    if !network.connected {
        session.is_connected = false;
    }
    #[cfg(target_arch = "wasm32")]
    browser_socket::render_lobby(&network, &session);
}

pub fn flush_outgoing(network: Res<NetworkClient>) {
    #[cfg(target_arch = "wasm32")]
    browser_socket::flush(&network);

    #[cfg(not(target_arch = "wasm32"))]
    let _ = network;
}

#[cfg(target_arch = "wasm32")]
mod browser_socket {
    use std::{
        cell::{Cell, RefCell},
        sync::mpsc,
    };

    use crate::GameSession;
    use bevy::prelude::*;
    use riverside_common::{
        ClientMessage, GamePhase, SeatChoiceRejection, SeatKind, ServerMessage,
    };
    use wasm_bindgen::{JsCast, closure::Closure};
    use web_sys::{Document, Event, HtmlInputElement, MessageEvent, Storage, WebSocket};

    use super::NetworkClient;

    thread_local! {
        static SOCKET: RefCell<Option<WebSocket>> = const { RefCell::new(None) };
        static MESSAGE_CALLBACK: RefCell<Option<Closure<dyn FnMut(MessageEvent)>>> = const { RefCell::new(None) };
        static UI_CALLBACKS: RefCell<Vec<Closure<dyn FnMut(Event)>>> = const { RefCell::new(Vec::new()) };
        static SOCKET_CALLBACKS: RefCell<Vec<Closure<dyn FnMut(Event)>>> = const { RefCell::new(Vec::new()) };
        static RETRY_CALLBACKS: RefCell<Vec<Closure<dyn FnMut()>>> = const { RefCell::new(Vec::new()) };
        static READY_STATE: Cell<bool> = const { Cell::new(false) };
        static PENDING_SWAP_ID: Cell<Option<u64>> = const { Cell::new(None) };
        static RETRY_ENABLED: Cell<bool> = const { Cell::new(true) };
        static RETRY_SCHEDULED: Cell<bool> = const { Cell::new(false) };
        static RECONNECT_ROOM: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    pub(super) fn connect(
        sender: mpsc::Sender<ServerMessage>,
        outgoing: mpsc::Sender<ClientMessage>,
    ) {
        install_lobby_handlers(outgoing.clone());
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
        let params = location
            .search()
            .ok()
            .and_then(|search| web_sys::UrlSearchParams::new_with_str(&search).ok());
        let requested_solo = params
            .as_ref()
            .and_then(|params| params.get("solo"))
            .is_some_and(|value| value == "1");
        let create_room = params
            .as_ref()
            .and_then(|params| params.get("create"))
            .is_some_and(|value| value == "1");
        let requested_room = params.as_ref().and_then(|params| params.get("room"));
        let active_room =
            storage().and_then(|storage| storage.get_item(ACTIVE_ROOM_KEY).ok().flatten());
        let room_to_reconnect = requested_room.clone().or_else(|| {
            (!requested_solo && create_room)
                .then_some(active_room)
                .flatten()
        });
        let base = custom.unwrap_or_else(|| {
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
        let reconnect_token = room_to_reconnect
            .as_deref()
            .and_then(|code| storage()?.get_item(&token_storage_key(code)).ok().flatten());
        let address =
            if let (Some(room_code), Some(token)) = (room_to_reconnect.clone(), reconnect_token) {
                RECONNECT_ROOM.with(|room| *room.borrow_mut() = Some(room_code.clone()));
                append_query(&base, &format!("room={room_code}&reconnect={token}"))
            } else if requested_solo {
                append_query(&base, "solo=1")
            } else if create_room {
                append_query(&base, "create=1")
            } else if let Some(room_code) = requested_room {
                RECONNECT_ROOM.with(|room| *room.borrow_mut() = Some(room_code.clone()));
                append_query(&base, &format!("room={room_code}"))
            } else {
                return;
            };
        RETRY_ENABLED.with(|enabled| enabled.set(true));
        open_socket(address, sender, outgoing);
    }

    const ACTIVE_ROOM_KEY: &str = "riverside-escape-active-room";

    fn append_query(base: &str, query: &str) -> String {
        format!(
            "{base}{}{query}",
            if base.contains('?') { '&' } else { '?' }
        )
    }

    fn token_storage_key(room_code: &str) -> String {
        format!("riverside-escape-reconnect:{room_code}")
    }

    fn storage() -> Option<Storage> {
        web_sys::window()?.session_storage().ok().flatten()
    }

    fn server_base() -> Option<String> {
        let window = web_sys::window()?;
        let location = window.location();
        let params = location
            .search()
            .ok()
            .and_then(|search| web_sys::UrlSearchParams::new_with_str(&search).ok());
        if let Some(custom) = params.as_ref().and_then(|params| params.get("server")) {
            return Some(custom);
        }
        let scheme = if location.protocol().as_deref() == Ok("https:") {
            "wss"
        } else {
            "ws"
        };
        let host = location
            .hostname()
            .unwrap_or_else(|_| "127.0.0.1".to_owned());
        Some(format!("{scheme}://{host}:3001/ws"))
    }

    fn clear_room_token(room_code: &str) {
        if let Some(storage) = storage() {
            let _ = storage.remove_item(&token_storage_key(room_code));
            if storage.get_item(ACTIVE_ROOM_KEY).ok().flatten().as_deref() == Some(room_code) {
                let _ = storage.remove_item(ACTIVE_ROOM_KEY);
            }
        }
    }

    fn store_welcome(room_code: &str, token: Option<&str>) {
        RECONNECT_ROOM.with(|room| *room.borrow_mut() = Some(room_code.to_owned()));
        if let Some(storage) = storage() {
            let _ = storage.set_item(ACTIVE_ROOM_KEY, room_code);
            if let Some(token) = token {
                let _ = storage.set_item(&token_storage_key(room_code), token);
            }
        }
    }

    fn open_socket(
        address: String,
        sender: mpsc::Sender<ServerMessage>,
        outgoing: mpsc::Sender<ClientMessage>,
    ) {
        let socket = match WebSocket::new(&address) {
            Ok(socket) => socket,
            Err(_) => {
                error!("Could not open {address}; running in local preview mode.");
                let _ = sender.send(ServerMessage::ConnectionLost);
                schedule_reconnect(sender, outgoing);
                return;
            }
        };
        let message_sender = sender.clone();
        let callback = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
            let Some(text) = event.data().as_string() else {
                return;
            };
            match serde_json::from_str::<ServerMessage>(&text) {
                Ok(message) => {
                    match &message {
                        ServerMessage::Welcome {
                            room_code,
                            reconnect_token,
                            ..
                        } => store_welcome(room_code, reconnect_token.as_deref()),
                        ServerMessage::ReconnectUnavailable => {
                            RETRY_ENABLED.with(|enabled| enabled.set(false));
                            RECONNECT_ROOM.with(|room| {
                                if let Some(room_code) = room.borrow().as_deref() {
                                    clear_room_token(room_code);
                                }
                            });
                        }
                        ServerMessage::ReconnectInUse => {
                            RETRY_ENABLED.with(|enabled| enabled.set(false));
                        }
                        ServerMessage::RoomLeft => {
                            RETRY_ENABLED.with(|enabled| enabled.set(false));
                            RECONNECT_ROOM.with(|room| {
                                if let Some(room_code) = room.borrow().as_deref() {
                                    clear_room_token(room_code);
                                }
                            });
                            if let Some(window) = web_sys::window() {
                                let _ = window.location().set_href("/");
                            }
                        }
                        ServerMessage::RoomFull
                        | ServerMessage::RoomLimitReached
                        | ServerMessage::RoomNotFound
                        | ServerMessage::RoomSelectionRequired
                        | ServerMessage::InvalidRoomCode
                        | ServerMessage::SoloUnavailable => {
                            RETRY_ENABLED.with(|enabled| enabled.set(false));
                        }
                        _ => {}
                    }
                    let _ = message_sender.send(message);
                }
                Err(error) => warn!("Ignoring malformed server message: {error}"),
            }
        });
        let close_sender = sender.clone();
        let close_outgoing = outgoing.clone();
        let close_callback = Closure::<dyn FnMut(Event)>::new(move |_| {
            let _ = close_sender.send(ServerMessage::ConnectionLost);
            schedule_reconnect(close_sender.clone(), close_outgoing.clone());
        });
        socket.set_onclose(Some(close_callback.as_ref().unchecked_ref()));
        socket.set_onmessage(Some(callback.as_ref().unchecked_ref()));
        SOCKET.with(|current| *current.borrow_mut() = Some(socket));
        MESSAGE_CALLBACK.with(|current| *current.borrow_mut() = Some(callback));
        SOCKET_CALLBACKS.with(|callbacks| callbacks.borrow_mut().push(close_callback));
        info!("Connecting to {address}");
    }

    fn schedule_reconnect(
        sender: mpsc::Sender<ServerMessage>,
        outgoing: mpsc::Sender<ClientMessage>,
    ) {
        if !RETRY_ENABLED.with(Cell::get)
            || RETRY_SCHEDULED.with(|scheduled| scheduled.replace(true))
        {
            return;
        }
        let Some(window) = web_sys::window() else {
            RETRY_SCHEDULED.with(|scheduled| scheduled.set(false));
            return;
        };
        let callback = Closure::<dyn FnMut()>::new(move || {
            RETRY_SCHEDULED.with(|scheduled| scheduled.set(false));
            if !RETRY_ENABLED.with(Cell::get) {
                return;
            }
            let room_code = RECONNECT_ROOM.with(|room| room.borrow().clone());
            let Some(room_code) = room_code else {
                RETRY_ENABLED.with(|enabled| enabled.set(false));
                return;
            };
            let token = storage().and_then(|storage| {
                storage
                    .get_item(&token_storage_key(&room_code))
                    .ok()
                    .flatten()
            });
            let Some(token) = token else {
                RETRY_ENABLED.with(|enabled| enabled.set(false));
                return;
            };
            let Some(base) = server_base() else {
                RETRY_ENABLED.with(|enabled| enabled.set(false));
                return;
            };
            open_socket(
                append_query(&base, &format!("room={room_code}&reconnect={token}")),
                sender.clone(),
                outgoing.clone(),
            );
        });
        if window
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                callback.as_ref().unchecked_ref(),
                1_000,
            )
            .is_ok()
        {
            RETRY_CALLBACKS.with(|callbacks| callbacks.borrow_mut().push(callback));
        } else {
            RETRY_SCHEDULED.with(|scheduled| scheduled.set(false));
        }
    }

    fn document() -> Option<Document> {
        web_sys::window().and_then(|window| window.document())
    }

    fn install_lobby_handlers(outgoing: mpsc::Sender<ClientMessage>) {
        let rider_outgoing = outgoing.clone();
        attach_click("choose-rider", move || {
            let _ = rider_outgoing.send(ClientMessage::ChooseSeat {
                seat: SeatKind::Rider,
            });
        });
        let gunner_outgoing = outgoing.clone();
        attach_click("choose-gunner", move || {
            let _ = gunner_outgoing.send(ClientMessage::ChooseSeat {
                seat: SeatKind::Gunner,
            });
        });

        let swap_outgoing = outgoing.clone();
        attach_click("request-seat-swap", move || {
            let _ = swap_outgoing.send(ClientMessage::RequestSeatSwap);
        });
        let accept_outgoing = outgoing.clone();
        attach_click("accept-seat-swap", move || {
            PENDING_SWAP_ID.with(|pending| {
                if let Some(request_id) = pending.get() {
                    let _ = accept_outgoing.send(ClientMessage::RespondSeatSwap {
                        request_id,
                        accept: true,
                    });
                }
            });
        });
        let reject_outgoing = outgoing.clone();
        attach_click("reject-seat-swap", move || {
            PENDING_SWAP_ID.with(|pending| {
                if let Some(request_id) = pending.get() {
                    let _ = reject_outgoing.send(ClientMessage::RespondSeatSwap {
                        request_id,
                        accept: false,
                    });
                }
            });
        });

        let leave_outgoing = outgoing.clone();
        attach_click("ready-button", move || {
            if let Some(document) = document()
                && document
                    .get_element_by_id("solo-mode")
                    .is_some_and(|element| !is_hidden(&element))
            {
                let _ = outgoing.send(ClientMessage::ForceStartSolo);
            } else {
                READY_STATE.with(|ready| {
                    let next = !ready.get();
                    ready.set(next);
                    let _ = outgoing.send(ClientMessage::SetReady { ready: next });
                });
            }
        });

        attach_click("leave-room-button", move || {
            RETRY_ENABLED.with(|enabled| enabled.set(false));
            let _ = leave_outgoing.send(ClientMessage::LeaveRoom);
        });
    }

    fn attach_click(id: &str, mut callback: impl FnMut() + 'static) {
        let Some(element) = document().and_then(|document| document.get_element_by_id(id)) else {
            return;
        };
        let handler = Closure::<dyn FnMut(Event)>::new(move |_| callback());
        if element
            .add_event_listener_with_callback("click", handler.as_ref().unchecked_ref())
            .is_ok()
        {
            UI_CALLBACKS.with(|callbacks| callbacks.borrow_mut().push(handler));
        }
    }

    fn is_hidden(element: &web_sys::Element) -> bool {
        element.has_attribute("hidden")
    }

    fn set_visible(document: &Document, id: &str, visible: bool) {
        let Some(element) = document.get_element_by_id(id) else {
            return;
        };
        if visible {
            let _ = element.remove_attribute("hidden");
        } else {
            let _ = element.set_attribute("hidden", "");
        }
    }

    fn set_disabled(document: &Document, id: &str, disabled: bool) {
        let Some(element) = document.get_element_by_id(id) else {
            return;
        };
        if disabled {
            let _ = element.set_attribute("disabled", "");
        } else {
            let _ = element.remove_attribute("disabled");
        }
    }

    fn set_text(document: &Document, id: &str, text: &str) {
        if let Some(element) = document.get_element_by_id(id) {
            element.set_text_content(Some(text));
        }
    }

    fn role_label(
        network: &NetworkClient,
        seat: SeatKind,
        status: Option<&riverside_common::SeatStatus>,
    ) -> String {
        if network.seat == Some(seat) {
            "你已選擇此角色".to_owned()
        } else if status.is_some_and(|status| status.reserved) {
            "玩家短暫斷線，座位保留中".to_owned()
        } else if status.is_some_and(|status| status.connected) {
            "隊友已選擇".to_owned()
        } else {
            "空席，可選擇".to_owned()
        }
    }

    pub(super) fn render_lobby(network: &NetworkClient, session: &GameSession) {
        let Some(document) = document() else {
            return;
        };
        PENDING_SWAP_ID.with(|pending| {
            pending.set(
                network
                    .pending_seat_swap
                    .as_ref()
                    .map(|swap| swap.request_id),
            );
        });
        let show_entry = !network.connected
            && !network.connection_lost
            && (network.room_full
                || network.room_limit_reached
                || network.room_not_found
                || network.room_selection_required
                || network.invalid_room_code
                || network.solo_unavailable);
        set_visible(
            &document,
            "room-entry",
            !network.connected && !network.connection_lost,
        );
        set_visible(
            &document,
            "room-lobby",
            network.connected || network.connection_lost,
        );
        set_visible(&document, "leave-room-button", network.connected);
        set_visible(
            &document,
            "lobby-panel",
            !network.connected || session.phase != GamePhase::Running,
        );
        let status = if network.reconnect_unavailable {
            "重連時限已過或房間已結束；請使用新的邀請碼加入。".to_owned()
        } else if network.reconnect_in_use {
            "這個連線憑證仍在使用中，請確認沒有另一個分頁正在連線。".to_owned()
        } else if network.connection_lost {
            format!(
                "連線中斷，正在嘗試重連。對局已暫停，座位保留 {} 秒。",
                network.reconnect_seconds_remaining.unwrap_or(30)
            )
        } else if session.phase == GamePhase::Paused {
            format!(
                "對局已暫停，等待斷線玩家重連並重新準備（座位保留 {} 秒）。",
                network.reconnect_seconds_remaining.unwrap_or(30)
            )
        } else if show_entry && network.room_full {
            "房間已滿，請建立新房間或使用其他邀請碼。".to_owned()
        } else if network.room_limit_reached {
            "目前房間數已達上限，請稍後再建立。".to_owned()
        } else if network.room_not_found {
            "找不到這個房間，請確認邀請碼。".to_owned()
        } else if network.invalid_room_code {
            "房間代碼格式不正確。".to_owned()
        } else if network.solo_unavailable {
            "開發用單人模式尚未啟用。".to_owned()
        } else if network.room_selection_required {
            "請先建立房間或輸入邀請碼加入。".to_owned()
        } else if !network.connected {
            "建立或加入房間後，選擇角色即可準備。".to_owned()
        } else if session.phase == GamePhase::Countdown {
            format!("倒數開始：{} 秒", session.countdown_seconds.unwrap_or(5))
        } else if session.phase == GamePhase::Finished {
            if session.match_result == Some(riverside_common::MatchResult::Aborted) {
                "對局已中止；準備後可開始下一局。".to_owned()
            } else {
                "本局已結束；準備後可開始下一局。".to_owned()
            }
        } else if session.ready {
            "已準備，等待另一位玩家。".to_owned()
        } else if network.seat.is_none() && !network.solo_mode {
            "請選擇 Rider 或 Gunner，選角後才能準備。".to_owned()
        } else {
            "確認角色與操作說明後按下準備。".to_owned()
        };
        set_text(&document, "entry-status", &status);
        set_text(&document, "lobby-status", &status);

        let solo = network.solo_mode;
        set_visible(&document, "solo-mode", solo && network.connected);
        set_visible(&document, "seat-selection", !solo && network.connected);
        set_visible(&document, "invite-field", !solo && network.connected);
        if let Some(code) = &network.room_code {
            set_text(&document, "room-code", code);
            let invite = web_sys::window()
                .and_then(|window| {
                    let location = window.location();
                    Some(format!(
                        "{}{}?room={code}",
                        location.origin().ok()?,
                        location.pathname().ok()?
                    ))
                })
                .unwrap_or_else(|| format!("?room={code}"));
            if let Some(input) = document
                .get_element_by_id("invite-url")
                .and_then(|element| element.dyn_into::<HtmlInputElement>().ok())
            {
                input.set_value(&invite);
            }
        }
        set_text(
            &document,
            "unassigned-count",
            &format!("尚未選角玩家：{}", network.unassigned_players),
        );
        let rider = session.seats.get(&SeatKind::Rider);
        let gunner = session.seats.get(&SeatKind::Gunner);
        let both_seats_filled = rider.is_some_and(|status| status.connected)
            && gunner.is_some_and(|status| status.connected);
        set_text(
            &document,
            "rider-status",
            &role_label(network, SeatKind::Rider, rider),
        );
        set_text(
            &document,
            "gunner-status",
            &role_label(network, SeatKind::Gunner, gunner),
        );
        let may_choose = matches!(
            session.phase,
            GamePhase::WaitingForPlayers | GamePhase::ReadyCheck | GamePhase::Finished
        );
        set_disabled(
            &document,
            "choose-rider",
            solo || !may_choose
                || network.seat == Some(SeatKind::Rider)
                || rider.is_some_and(|status| {
                    status.connected && network.seat != Some(SeatKind::Rider)
                }),
        );
        set_disabled(
            &document,
            "choose-gunner",
            solo || !may_choose
                || network.seat == Some(SeatKind::Gunner)
                || gunner.is_some_and(|status| {
                    status.connected && network.seat != Some(SeatKind::Gunner)
                }),
        );
        let may_swap = !solo
            && both_seats_filled
            && matches!(
                session.phase,
                GamePhase::WaitingForPlayers | GamePhase::ReadyCheck | GamePhase::Finished
            );
        set_visible(&document, "seat-swap-controls", may_swap);
        set_visible(
            &document,
            "request-seat-swap",
            may_swap && network.pending_seat_swap.is_none(),
        );
        let is_swap_target = network
            .pending_seat_swap
            .as_ref()
            .is_some_and(|swap| network.player_id == Some(swap.target_id));
        set_visible(&document, "seat-swap-response", may_swap && is_swap_target);
        let swap_status = network.seat_swap_status.clone().unwrap_or_else(|| {
            network.pending_seat_swap.as_ref().map_or_else(
                || "對局開始前或結束後可提出角色交換。".to_owned(),
                |swap| {
                    if network.player_id == Some(swap.requester_id) {
                        format!("等待隊友回覆，{} 秒後逾時。", swap.expires_in_seconds)
                    } else {
                        format!("接受後你會改選 {}。", swap.from_seat.label())
                    }
                },
            )
        });
        set_text(&document, "seat-swap-status", &swap_status);
        let ready_label = if solo {
            "強制準備並開始"
        } else if session.phase == GamePhase::Paused {
            "準備並恢復對局"
        } else if session.phase == GamePhase::Countdown && session.ready {
            "取消準備並停止倒數"
        } else if session.phase == GamePhase::Finished {
            "準備開始下一局"
        } else if session.ready {
            "取消準備"
        } else {
            "準備開始"
        };
        set_text(&document, "ready-button", ready_label);
        set_disabled(
            &document,
            "ready-button",
            !network.connected
                || (!solo && (network.seat.is_none() || session.phase == GamePhase::Running)),
        );
        READY_STATE.with(|ready| ready.set(session.ready));
        if let Some(reason) = network.seat_choice_error {
            let text = match reason {
                SeatChoiceRejection::SeatOccupied => "這個角色已被隊友選擇。",
                SeatChoiceRejection::MatchLocked => "倒數或對局中不能更換角色。",
                SeatChoiceRejection::NotRoomMember => "你目前不在這個房間。",
                SeatChoiceRejection::SoloUsesRoleSwitch => "單人測試請按 V 切換角色。",
            };
            set_text(&document, "lobby-status", text);
        }
        if network.seat_swap_status.is_some() {
            set_text(&document, "lobby-status", &swap_status);
        }
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
