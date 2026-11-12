//! The main menu: choose single player, host a game, or join by IP.
//!
//! The menu is plain Bevy UI. It is the first thing a player sees, and it is
//! also where a refused connection ("Content mismatch") is reported.

use bevy::prelude::*;
use bevy::text::FontSize;

use deepcore_shared::protocol::DEFAULT_PORT;

use crate::net::{ConnectionState, SessionMode, parse_server_addr};

/// The application's top-level screen.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AppScreen {
    /// Choosing how to play.
    #[default]
    Menu,
    /// Connecting or loading the world.
    Loading,
    /// In the world.
    Playing,
}

/// What the menu is currently doing.
#[derive(Resource, Debug, Default)]
pub struct MenuState {
    /// The address typed into the Join field.
    pub address: String,
    /// The name typed into the name field.
    pub name: String,
    /// An error to display under the menu, e.g. a bad address.
    pub error: String,
    /// Whether the address field has focus, so typed characters go to it.
    pub address_focused: bool,
    /// Whether the name field has focus.
    pub name_focused: bool,
}

impl MenuState {
    /// Creates the default menu state with a sensible name and address.
    pub fn new() -> Self {
        Self {
            address: format!("127.0.0.1:{DEFAULT_PORT}"),
            name: "Miner".to_string(),
            error: String::new(),
            address_focused: false,
            name_focused: false,
        }
    }
}

/// Marker for the menu root, so it can be despawned when play begins.
#[derive(Component)]
pub struct MenuRoot;

/// Marker for the error label.
#[derive(Component)]
pub struct MenuError;

/// Marker for the address field text.
#[derive(Component)]
pub struct AddressLabel;

/// Marker for the name field text.
#[derive(Component)]
pub struct NameLabel;

/// Marker for a menu button, carrying its action.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    /// Play alone with a local world.
    SinglePlayer,
    /// Start a server and play in it.
    HostGame,
    /// Join the address in the field.
    JoinByIp,
    /// Focus the address field.
    FocusAddress,
    /// Focus the name field.
    FocusName,
    /// Quit.
    Quit,
}

/// A request from the menu to start a session.
#[derive(Message, Debug, Clone)]
pub struct StartSession {
    /// How to play.
    pub mode: SessionMode,
    /// Whether to also run a server in this process.
    pub host_server: bool,
}

/// The menu plugin.
pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<AppScreen>()
            .insert_resource(MenuState::new())
            .add_systems(OnEnter(AppScreen::Menu), spawn_menu)
            .add_systems(OnExit(AppScreen::Menu), despawn_menu)
            .add_systems(
                Update,
                (handle_menu_buttons, type_into_fields, show_error)
                    .run_if(in_state(AppScreen::Menu)),
            );
    }
}

/// Colours used by the menu.
const BG: Color = Color::srgb(0.05, 0.07, 0.10);
const PANEL: Color = Color::srgba(0.10, 0.13, 0.18, 0.96);
const ACCENT: Color = Color::srgb(0.55, 0.72, 0.95);
const DIM: Color = Color::srgb(0.62, 0.68, 0.78);

