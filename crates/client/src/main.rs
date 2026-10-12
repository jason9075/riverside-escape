mod animation;
mod game;
mod network;
mod scenery;

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
        .init_resource::<animation::MotionClock>()
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
        .add_systems(
            Startup,
            (
                game::setup_scene,
                animation::spawn_weapons,
                network::connect_browser,
            )
                .chain(),
        )
        .add_systems(
            Update,
            (
                network::pump_network,
                game::rebuild_course_visuals,
                game::knock_cones,
                game::capture_input,
                game::simulate_bike,
                game::apply_world_transforms,
                game::apply_steering_visual,
                game::update_combat_visuals,
                game::update_camera,
                animation::bind_parts,
                animation::animate_parts,
                animation::update_weapons,
                game::update_hud,
                network::flush_outgoing,
                network::refresh_diagnostics,
            )
                .chain(),
        )
        .run();
}
