use std::sync::{Mutex, mpsc};

use bevy::prelude::*;
use riverside_common::{ClientMessage, GamePhase, SeatChoiceRejection, SeatKind, ServerMessage};

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
    pub solo_mode: bool,
    pub player_id: Option<u64>,
    pub room_code: Option<String>,
    pub seat: Option<SeatKind>,
    pub seat_choice_error: Option<SeatChoiceRejection>,
    pub unassigned_players: u8,
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
            solo_mode: false,
            player_id: None,
            room_code: None,
            seat: None,
            seat_choice_error: None,
            unassigned_players: 0,
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
            } => {
                network.connected = true;
                network.player_id = Some(player_id);
                network.room_code = Some(room_code);
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
                session.change_seat(seat);
                session.rider_input = Default::default();
                session.gunner_input.aim_held = false;
            }
            ServerMessage::SeatChoiceRejected { seat: _, reason } => {
                network.seat_choice_error = Some(reason);
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
    use std::{cell::{Cell, RefCell}, sync::mpsc};

    use bevy::prelude::*;
    use crate::GameSession;
    use riverside_common::{ClientMessage, GamePhase, SeatChoiceRejection, SeatKind, ServerMessage};
    use wasm_bindgen::{JsCast, closure::Closure};
    use web_sys::{Document, Event, HtmlInputElement, MessageEvent, WebSocket};

    use super::NetworkClient;

    thread_local! {
        static SOCKET: RefCell<Option<WebSocket>> = const { RefCell::new(None) };
        static MESSAGE_CALLBACK: RefCell<Option<Closure<dyn FnMut(MessageEvent)>>> = const { RefCell::new(None) };
        static UI_CALLBACKS: RefCell<Vec<Closure<dyn FnMut(Event)>>> = const { RefCell::new(Vec::new()) };
        static READY_STATE: Cell<bool> = const { Cell::new(false) };
    }

    pub(super) fn connect(
        sender: mpsc::Sender<ServerMessage>,
        outgoing: mpsc::Sender<ClientMessage>,
    ) {
        install_lobby_handlers(outgoing);
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
        let room_code = params.as_ref().and_then(|params| params.get("room"));
        if !requested_solo && !create_room && room_code.is_none() {
            return;
        }
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
        let action = if requested_solo {
            "solo=1".to_owned()
        } else if create_room {
            "create=1".to_owned()
        } else {
            format!("room={}", room_code.unwrap_or_default())
        };
        address.push(if address.contains('?') { '&' } else { '?' });
        address.push_str(&action);

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

    fn document() -> Option<Document> {
        web_sys::window().and_then(|window| window.document())
    }

    fn install_lobby_handlers(outgoing: mpsc::Sender<ClientMessage>) {
        let rider_outgoing = outgoing.clone();
        attach_click("choose-rider", move || {
            let _ = rider_outgoing.send(ClientMessage::ChooseSeat { seat: SeatKind::Rider });
        });
        let gunner_outgoing = outgoing.clone();
        attach_click("choose-gunner", move || {
            let _ = gunner_outgoing.send(ClientMessage::ChooseSeat { seat: SeatKind::Gunner });
        });

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
        let show_entry = !network.connected
            && (network.room_full
                || network.room_limit_reached
                || network.room_not_found
                || network.room_selection_required
                || network.invalid_room_code
                || network.solo_unavailable);
        set_visible(&document, "room-entry", !network.connected);
        set_visible(&document, "room-lobby", network.connected);
        set_visible(
            &document,
            "lobby-panel",
            !network.connected || session.phase != GamePhase::Running,
        );
        let status = if show_entry && network.room_full {
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
            "本局已結束；準備後可開始下一局。".to_owned()
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
        set_text(&document, "rider-status", &role_label(network, SeatKind::Rider, rider));
        set_text(&document, "gunner-status", &role_label(network, SeatKind::Gunner, gunner));
        let may_choose = matches!(
            session.phase,
            GamePhase::WaitingForPlayers | GamePhase::ReadyCheck | GamePhase::Finished
        );
        set_disabled(
            &document,
            "choose-rider",
            solo || !may_choose || network.seat == Some(SeatKind::Rider)
                || rider.is_some_and(|status| status.connected && network.seat != Some(SeatKind::Rider)),
        );
        set_disabled(
            &document,
            "choose-gunner",
            solo || !may_choose || network.seat == Some(SeatKind::Gunner)
                || gunner.is_some_and(|status| status.connected && network.seat != Some(SeatKind::Gunner)),
        );
        let ready_label = if solo {
            "強制準備並開始"
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
            !solo && (network.seat.is_none() || session.phase == GamePhase::Running),
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
