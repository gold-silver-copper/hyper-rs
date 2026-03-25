use avian3d::prelude::*;
use bevy::{
    color::palettes::tailwind,
    input::common_conditions::input_just_pressed,
    math::primitives::Cuboid,
    prelude::*,
    window::{CursorGrabMode, CursorOptions, WindowResolution},
};
use bevy_ahoy::{
    CharacterControllerOutput, CharacterLook, PickupConfig, PickupHoldConfig, PickupPullConfig,
    prelude::*,
};
use bevy_ecs::{lifecycle::HookContext, world::DeferredWorld};
use bevy_enhanced_input::prelude::{Hold, Press, *};

use crate::util::{ExampleUtilPlugin, StableGround};

mod util;

const PLAYER_SPAWN: Vec3 = Vec3::new(0.0, 2.5, 18.0);
const WALL_RUN_SPEED: f32 = 11.5;
const WALL_RUN_STICK_SPEED: f32 = 2.0;
const WALL_RUN_FALL_SPEED: f32 = 2.25;
const WALL_RUN_MIN_SPEED: f32 = 4.0;
const WALL_RUN_DURATION: f32 = 0.95;
const WALL_RUN_COOLDOWN: f32 = 0.2;

fn main() -> AppExit {
    App::new()
        .register_type::<SpawnPlayer>()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Window {
                    title: "Bevy Ahoy Sandbox".into(),
                    resolution: WindowResolution::new(1600, 900),
                    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "macos")))]
                    present_mode: bevy::window::PresentMode::Mailbox,
                    ..default()
                }
                .into(),
                ..default()
            }),
            PhysicsPlugins::default(),
            EnhancedInputPlugin,
            AhoyPlugins::default(),
            ExampleUtilPlugin,
        ))
        .add_input_context::<PlayerInput>()
        .insert_resource(ClearColor(tailwind::SKY_200.into()))
        .add_systems(Startup, (setup_scene, setup_help_text))
        .add_systems(PostStartup, tune_player_camera)
        .add_systems(
            Update,
            (
                capture_cursor.run_if(input_just_pressed(MouseButton::Left)),
                release_cursor.run_if(input_just_pressed(KeyCode::Escape)),
            ),
        )
        .add_systems(FixedUpdate, move_movers)
        .add_systems(
            FixedPostUpdate,
            apply_wall_run.after(AhoySystems::MoveCharacters),
        )
        .run()
}

fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Name::new("Spawn Point"),
        SpawnPlayer,
        Transform::from_translation(PLAYER_SPAWN),
        GlobalTransform::default(),
    ));

    let player = commands
        .spawn((
            Name::new("Player"),
            Player,
            PlayerInput,
            WallRunState::default(),
            CharacterController {
                speed: 9.0,
                air_speed: 2.0,
                jump_height: 1.9,
                max_speed: 24.0,
                ..default()
            },
            RigidBody::Kinematic,
            Collider::cylinder(0.7, 1.8),
            CollisionLayers::new(CollisionLayer::Player, LayerMask::ALL),
            Mass(45.0),
            StableGround::default(),
            Transform::from_translation(PLAYER_SPAWN),
        ))
        .id();

    commands.spawn((
        Name::new("Player Camera"),
        Camera3d::default(),
        CharacterControllerCameraOf::new(player),
        PickupConfig {
            prop_filter: SpatialQueryFilter::from_mask(CollisionLayer::Prop),
            actor_filter: SpatialQueryFilter::from_mask(CollisionLayer::Player),
            obstacle_filter: SpatialQueryFilter::from_mask(CollisionLayer::Default),
            hold: PickupHoldConfig {
                preferred_distance: 1.25,
                linear_velocity_easing: 0.8,
                ..default()
            },
            pull: PickupPullConfig {
                max_prop_mass: 250.0,
                ..default()
            },
            ..default()
        },
    ));

    commands.spawn((
        Name::new("Sun"),
        Transform::from_xyz(-25.0, 35.0, 15.0).looking_at(Vec3::ZERO, Vec3::Y),
        DirectionalLight {
            shadows_enabled: true,
            illuminance: 25_000.0,
            ..default()
        },
    ));

    spawn_static_box(
        &mut commands,
        &mut meshes,
        &mut materials,
        "Ground",
        Vec3::new(80.0, 1.0, 80.0),
        Vec3::new(0.0, -0.5, 0.0),
        tailwind::STONE_300.into(),
    );

    spawn_static_box(
        &mut commands,
        &mut meshes,
        &mut materials,
        "Ice Strip",
        Vec3::new(12.0, 0.5, 8.0),
        Vec3::new(-18.0, 0.25, 14.0),
        tailwind::SKY_300.into(),
    )
    .insert(Friction::new(0.02));

    spawn_static_box(
        &mut commands,
        &mut meshes,
        &mut materials,
        "Grip Pad",
        Vec3::new(10.0, 0.5, 8.0),
        Vec3::new(-18.0, 0.25, 4.0),
        tailwind::AMBER_300.into(),
    )
    .insert(Friction::new(3.0));

    for (index, height) in [1.0, 2.0, 3.0, 4.0, 5.5].into_iter().enumerate() {
        let size = Vec3::new(4.0, height, 4.0);
        let x = -4.5 + index as f32 * 4.5;
        spawn_static_box(
            &mut commands,
            &mut meshes,
            &mut materials,
            "Climb Box",
            size,
            Vec3::new(x, height * 0.5, 4.0 - index as f32 * 4.5),
            tailwind::ORANGE_300.into(),
        );
    }

    spawn_static_box(
        &mut commands,
        &mut meshes,
        &mut materials,
        "Mantle Tower",
        Vec3::new(7.0, 3.5, 7.0),
        Vec3::new(12.0, 1.75, 4.0),
        tailwind::ROSE_300.into(),
    );
    spawn_static_box(
        &mut commands,
        &mut meshes,
        &mut materials,
        "High Ledge",
        Vec3::new(4.0, 6.0, 4.0),
        Vec3::new(12.0, 5.25, -5.0),
        tailwind::ROSE_400.into(),
    );
    spawn_static_box(
        &mut commands,
        &mut meshes,
        &mut materials,
        "Climbdown Ledge",
        Vec3::new(4.0, 2.0, 4.0),
        Vec3::new(16.0, 1.0, -10.0),
        tailwind::ORANGE_200.into(),
    );

    spawn_static_box(
        &mut commands,
        &mut meshes,
        &mut materials,
        "Wall Run Start",
        Vec3::new(6.0, 2.0, 6.0),
        Vec3::new(-12.0, 1.0, -10.0),
        tailwind::EMERALD_300.into(),
    );
    spawn_static_box(
        &mut commands,
        &mut meshes,
        &mut materials,
        "Wall Run Finish",
        Vec3::new(6.0, 2.0, 6.0),
        Vec3::new(-30.0, 1.0, -10.0),
        tailwind::EMERALD_400.into(),
    );
    spawn_static_box(
        &mut commands,
        &mut meshes,
        &mut materials,
        "Wall Run Wall",
        Vec3::new(1.2, 8.0, 18.0),
        Vec3::new(-20.0, 4.0, -10.0),
        tailwind::SLATE_500.into(),
    );

    spawn_static_box(
        &mut commands,
        &mut meshes,
        &mut materials,
        "Water Basin Floor",
        Vec3::new(12.0, 0.5, 12.0),
        Vec3::new(20.0, -0.25, -8.0),
        tailwind::STONE_400.into(),
    );
    spawn_water_box(
        &mut commands,
        &mut meshes,
        &mut materials,
        Vec3::new(12.0, 4.0, 12.0),
        Vec3::new(20.0, 2.0, -8.0),
    );

    spawn_mover(
        &mut commands,
        &mut meshes,
        &mut materials,
        "Moving Platform",
        Vec3::new(4.0, 0.8, 4.0),
        Vec3::new(20.0, 1.8, -14.0),
        Vec3::new(20.0, 1.8, 2.0),
        5.0,
        tailwind::VIOLET_300.into(),
    );
    spawn_mover(
        &mut commands,
        &mut meshes,
        &mut materials,
        "Pusher Wall",
        Vec3::new(1.0, 4.0, 8.0),
        Vec3::new(6.0, 2.0, 18.0),
        Vec3::new(-6.0, 2.0, 18.0),
        3.0,
        tailwind::RED_300.into(),
    );

    for (index, offset) in [
        Vec3::new(6.0, 0.75, 12.0),
        Vec3::new(9.0, 0.75, 12.5),
        Vec3::new(12.0, 0.75, 13.0),
        Vec3::new(8.0, 2.25, 10.0),
        Vec3::new(10.5, 2.25, 9.5),
    ]
    .into_iter()
    .enumerate()
    {
        spawn_dynamic_box(
            &mut commands,
            &mut meshes,
            &mut materials,
            &format!("Prop Box {}", index + 1),
            Vec3::splat(1.5),
            offset,
            tailwind::AMBER_400.into(),
        );
    }
}

