mod game;
mod network;

use bevy::{
    asset::{AssetMetaCheck, AssetPlugin},
    prelude::*,
    window::CursorOptions,
};
use game::GameSession;
use network::NetworkClient;

fn main() {
    App::new()
        .insert_resource(GameSession::default())
        .insert_resource(NetworkClient::default())
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    meta_check: AssetMetaCheck::Never,
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Riverside Escape | Co-op prototype".to_owned(),
                        resolution: (1440, 900).into(),
                        canvas: Some("#game-canvas".to_owned()),
                        fit_canvas_to_parent: true,
                        prevent_default_event_handling: false,
                        ..default()
                    }),
                    primary_cursor_options: Some(CursorOptions::default()),
                    ..default()
                }),
        )
        .add_systems(Startup, (game::setup_scene, network::connect_browser))
        .add_systems(
            Update,
            (
                network::pump_network,
                game::capture_input,
                game::simulate_bike,
                game::apply_world_transforms,
                game::apply_steering_visual,
                game::update_combat_visuals,
                game::update_camera,
                game::update_hud,
                network::flush_outgoing,
                network::refresh_diagnostics,
            )
                .chain(),
        )
        .run();
}
