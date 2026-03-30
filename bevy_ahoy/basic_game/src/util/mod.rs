//! Common functionality for the examples. This is just aesthetic stuff, you don't need to copy any of this into your own projects.

use std::{collections::VecDeque, time::Duration};

use avian3d::prelude::*;
use bevy::{
    light::DirectionalLightShadowMap,
    platform::collections::HashSet,
    prelude::*,
    window::{CursorGrabMode, CursorOptions},
};
use bevy_ahoy::{CharacterControllerOutput, CharacterControllerState, prelude::*};
use bevy_enhanced_input::prelude::{Release, *};
use bevy_fix_cursor_unlock_web::{FixPointerUnlockPlugin, ForceUnlockCursor};
use bevy_framepace::FramepacePlugin;
use bevy_mod_mipmap_generator::{MipmapGeneratorPlugin, generate_mipmaps};
use bevy_time::common_conditions::on_timer;

pub(super) struct ExampleUtilPlugin;

impl Plugin for ExampleUtilPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            MipmapGeneratorPlugin,
            FixPointerUnlockPlugin,
            FramepacePlugin,
        ))
        .add_systems(Startup, (setup_ui, spawn_crosshair))
        .add_systems(
            Update,
            (
                update_debug_text,
                tweak_materials,
                generate_mipmaps::<StandardMaterial>,
                calculate_stable_ground.run_if(on_timer(Duration::from_secs(1))),
                apply_last_stable_ground.after(calculate_stable_ground),
            ),
        )
        .add_observer(toggle_debug)
        .add_observer(unlock_cursor_web)
        .insert_resource(DirectionalLightShadowMap { size: 4096 })
        .insert_resource(GlobalAmbientLight::NONE)
        .add_input_context::<DebugInput>();
    }
}

fn update_debug_text(
    mut text: Single<&mut Text, With<DebugText>>,
    kcc: Single<
        (
            &CharacterControllerState,
            &CharacterControllerOutput,
            &LinearVelocity,
            &CollidingEntities,
            &ColliderAabb,
            &StableGround,
        ),
        (With<CharacterController>, With<CharacterControllerCamera>),
    >,
    camera: Single<&Transform, With<Camera>>,
    names: Query<NameOrEntity>,
) {
    let (state, output, velocity, colliding_entities, aabb, stable_ground) = kcc.into_inner();
    let velocity = **velocity;
    let speed = velocity.length();
    let horizontal_speed = velocity.xz().length();
    let camera_position = camera.translation;
    let collisions = names
        .iter_many(
            output
                .touching_entities
                .iter()
                .map(|e| e.entity)
                .collect::<HashSet<_>>(),
        )
        .map(|name| {
            name.name
                .map(|n| format!("{} ({})", name.entity, n))
                .unwrap_or_else(|| format!("{}", name.entity))
        })
        .collect::<Vec<_>>();
    let real_collisions = names
        .iter_many(colliding_entities.iter())
        .map(|name| {
            name.name
                .map(|n| format!("{} ({})", name.entity, n))
                .unwrap_or_else(|| format!("{}", name.entity))
        })
        .collect::<Vec<_>>();
    let ground = state
        .grounded
        .and_then(|ground| names.get(ground.entity).ok())
        .map(|name| {
            name.name
                .map(|n| format!("{} ({})", name.entity, n))
                .unwrap_or(format!("{}", name.entity))
        });
    let stable_ground = stable_ground.previous.back();
    text.0 = format!(
        "Speed: {speed:.3}\nHorizontal Speed: {horizontal_speed:.3}\nVelocity: [{:.3}, {:.3}, {:.3}]\nCamera Position: [{:.3}, {:.3}, {:.3}]\nCollider Aabb:\n  min:[{:.3}, {:.3}, {:.3}]\n  max:[{:.3}, {:.3}, {:.3}]\nReal Collisions: {:#?}\nCollisions: {:#?}\nGround: {:?}\nLast Stable Ground: {:?}",
        velocity.x,
        velocity.y,
        velocity.z,
        camera_position.x,
        camera_position.y,
        camera_position.z,
        aabb.min.x,
        aabb.min.y,
        aabb.min.z,
        aabb.max.x,
        aabb.max.y,
        aabb.max.z,
        real_collisions,
        collisions,
        ground,
        stable_ground
    );
}

#[derive(Component, Reflect, Debug)]
#[reflect(Component)]
struct DebugText;

#[derive(Component)]
pub(super) struct ControlsOverlay;