fn spawn_menu(mut commands: Commands) {
    commands
        .spawn((
            MenuRoot,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(BG),
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(32.0)),
                    row_gap: Val::Px(14.0),
                    width: Val::Px(460.0),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(PANEL),
                BorderColor::all(Color::srgba(0.35, 0.45, 0.6, 0.5)),
            ))
            .with_children(|panel| {
                panel.spawn((
                    Text::new("DEEPCORE"),
                    TextFont {
                        font_size: FontSize::Px(40.0),
                        ..default()
                    },
                    TextColor(ACCENT),
                ));
                panel.spawn((
                    Text::new("Dig deep. Build machines. Go nuclear."),
                    TextFont {
                        font_size: FontSize::Px(15.0),
                        ..default()
                    },
                    TextColor(DIM),
                ));

                spacer(panel, 10.0);

                // Name field.
                field_label(panel, "Player name");
                panel
                    .spawn((
                        MenuAction::FocusName,
                        Node {
                            padding: UiRect::all(Val::Px(8.0)),
                            border: UiRect::all(Val::Px(1.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.04, 0.05, 0.08, 1.0)),
                        BorderColor::all(Color::srgba(0.3, 0.38, 0.5, 0.7)),
                    ))
                    .with_child((
                        NameLabel,
                        Text::new("Miner"),
                        TextFont {
                            font_size: FontSize::Px(16.0),
                            ..default()
                        },
                        TextColor(Color::WHITE),
                    ));

                // Address field.
                field_label(panel, "Server address");
                panel
                    .spawn((
                        MenuAction::FocusAddress,
                        Node {
                            padding: UiRect::all(Val::Px(8.0)),
                            border: UiRect::all(Val::Px(1.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.04, 0.05, 0.08, 1.0)),
                        BorderColor::all(Color::srgba(0.3, 0.38, 0.5, 0.7)),
                    ))
                    .with_child((
                        AddressLabel,
                        Text::new(""),
                        TextFont {
                            font_size: FontSize::Px(16.0),
                            ..default()
                        },
                        TextColor(Color::WHITE),
                    ));

                spacer(panel, 6.0);

                button(panel, MenuAction::HostGame, "Host game", ACCENT);
                button(panel, MenuAction::JoinByIp, "Join by IP", ACCENT);
                button(panel, MenuAction::SinglePlayer, "Play alone", DIM);
                button(panel, MenuAction::Quit, "Quit", DIM);

                spacer(panel, 4.0);
                panel.spawn((
                    MenuError,
                    Text::new(""),
                    TextFont {
                        font_size: FontSize::Px(14.0),
                        ..default()
                    },
                    TextColor(Color::srgb(1.0, 0.45, 0.35)),
                ));
            });
        });
}

fn spacer(parent: &mut ChildSpawnerCommands, height: f32) {
    parent.spawn(Node {
        height: Val::Px(height),
        ..default()
    });
}

fn field_label(parent: &mut ChildSpawnerCommands, text: &str) {
    parent.spawn((
        Text::new(text.to_string()),
        TextFont {
            font_size: FontSize::Px(13.0),
            ..default()
        },
        TextColor(DIM),
    ));
}

fn button(parent: &mut ChildSpawnerCommands, action: MenuAction, label: &str, color: Color) {
    parent
        .spawn((
            action,
            Node {
                padding: UiRect::all(Val::Px(10.0)),
                justify_content: JustifyContent::Center,
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.14, 0.18, 0.25, 1.0)),
            BorderColor::all(color),
        ))
        .with_child((
            Text::new(label.to_string()),
            TextFont {
                font_size: FontSize::Px(17.0),
                ..default()
            },
            TextColor(color),
        ));
}

fn despawn_menu(mut commands: Commands, roots: Query<Entity, With<MenuRoot>>) {
    for entity in &roots {
        commands.entity(entity).despawn();
    }
}

