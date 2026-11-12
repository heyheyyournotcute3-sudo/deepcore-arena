//! Heads-up display: crosshair, hotbar, depth meter and mining feedback.
//!
//! Built with Bevy UI rather than egui because it is always on screen and needs
//! to stay cheap to lay out.

use bevy::prelude::*;
use bevy::text::FontSize;

use deepcore_shared::content::ContentRegistry;
use deepcore_shared::worldgen::WorldGen;

use crate::player::{LocalPlayer, MiningFeedback, PlayerRuntime, TargetedBlock};

/// Marker for the crosshair root.
#[derive(Component)]
struct Crosshair;

/// Marker for the depth meter text.
#[derive(Component)]
struct DepthText;

/// Marker for the mining feedback text.
#[derive(Component)]
struct FeedbackText;

/// Marker for a hotbar slot's label, with its slot index.
#[derive(Component)]
struct HotbarSlot(usize);

/// The HUD plugin.
pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_hud).add_systems(
            Update,
            (update_depth, update_feedback, update_hotbar, tint_crosshair),
        );
    }
}

fn setup_hud(mut commands: Commands, registry: Res<ContentRegistry>) {
    // Crosshair: a small plus made of two bars.
    commands
        .spawn((
            Crosshair,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.0),
                top: Val::Percent(50.0),
                width: Val::Px(2.0),
                height: Val::Px(18.0),
                margin: UiRect::new(Val::Px(-1.0), Val::Auto, Val::Px(-9.0), Val::Auto),
                ..default()
            },
            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
        ))
        .with_children(|parent| {
            parent.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(-8.0),
                    top: Val::Px(8.0),
                    width: Val::Px(18.0),
                    height: Val::Px(2.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
            ));
        });

    // Depth meter, top-left.
    commands.spawn((
        DepthText,
        Text::new("Depth: 0 m"),
        TextFont {
            font_size: FontSize::Px(20.0),
            ..default()
        },
        TextColor(Color::srgb(0.9, 0.95, 1.0)),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(16.0),
            top: Val::Px(12.0),
            ..default()
        },
    ));

    // Mining feedback, just under the crosshair.
    commands.spawn((
        FeedbackText,
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(22.0),
            ..default()
        },
        TextColor(Color::srgb(1.0, 0.45, 0.35)),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(50.0),
            top: Val::Percent(56.0),
            margin: UiRect::new(Val::Px(-90.0), Val::Auto, Val::Auto, Val::Auto),
            ..default()
        },
    ));

    // Hotbar: nine slots along the bottom.
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(50.0),
            bottom: Val::Px(20.0),
            margin: UiRect::new(Val::Px(-243.0), Val::Auto, Val::Auto, Val::Auto),
            column_gap: Val::Px(6.0),
            ..default()
        })
        .with_children(|parent| {
            for slot in 0..9 {
                parent
                    .spawn((
                        Node {
                            width: Val::Px(48.0),
                            height: Val::Px(48.0),
                            border: UiRect::all(Val::Px(2.0)),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BorderColor::all(Color::srgba(0.6, 0.65, 0.75, 0.55)),
                        BackgroundColor(Color::srgba(0.05, 0.06, 0.09, 0.62)),
                    ))
                    .with_children(|slot_entity| {
                        slot_entity.spawn((
                            HotbarSlot(slot),
                            Text::new(""),
                            TextFont {
                                font_size: FontSize::Px(12.0),
                                ..default()
                            },
                            TextColor(Color::srgb(0.92, 0.94, 1.0)),
                        ));
                    });
            }
        });

    // Show what the starting tool is, so the first frame is not empty.
    let _ = registry;
}

/// Updates the depth readout from the player's height.
fn update_depth(
    player: Query<&PlayerRuntime, With<LocalPlayer>>,
    mut text: Query<&mut Text, With<DepthText>>,
) {
    let Ok(runtime) = player.single() else {
        return;
    };
    let Ok(mut text) = text.single_mut() else {
        return;
    };
    let depth = WorldGen::depth_at(runtime.state.position.y.floor() as i32);
    let layer = if depth < 16 {
        "Topsoil"
    } else if depth < 64 {
        "Shale Beds"
    } else if depth < 160 {
        "Granite Depths"
    } else if depth < 288 {
        "Basalt Furnace"
    } else if depth < 448 {
        "Deep Crust"
    } else {
        "Mantle Edge"
    };
    **text = format!("Depth: {depth} m  ·  {layer}");
}

/// Shows or hides the mining refusal message.
fn update_feedback(feedback: Res<MiningFeedback>, mut text: Query<&mut Text, With<FeedbackText>>) {
    let Ok(mut text) = text.single_mut() else {
        return;
    };
    **text = feedback.message.clone();
}

/// Fills in the hotbar labels from the player's inventory.
fn update_hotbar(
    player: Query<&PlayerRuntime, With<LocalPlayer>>,
    registry: Res<ContentRegistry>,
    mut slots: Query<(&HotbarSlot, &mut Text, &ChildOf)>,
    mut containers: Query<&mut BackgroundColor>,
) {
    let Ok(runtime) = player.single() else {
        return;
    };
    for (slot, mut text, child_of) in &mut slots {
        let content = runtime.hotbar[slot.0];
        **text = match content {
            Some((item, count)) => {
                let name = &registry.0.item_info(item).display;
                if count > 1 {
                    format!("{}\n×{count}", short_name(name))
                } else {
                    short_name(name).to_string()
                }
            }
            None => String::new(),
        };

        // Highlight the selected slot's background.
        if let Ok(mut bg) = containers.get_mut(child_of.parent()) {
            *bg = if slot.0 == runtime.selected_slot {
                BackgroundColor(Color::srgba(0.30, 0.42, 0.62, 0.75))
            } else {
                BackgroundColor(Color::srgba(0.05, 0.06, 0.09, 0.62))
            };
        }
    }
}

/// Shortens long item names so they fit in a hotbar slot.
fn short_name(name: &str) -> &str {
    match name {
        "Steel Pickaxe" => "Pickaxe",
        "Hardened Steel Pickaxe" => "H.Pick",
        "Tungsten-Carbide Drill" => "T.Drill",
        "Nuclear Drill" => "N.Drill",
        other => other,
    }
}

/// Tints the crosshair red when the targeted block cannot be mined.
fn tint_crosshair(
    target: Res<TargetedBlock>,
    registry: Res<ContentRegistry>,
    player: Query<&PlayerRuntime, With<LocalPlayer>>,
    mut crosshair: Query<&mut BackgroundColor, With<Crosshair>>,
    chunks: Res<crate::render::chunks::ClientChunks>,
) {
    let Ok(runtime) = player.single() else {
        return;
    };
    let tool = runtime
        .held
        .and_then(|item| deepcore_shared::mining::MiningTool::from_item(registry.0.item_info(item)))
        .unwrap_or(deepcore_shared::mining::MiningTool::HANDS);

    let blocked = match target.0 {
        Some(hit) => match chunks.block_at(hit.block) {
            Some(block) if !block.is_air() => {
                let info = registry.0.block_info(block);
                tool.tier < info.min_tier || info.min_tier >= 200
            }
            _ => false,
        },
        None => false,
    };

    let color = if blocked {
        Color::srgba(1.0, 0.30, 0.28, 0.95)
    } else {
        Color::srgba(1.0, 1.0, 1.0, 0.85)
    };
    for mut bg in &mut crosshair {
        *bg = BackgroundColor(color);
    }
}