fn setup_help_text(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: px(12.0),
            left: px(12.0),
            max_width: px(540.0),
            padding: UiRect::all(px(10.0)),
            ..default()
        },
        Text::new(
            "Sandbox\n\
             Hold Space: auto-bhop, tic-tac, crane\n\
             Hold Space at a ledge: mantle\n\
             Ctrl: crouch and climb down\n\
             Swim with Space in the blue pool\n\
             Right Mouse: pull or drop a prop\n\
             Left Mouse: throw a held prop\n\
             Wall run: jump into the long left wall while moving forward",
        ),
        BackgroundColor(Color::BLACK.with_alpha(0.3)),
    ));
}

fn tune_player_camera(mut cameras: Query<&mut Projection, With<Camera3d>>) {
    for mut projection in &mut cameras {
        if let Projection::Perspective(perspective) = &mut *projection {
            perspective.near = 0.03;
        }
    }
}

fn capture_cursor(mut cursor: Single<&mut CursorOptions>) {
    cursor.grab_mode = CursorGrabMode::Locked;
    cursor.visible = false;
}

fn release_cursor(mut cursor: Single<&mut CursorOptions>) {
    cursor.visible = true;
    cursor.grab_mode = CursorGrabMode::None;
}

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
struct SpawnPlayer;

#[derive(Component)]
struct Player;

#[derive(Component, Default)]
#[component(on_add = PlayerInput::on_add)]
struct PlayerInput;