fn setup_ui(mut commands: Commands) {
    commands.spawn((
        Node::default(),
        Text::default(),
        Visibility::Hidden,
        DebugText,
    ));
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            right: px(16.0),
            bottom: px(16.0),
            justify_self: JustifySelf::End,
            justify_content: JustifyContent::End,
            align_self: AlignSelf::End,
            padding: UiRect::axes(px(14.0), px(12.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.01, 0.03, 0.08, 0.34)),
        ControlsOverlay,
        Text::new(
            "Controls:\nWASD: move\nSpace: jump\nCtrl: crouch\nEsc: free mouse\nR: reset position\nBacktick: Toggle Debug Menu",
        ),
        TextFont {
            font_size: 18.0,
            ..default()
        },
        TextColor(Color::srgb(0.9, 0.96, 1.0)),
    ));
    commands.spawn((
        DebugInput,
        actions!(DebugInput[
            (
                Action::<ToggleDebug>::new(),
                bindings![KeyCode::Backquote, GamepadButton::Start],
                Release::default(),
            ),
        ]),
    ));
}

#[derive(Component, Default)]
struct DebugInput;

#[derive(Debug, InputAction)]
#[action_output(bool)]
pub(super) struct ToggleDebug;

fn toggle_debug(
    _fire: On<Fire<ToggleDebug>>,
    mut visibility: Single<&mut Visibility, With<DebugText>>,
) {
    **visibility = match **visibility {
        Visibility::Hidden => Visibility::Inherited,
        _ => Visibility::Hidden,
    };
}

fn unlock_cursor_web(
    _unlock: On<ForceUnlockCursor>,
    mut cursor_options: Single<&mut CursorOptions>,
) {
    cursor_options.grab_mode = CursorGrabMode::None;
    cursor_options.visible = true;
}

/// Show a crosshair for better aiming
fn spawn_crosshair(mut commands: Commands, asset_server: Res<AssetServer>) {
    let crosshair_texture = asset_server.load("sprites/crosshair.png");
    commands
        .spawn(Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|parent| {
            parent
                .spawn(ImageNode::new(crosshair_texture).with_color(Color::WHITE.with_alpha(0.3)));
        });
}

fn tweak_materials(
    mut asset_events: MessageReader<AssetEvent<StandardMaterial>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    assets: Res<AssetServer>,
) {
    for event in asset_events.read() {
        let AssetEvent::LoadedWithDependencies { id } = event else {
            continue;
        };
        let Some(mat) = mats.get_mut(*id) else {
            continue;
        };
        if mat
            .base_color_texture
            .as_ref()
            .and_then(|t| {
                assets
                    .get_path(t.id())?
                    .path()
                    .file_name()?
                    .to_string_lossy()
                    .to_lowercase()
                    .into()
            })
            .is_some_and(|name| name.contains("water_01"))
        {
            mat.base_color = Color::srgb(0.92, 0.94, 0.98);
            mat.perceptual_roughness = 0.2;
            mat.alpha_mode = AlphaMode::Opaque;
        } else {
            mat.perceptual_roughness = 0.8;
        }
    }
}

#[derive(Component, Reflect)]
pub struct StableGround {
    previous: VecDeque<Vec3>,
    fall_timer: Timer,
}
impl Default for StableGround {
    fn default() -> Self {
        Self {
            previous: VecDeque::default(),
            fall_timer: Timer::new(Duration::from_secs(5), TimerMode::Once),
        }
    }
}

pub(crate) fn calculate_stable_ground(
    mut kccs: Query<(&Transform, &CharacterControllerState, &mut StableGround)>,
) {
    for (transform, state, mut stable_ground) in &mut kccs {
        let Some(ground) = state.grounded else {
            continue;
        };

        let up_diff = (1. - ground.normal1.y).abs();

        // If we don't compare to EPSILON, Vec3::y will *almost* always be 0.9...
        if up_diff <= f32::EPSILON {
            stable_ground.previous.push_front(transform.translation);

            // Used to ensure that player doesn't get stuck in infinite loop if the most recent
            // stable ground wasn't so stable.
            while stable_ground.previous.len() > 5 {
                stable_ground.previous.pop_back();
            }
        }
    }
}

pub(crate) fn apply_last_stable_ground(
    mut kccs: Query<(
        &mut Transform,
        &LinearVelocity,
        &CharacterController,
        &mut StableGround,
    )>,
    time: Res<Time>,
) {
    for (mut transform, velocity, controller, mut stable_ground) in &mut kccs {
        let speed_diff = 1. - (velocity.0.y.abs() / controller.max_speed);

        // Terminal velocity will take quite a while to reach exactly 100., so we compare to 0.01
        // to ensure that it doesn't take longer than expected
        if speed_diff <= 0.01 {
            stable_ground.fall_timer.tick(time.elapsed());
        } else {
            stable_ground.fall_timer.reset();
        }

        let max_fall_elapsed = stable_ground.fall_timer.is_finished();

        if max_fall_elapsed && let Some(last_stable_ground) = stable_ground.previous.pop_front() {
            transform.translation = last_stable_ground;
        }
    }
}