/// Handles clicks and keyboard shortcuts on the menu.
fn handle_menu_buttons(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window>,
    actions: Query<(&MenuAction, &Interaction, &ComputedNode, &GlobalTransform)>,
    mut menu: ResMut<MenuState>,
    mut start: MessageWriter<StartSession>,
    mut exit: MessageWriter<AppExit>,
    mut next_screen: ResMut<NextState<AppScreen>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let cursor = window.cursor_position();
    let clicked = mouse.just_pressed(MouseButton::Left);

    if clicked && let Some(cursor) = cursor {
        for (action, _interaction, node, transform) in &actions {
            let size = node.size();
            let center = transform.translation().truncate();
            let half = size * 0.5;
            let inside =
                (cursor.x - center.x).abs() <= half.x && (cursor.y - center.y).abs() <= half.y;
            if !inside {
                continue;
            }
            match action {
                MenuAction::FocusAddress => {
                    menu.address_focused = true;
                    menu.name_focused = false;
                }
                MenuAction::FocusName => {
                    menu.name_focused = true;
                    menu.address_focused = false;
                }
                MenuAction::SinglePlayer => {
                    start.write(StartSession {
                        mode: SessionMode::SinglePlayer,
                        host_server: false,
                    });
                    next_screen.set(AppScreen::Loading);
                }
                MenuAction::HostGame => {
                    let addr =
                        parse_server_addr(&menu.address, DEFAULT_PORT).unwrap_or_else(|_| {
                            format!("127.0.0.1:{DEFAULT_PORT}").parse().unwrap_or(
                                std::net::SocketAddr::from(([127, 0, 0, 1], DEFAULT_PORT)),
                            )
                        });
                    start.write(StartSession {
                        mode: SessionMode::Client {
                            addr,
                            name: menu.name.clone(),
                        },
                        host_server: true,
                    });
                    next_screen.set(AppScreen::Loading);
                }
                MenuAction::JoinByIp => match parse_server_addr(&menu.address, DEFAULT_PORT) {
                    Ok(addr) => {
                        start.write(StartSession {
                            mode: SessionMode::Client {
                                addr,
                                name: menu.name.clone(),
                            },
                            host_server: false,
                        });
                        next_screen.set(AppScreen::Loading);
                    }
                    Err(err) => menu.error = err,
                },
                MenuAction::Quit => {
                    exit.write(AppExit::Success);
                }
            }
            return;
        }
    }

    // Keyboard: Enter joins the typed address, Escape clears focus.
    if keys.just_pressed(KeyCode::Enter) {
        match parse_server_addr(&menu.address, DEFAULT_PORT) {
            Ok(addr) => {
                start.write(StartSession {
                    mode: SessionMode::Client {
                        addr,
                        name: menu.name.clone(),
                    },
                    host_server: false,
                });
                next_screen.set(AppScreen::Loading);
            }
            Err(err) => menu.error = err,
        }
    }
    if keys.just_pressed(KeyCode::Escape) {
        menu.address_focused = false;
        menu.name_focused = false;
    }
}

/// Types into whichever field has focus.
fn type_into_fields(
    mut events: MessageReader<bevy::input::keyboard::KeyboardInput>,
    mut menu: ResMut<MenuState>,
    mut address: Query<&mut Text, (With<AddressLabel>, Without<NameLabel>)>,
    mut name: Query<&mut Text, (With<NameLabel>, Without<AddressLabel>)>,
) {
    for event in events.read() {
        if !event.state.is_pressed() {
            continue;
        }
        let typed = match event.key_code {
            KeyCode::Backspace => {
                if menu.address_focused {
                    menu.address.pop();
                } else if menu.name_focused {
                    menu.name.pop();
                }
                continue;
            }
            KeyCode::Space => Some(' '),
            KeyCode::Period => Some('.'),
            KeyCode::Minus => Some('-'),
            KeyCode::Slash => Some('/'),
            _ => event.text.as_ref().and_then(|t| t.chars().next()),
        };
        let Some(ch) = typed else {
            continue;
        };
        if menu.address_focused {
            if menu.address.chars().count() < 64 {
                menu.address.push(ch);
            }
        } else if menu.name_focused && menu.name.chars().count() < 32 {
            menu.name.push(ch);
        }
    }

    if let Ok(mut text) = address.single_mut() {
        **text = menu.address.clone();
    }
    if let Ok(mut text) = name.single_mut() {
        **text = menu.name.clone();
    }
}

/// Shows an error, or the connection failure reason, under the menu.
fn show_error(
    menu: Res<MenuState>,
    connection: Res<ConnectionState>,
    mut label: Query<&mut Text, With<MenuError>>,
    mut next_screen: ResMut<NextState<AppScreen>>,
) {
    let Ok(mut text) = label.single_mut() else {
        return;
    };
    if let ConnectionState::Failed(reason) = &*connection {
        // A refused connection (e.g. "Content mismatch") sends the player back
        // to the menu with the reason visible.
        **text = reason.clone();
        next_screen.set(AppScreen::Menu);
    } else {
        **text = menu.error.clone();
    }
}