impl PlayerInput {
    fn on_add(mut world: DeferredWorld, ctx: HookContext) {
        world
            .commands()
            .entity(ctx.entity)
            .insert(actions!(PlayerInput[
                (
                    Action::<Movement>::new(),
                    DeadZone::default(),
                    Bindings::spawn((Cardinal::wasd_keys(), Axial::left_stick()))
                ),
                (
                    Action::<Jump>::new(),
                    Hold::new(0.0),
                    bindings![KeyCode::Space, GamepadButton::South],
                ),
                (
                    Action::<Tac>::new(),
                    Press::default(),
                    bindings![KeyCode::Space, GamepadButton::South],
                ),
                (
                    Action::<Crane>::new(),
                    Press::default(),
                    bindings![KeyCode::Space, GamepadButton::South],
                ),
                (
                    Action::<Mantle>::new(),
                    Hold::new(0.18),
                    bindings![KeyCode::Space, GamepadButton::South],
                ),
                (
                    Action::<Climbdown>::new(),
                    bindings![KeyCode::ControlLeft, GamepadButton::LeftTrigger2],
                ),
                (
                    Action::<Crouch>::new(),
                    bindings![KeyCode::ControlLeft, GamepadButton::LeftTrigger2],
                ),
                (
                    Action::<SwimUp>::new(),
                    bindings![KeyCode::Space, GamepadButton::South],
                ),
                (
                    Action::<PullObject>::new(),
                    ActionSettings {
                        consume_input: true,
                        ..default()
                    },
                    Press::default(),
                    bindings![MouseButton::Right],
                ),
                (
                    Action::<DropObject>::new(),
                    ActionSettings {
                        consume_input: true,
                        ..default()
                    },
                    Press::default(),
                    bindings![MouseButton::Right],
                ),
                (
                    Action::<ThrowObject>::new(),
                    ActionSettings {
                        consume_input: true,
                        ..default()
                    },
                    Press::default(),
                    bindings![MouseButton::Left],
                ),
                (
                    Action::<RotateCamera>::new(),
                    Bindings::spawn((
                        Spawn((Binding::mouse_motion(), Scale::splat(0.07))),
                        Axial::right_stick().with((Scale::splat(4.0), DeadZone::default())),
                    ))
                ),
            ]));
    }
}

#[derive(Component, Default)]
struct WallRunState {
    active: bool,
    time_left: f32,
    cooldown: f32,
}

fn apply_wall_run(
    time: Res<Time>,
    mut players: Query<
        (
            &CharacterLook,
            &CharacterControllerState,
            &CharacterControllerOutput,
            &WaterState,
            &mut LinearVelocity,
            &mut WallRunState,
        ),
        With<Player>,
    >,
) {
    let dt = time.delta_secs();
    for (look, state, output, water, mut velocity, mut wall_run) in &mut players {
        wall_run.cooldown = (wall_run.cooldown - dt).max(0.0);

        let horizontal_velocity = Vec3::new(velocity.x, 0.0, velocity.z);
        let horizontal_speed = horizontal_velocity.length();
        let look_forward = (look.to_quat() * Vec3::NEG_Z)
            .with_y(0.0)
            .normalize_or_zero();
        let travel_dir = if horizontal_speed > 0.1 {
            horizontal_velocity.normalize()
        } else {
            look_forward
        };

        let Some(wall_normal) = find_wall_normal(output, travel_dir) else {
            wall_run.active = false;
            continue;
        };

        if wall_run.cooldown > 0.0
            || state.grounded.is_some()
            || state.mantle.is_some()
            || state.crane_height_left.is_some()
            || water.level > WaterLevel::Feet
            || horizontal_speed < WALL_RUN_MIN_SPEED
        {
            wall_run.active = false;
            continue;
        }

        if !wall_run.active {
            wall_run.active = true;
            wall_run.time_left = WALL_RUN_DURATION;
        } else {
            wall_run.time_left -= dt;
            if wall_run.time_left <= 0.0 {
                wall_run.active = false;
                wall_run.cooldown = WALL_RUN_COOLDOWN;
                continue;
            }
        }

        let mut wall_dir = Vec3::Y.cross(wall_normal).normalize_or_zero();
        if wall_dir.dot(travel_dir) < 0.0 {
            wall_dir = -wall_dir;
        }

        let run_speed = horizontal_speed.max(WALL_RUN_SPEED);
        velocity.x = wall_dir.x * run_speed - wall_normal.x * WALL_RUN_STICK_SPEED;
        velocity.z = wall_dir.z * run_speed - wall_normal.z * WALL_RUN_STICK_SPEED;
        velocity.y = velocity.y.max(-WALL_RUN_FALL_SPEED);
    }
}

fn find_wall_normal(output: &CharacterControllerOutput, travel_dir: Vec3) -> Option<Vec3> {
    let mut best_normal = None;
    let mut best_alignment = 0.15;

    for touch in &output.touching_entities {
        if touch.normal.y.abs() > 0.2 {
            continue;
        }
        let wall_normal = Vec3::new(touch.normal.x, 0.0, touch.normal.z).normalize_or_zero();
        if wall_normal == Vec3::ZERO {
            continue;
        }
        let alignment = (-wall_normal).dot(travel_dir);
        if alignment > best_alignment {
            best_alignment = alignment;
            best_normal = Some(wall_normal);
        }
    }

    best_normal
}

#[derive(Component)]
struct Mover {
    start: Vec3,
    end: Vec3,
    speed: f32,
    direction: f32,
}

fn move_movers(mut movers: Query<(&GlobalTransform, &mut LinearVelocity, &mut Mover)>) {
    for (transform, mut velocity, mut mover) in &mut movers {
        let target = if mover.direction > 0.0 {
            mover.end
        } else {
            mover.start
        };
        let offset = target - transform.translation();
        if offset.length_squared() < 0.25 {
            mover.direction *= -1.0;
        }

        let target = if mover.direction > 0.0 {
            mover.end
        } else {
            mover.start
        };
        velocity.0 = (target - transform.translation()).normalize_or_zero() * mover.speed;
    }
}

fn spawn_static_box<'a>(
    commands: &'a mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    name: &str,
    size: Vec3,
    translation: Vec3,
    color: Color,
) -> EntityCommands<'a> {
    commands.spawn((
        Name::new(name.to_owned()),
        Mesh3d(meshes.add(Cuboid::new(size.x, size.y, size.z))),
        MeshMaterial3d(materials.add(color)),
        Transform::from_translation(translation),
        RigidBody::Static,
        Collider::cuboid(size.x, size.y, size.z),
        CollisionLayers::new(CollisionLayer::Default, LayerMask::ALL),
    ))
}

fn spawn_dynamic_box(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    name: &str,
    size: Vec3,
    translation: Vec3,
    color: Color,
) {
    commands.spawn((
        Name::new(name.to_owned()),
        Mesh3d(meshes.add(Cuboid::new(size.x, size.y, size.z))),
        MeshMaterial3d(materials.add(color)),
        Transform::from_translation(translation),
        RigidBody::Dynamic,
        Collider::cuboid(size.x, size.y, size.z),
        CollisionLayers::new(CollisionLayer::Prop, LayerMask::ALL),
        Mass(140.0),
        LinearDamping(1.8),
        AngularDamping(2.4),
        Friction::new(1.2),
    ));
}

fn spawn_water_box(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    size: Vec3,
    translation: Vec3,
) {
    commands.spawn((
        Name::new("Water"),
        Mesh3d(meshes.add(Cuboid::new(size.x, size.y, size.z))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgba(0.15, 0.45, 0.95, 0.45),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.15,
            reflectance: 0.5,
            ..default()
        })),
        Transform::from_translation(translation),
        RigidBody::Static,
        Collider::cuboid(size.x, size.y, size.z),
        CollisionLayers::new(CollisionLayer::Default, LayerMask::ALL),
        Water { speed: 0.7 },
    ));
}

fn spawn_mover(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    name: &str,
    size: Vec3,
    start: Vec3,
    end: Vec3,
    speed: f32,
    color: Color,
) {
    commands.spawn((
        Name::new(name.to_owned()),
        Mesh3d(meshes.add(Cuboid::new(size.x, size.y, size.z))),
        MeshMaterial3d(materials.add(color)),
        Transform::from_translation(start),
        RigidBody::Kinematic,
        TransformInterpolation,
        Collider::cuboid(size.x, size.y, size.z),
        CollisionLayers::new(CollisionLayer::Default, LayerMask::ALL),
        Mover {
            start,
            end,
            speed,
            direction: 1.0,
        },
    ));
}

#[derive(Debug, PhysicsLayer, Default, Clone, Copy)]
enum CollisionLayer {
    #[default]
    Default,
    Player,
    Prop,
}
