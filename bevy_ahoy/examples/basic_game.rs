use std::{
    collections::HashSet,
    time::{SystemTime, UNIX_EPOCH},
};

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
use bevy_time::Stopwatch;

use crate::util::{ExampleUtilPlugin, StableGround};

mod util;

const ROOM_HEIGHT: f32 = 1.2;
const ROOM_CLEARANCE_HEIGHT: f32 = 3.4;
const CELL_SIZE: f32 = 22.0;
const PLAYER_SPAWN_CLEARANCE: f32 = 2.4;
const CHECKPOINT_INTERVAL: usize = 3;
const WALL_RUN_SPEED: f32 = 11.5;
const WALL_RUN_STICK_SPEED: f32 = 2.0;
const WALL_RUN_FALL_SPEED: f32 = 2.25;
const WALL_RUN_MIN_SPEED: f32 = 4.0;
const WALL_RUN_DURATION: f32 = 0.95;
const WALL_RUN_COOLDOWN: f32 = 0.2;
const TREASURE_PICKUP_RADIUS: f32 = 1.8;
const CHECKPOINT_RADIUS: f32 = 2.8;
const SUMMIT_RADIUS: f32 = 4.5;
const SHORTCUT_TRIGGER_RADIUS: f32 = 2.0;

type SocketMask = u32;
const SOCKET_SAFE_REST: SocketMask = 1 << 0;
const SOCKET_MANTLE_ENTRY: SocketMask = 1 << 1;
const SOCKET_WALLRUN_READY: SocketMask = 1 << 2;
const SOCKET_HAZARD_BRANCH: SocketMask = 1 << 3;
const SOCKET_SHORTCUT_ANCHOR: SocketMask = 1 << 4;

fn main() -> AppExit {
    App::new()
        .register_type::<SpawnPlayer>()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Window {
                    title: "Bevy Ahoy Chronoclimb".into(),
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
        .insert_resource(ClearColor(tailwind::SLATE_900.into()))
        .insert_resource(RunDirector::default())
        .add_systems(Startup, (setup_scene, setup_hud).chain())
        .add_systems(PostStartup, tune_player_camera)
        .add_systems(
            Update,
            (
                capture_cursor.run_if(input_just_pressed(MouseButton::Left)),
                release_cursor.run_if(input_just_pressed(KeyCode::Escape)),
                tick_run_timer,
                queue_run_controls,
                activate_checkpoints,
                collect_treasures,
                activate_shortcuts,
                sync_shortcut_bridges,
                detect_summit_completion,
                detect_failures,
                update_hud,
                process_run_request,
            ),
        )
        .add_systems(
            FixedUpdate,
            (move_movers, update_crumbling_platforms, apply_wind),
        )
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
    let blueprint = build_run_blueprint(current_run_seed());
    let snapshot = spawn_run_world(
        &blueprint,
        &HashSet::default(),
        &HashSet::default(),
        &mut commands,
        &mut meshes,
        &mut materials,
    );

    commands.insert_resource(RunState::new(&blueprint, snapshot));

    commands.spawn((
        Name::new("Spawn Point"),
        SpawnPlayer,
        Transform::from_translation(blueprint.spawn),
        GlobalTransform::default(),
    ));

    let player = commands
        .spawn((
            Name::new("Player"),
            Player,
            PlayerInput,
            WallRunState::default(),
            CharacterController {
                speed: 9.2,
                air_speed: 2.3,
                jump_height: 1.9,
                max_speed: 26.0,
                ..default()
            },
            RigidBody::Kinematic,
            Collider::cylinder(0.7, 1.8),
            CollisionLayers::new(CollisionLayer::Player, LayerMask::ALL),
            Mass(45.0),
            StableGround::default(),
            Transform::from_translation(blueprint.spawn),
            Position(blueprint.spawn),
            CharacterLook::default(),
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
                preferred_distance: 1.2,
                linear_velocity_easing: 0.8,
                ..default()
            },
            pull: PickupPullConfig {
                max_prop_mass: 350.0,
                ..default()
            },
            ..default()
        },
    ));
}

fn setup_hud(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: px(12.0),
            left: px(12.0),
            max_width: px(480.0),
            padding: UiRect::all(px(12.0)),
            ..default()
        },
        Text::new("Chronoclimb\nBooting run director..."),
        BackgroundColor(Color::BLACK.with_alpha(0.44)),
        RunHud,
    ));
}

fn update_hud(
    run: Res<RunState>,
    players: Query<&Transform, With<Player>>,
    mut hud: Single<&mut Text, With<RunHud>>,
) {
    let Ok(player) = players.single() else {
        return;
    };

    let current_height = player.translation.y.max(0.0);
    let summit_height = run.summit.y.max(1.0);
    let progress = (current_height / summit_height).clamp(0.0, 1.0) * 100.0;
    let elapsed = run.timer.elapsed_secs();
    let objective = if run.finished {
        "Summit reached. F5 reruns this seed, F6 rolls a new tower."
    } else {
        "Goal: reach the beacon at the top. F5 same seed, F6 fresh seed."
    };

    hud.0 = format!(
        "Chronoclimb\n\
         Seed: {seed:016x}\n\
         Floors: {floors} | Checkpoint: {checkpoint}/{checkpoint_total}\n\
         Height: {height:.1}m / {summit:.1}m ({progress:.0}%)\n\
         Time: {elapsed:.1}s | Deaths: {deaths}\n\
         Treasures: {treasures}/{treasure_total} | Shortcuts: {shortcuts}\n\
         Gen: attempts {attempts}, repairs {repairs}, overlaps {overlaps}, clearance {clearance}, reach {reach}\n\
         {objective}\n\
         Controls: WASD move | hold Space bhop/jump | Ctrl crouch/mantle\n\
         RMB pull/drop props | LMB throw | R reset to checkpoint",
        seed = run.seed,
        floors = run.floors,
        checkpoint = run.current_checkpoint + 1,
        checkpoint_total = run.checkpoints.len(),
        height = current_height,
        summit = summit_height,
        progress = progress,
        elapsed = elapsed,
        deaths = run.deaths,
        treasures = run.collected_treasures.len(),
        treasure_total = run.total_treasures,
        shortcuts = run.unlocked_shortcuts.len(),
        attempts = run.stats.attempts,
        repairs = run.stats.repairs,
        overlaps = run.stats.overlap_issues,
        clearance = run.stats.clearance_issues,
        reach = run.stats.reachability_issues,
        objective = objective,
    );
}

fn tick_run_timer(time: Res<Time>, mut run: ResMut<RunState>) {
    if !run.finished {
        run.timer.tick(time.delta());
    }
}

fn queue_run_controls(
    keys: Res<ButtonInput<KeyCode>>,
    run: Res<RunState>,
    mut director: ResMut<RunDirector>,
) {
    if director.pending.is_some() {
        return;
    }

    if keys.just_pressed(KeyCode::F5) {
        director.pending = Some(RunRequest {
            kind: RunRequestKind::RestartSameSeed,
            seed: run.seed,
        });
    } else if keys.just_pressed(KeyCode::F6) {
        director.pending = Some(RunRequest {
            kind: RunRequestKind::RestartNewSeed,
            seed: current_run_seed(),
        });
    }
}

fn activate_checkpoints(
    players: Query<&Transform, With<Player>>,
    pads: Query<(&Transform, &CheckpointPad), Without<SpawnPlayer>>,
    mut run: ResMut<RunState>,
    mut spawn_marker: Single<
        &mut Transform,
        (With<SpawnPlayer>, Without<Player>, Without<CheckpointPad>),
    >,
) {
    let Ok(player) = players.single() else {
        return;
    };

    for (transform, checkpoint) in &pads {
        let delta = transform.translation - player.translation;
        if delta.y.abs() < 2.0 && delta.xz().length() <= CHECKPOINT_RADIUS {
            if checkpoint.index > run.current_checkpoint {
                run.current_checkpoint = checkpoint.index;
                if let Some(spawn) = run.checkpoints.get(checkpoint.index).copied() {
                    spawn_marker.translation = spawn;
                }
            }
        }
    }
}

fn collect_treasures(
    mut commands: Commands,
    players: Query<&Transform, With<Player>>,
    treasures: Query<(Entity, &Transform, &TreasurePickup)>,
    mut run: ResMut<RunState>,
) {
    let Ok(player) = players.single() else {
        return;
    };

    for (entity, transform, pickup) in &treasures {
        if run.collected_treasures.contains(&pickup.id) {
            continue;
        }
        if transform.translation.distance(player.translation) <= TREASURE_PICKUP_RADIUS {
            run.collected_treasures.insert(pickup.id);
            commands.entity(entity).despawn();
        }
    }
}

fn activate_shortcuts(
    players: Query<&Transform, With<Player>>,
    triggers: Query<(&Transform, &ShortcutTrigger)>,
    mut run: ResMut<RunState>,
) {
    let Ok(player) = players.single() else {
        return;
    };

    for (transform, trigger) in &triggers {
        if run.unlocked_shortcuts.contains(&trigger.id) {
            continue;
        }
        if transform.translation.distance(player.translation) <= SHORTCUT_TRIGGER_RADIUS {
            run.unlocked_shortcuts.insert(trigger.id);
        }
    }
}

fn sync_shortcut_bridges(
    mut commands: Commands,
    run: Res<RunState>,
    bridges: Query<(Entity, &ShortcutBridge, Option<&Collider>)>,
) {
    for (entity, bridge, collider) in &bridges {
        if run.unlocked_shortcuts.contains(&bridge.id) && collider.is_none() {
            commands.entity(entity).insert((
                Collider::cuboid(bridge.size.x, bridge.size.y, bridge.size.z),
                CollisionLayers::new(CollisionLayer::Default, LayerMask::ALL),
            ));
        }
    }
}

fn detect_summit_completion(
    players: Query<&Transform, With<Player>>,
    summits: Query<&Transform, With<SummitGoal>>,
    mut run: ResMut<RunState>,
) {
    if run.finished {
        return;
    }

    let Ok(player) = players.single() else {
        return;
    };

    for transform in &summits {
        let delta = transform.translation - player.translation;
        if delta.y.abs() < 3.0 && delta.xz().length() <= SUMMIT_RADIUS {
            run.finished = true;
            break;
        }
    }
}

fn detect_failures(
    players: Query<(&Transform, &CharacterControllerOutput), With<Player>>,
    lethal: Query<(), With<LethalHazard>>,
    run: Res<RunState>,
    mut director: ResMut<RunDirector>,
) {
    if director.pending.is_some() {
        return;
    }

    let Ok((transform, output)) = players.single() else {
        return;
    };

    if transform.translation.y < run.death_plane {
        director.pending = Some(RunRequest {
            kind: RunRequestKind::Respawn,
            seed: run.seed,
        });
        return;
    }

    if output
        .touching_entities
        .iter()
        .any(|touch| lethal.get(touch.entity).is_ok())
    {
        director.pending = Some(RunRequest {
            kind: RunRequestKind::Respawn,
            seed: run.seed,
        });
    }
}

fn process_run_request(
    mut commands: Commands,
    mut director: ResMut<RunDirector>,
    mut run: ResMut<RunState>,
    generated: Query<Entity, With<GeneratedWorld>>,
    mut players: Query<
        (
            &mut Position,
            &mut Transform,
            &mut LinearVelocity,
            &mut CharacterLook,
            &mut WallRunState,
        ),
        With<Player>,
    >,
    mut camera: Query<&mut Transform, (With<Camera3d>, Without<Player>, Without<SpawnPlayer>)>,
    mut spawn_marker: Single<
        &mut Transform,
        (With<SpawnPlayer>, Without<Player>, Without<Camera3d>),
    >,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(request) = director.pending.take() else {
        return;
    };

    for entity in &generated {
        commands.entity(entity).despawn();
    }

    let (seed, reset_progress) = match request.kind {
        RunRequestKind::Respawn => (request.seed, false),
        RunRequestKind::RestartSameSeed => (request.seed, true),
        RunRequestKind::RestartNewSeed => (request.seed, true),
    };

    let blueprint = build_run_blueprint(seed);

    if reset_progress {
        run.seed = seed;
        run.timer = Stopwatch::new();
        run.finished = false;
        run.deaths = 0;
        run.current_checkpoint = 0;
        run.collected_treasures.clear();
        run.unlocked_shortcuts.clear();
    } else {
        run.deaths += 1;
        run.finished = false;
    }

    let snapshot = spawn_run_world(
        &blueprint,
        &run.collected_treasures,
        &run.unlocked_shortcuts,
        &mut commands,
        &mut meshes,
        &mut materials,
    );

    run.apply_blueprint(&blueprint, snapshot);

    let spawn = run
        .checkpoints
        .get(run.current_checkpoint)
        .copied()
        .unwrap_or(blueprint.spawn);
    spawn_marker.translation = spawn;

    if let Ok((mut position, mut transform, mut velocity, mut look, mut wall_run)) =
        players.single_mut()
    {
        position.0 = spawn;
        transform.translation = spawn;
        velocity.0 = Vec3::ZERO;
        *look = CharacterLook::default();
        *wall_run = WallRunState::default();
    }

    if let Ok(mut camera_transform) = camera.single_mut() {
        camera_transform.rotation = Quat::IDENTITY;
    }
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

#[derive(Component)]
struct RunHud;

#[derive(Component)]
struct GeneratedWorld;

#[derive(Component)]
struct CheckpointPad {
    index: usize,
}

#[derive(Component)]
struct SummitGoal;

#[derive(Component)]
struct TreasurePickup {
    id: u64,
}

#[derive(Component)]
struct ShortcutTrigger {
    id: u64,
}

#[derive(Component)]
struct ShortcutBridge {
    id: u64,
    size: Vec3,
}

#[derive(Component)]
struct WindZone {
    size: Vec3,
    direction: Vec3,
    strength: f32,
    gust: f32,
}

#[derive(Component)]
struct LethalHazard;

#[derive(Component)]
struct Mover {
    start: Vec3,
    end: Vec3,
    speed: f32,
    direction: f32,
}

#[derive(Component)]
struct CrumblingPlatform {
    timer: Timer,
    sink_speed: f32,
    armed: bool,
    collapsed: bool,
}

#[derive(Resource, Default)]
struct RunDirector {
    pending: Option<RunRequest>,
}

struct RunRequest {
    kind: RunRequestKind,
    seed: u64,
}

enum RunRequestKind {
    Respawn,
    RestartSameSeed,
    RestartNewSeed,
}

#[derive(Resource)]
struct RunState {
    seed: u64,
    floors: usize,
    summit: Vec3,
    death_plane: f32,
    checkpoints: Vec<Vec3>,
    current_checkpoint: usize,
    deaths: u32,
    timer: Stopwatch,
    finished: bool,
    total_treasures: usize,
    collected_treasures: HashSet<u64>,
    unlocked_shortcuts: HashSet<u64>,
    stats: GenerationStats,
}

impl RunState {
    fn new(blueprint: &RunBlueprint, snapshot: RunSnapshot) -> Self {
        let mut state = Self {
            seed: blueprint.seed,
            floors: blueprint.floors,
            summit: blueprint.summit,
            death_plane: blueprint.death_plane,
            checkpoints: snapshot.checkpoints,
            current_checkpoint: 0,
            deaths: 0,
            timer: Stopwatch::new(),
            finished: false,
            total_treasures: snapshot.total_treasures,
            collected_treasures: HashSet::default(),
            unlocked_shortcuts: HashSet::default(),
            stats: blueprint.stats.clone(),
        };
        if state.checkpoints.is_empty() {
            state.checkpoints.push(blueprint.spawn);
        }
        state
    }

    fn apply_blueprint(&mut self, blueprint: &RunBlueprint, snapshot: RunSnapshot) {
        self.seed = blueprint.seed;
        self.floors = blueprint.floors;
        self.summit = blueprint.summit;
        self.death_plane = blueprint.death_plane;
        self.checkpoints = snapshot.checkpoints;
        self.total_treasures = snapshot.total_treasures;
        self.stats = blueprint.stats.clone();
        if self.current_checkpoint >= self.checkpoints.len() {
            self.current_checkpoint = self.checkpoints.len().saturating_sub(1);
        }
    }
}

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

fn update_crumbling_platforms(
    time: Res<Time>,
    mut commands: Commands,
    players: Query<&CharacterControllerState, With<Player>>,
    mut platforms: Query<(Entity, &mut Transform, &mut CrumblingPlatform)>,
) {
    let grounded_entity = players
        .single()
        .ok()
        .and_then(|state| state.grounded.map(|ground| ground.entity));

    for (entity, mut transform, mut crumble) in &mut platforms {
        if Some(entity) == grounded_entity && !crumble.armed {
            crumble.armed = true;
            crumble.timer.reset();
        }

        if crumble.armed && !crumble.collapsed {
            crumble.timer.tick(time.delta());
            if crumble.timer.is_finished() {
                crumble.collapsed = true;
                commands.entity(entity).remove::<Collider>();
            }
        }

        if crumble.collapsed {
            transform.translation.y -= crumble.sink_speed * time.delta_secs();
        }
    }
}

fn apply_wind(
    time: Res<Time>,
    mut players: Query<(&Transform, &mut LinearVelocity), With<Player>>,
    wind_zones: Query<(&Transform, &WindZone)>,
) {
    let Ok((player, mut velocity)) = players.single_mut() else {
        return;
    };

    for (transform, zone) in &wind_zones {
        let local = player.translation - transform.translation;
        let half = zone.size * 0.5;
        if local.x.abs() <= half.x && local.y.abs() <= half.y && local.z.abs() <= half.z {
            let pulse =
                0.7 + 0.3 * ((time.elapsed_secs() * zone.gust).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
            velocity.0 +=
                zone.direction.normalize_or_zero() * zone.strength * pulse * time.delta_secs();
        }
    }
}

#[derive(Clone)]
struct RunBlueprint {
    seed: u64,
    floors: usize,
    rooms: Vec<RoomPlan>,
    segments: Vec<SegmentPlan>,
    branches: Vec<BranchPlan>,
    spawn: Vec3,
    summit: Vec3,
    death_plane: f32,
    stats: GenerationStats,
}

#[derive(Clone)]
struct RoomPlan {
    index: usize,
    cell: IVec2,
    top: Vec3,
    size: Vec2,
    theme: Theme,
    checkpoint_slot: Option<usize>,
}

#[derive(Clone)]
struct SegmentPlan {
    index: usize,
    from: usize,
    to: usize,
    kind: ModuleKind,
    difficulty: f32,
    seed: u64,
    shortcut_id: Option<u64>,
    exit_socket: SocketMask,
}

#[derive(Clone)]
struct BranchPlan {
    room_index: usize,
    dir: IVec2,
    top: Vec3,
    size: Vec2,
    theme: Theme,
    kind: BranchKind,
    seed: u64,
    treasure_id: Option<u64>,
    shortcut_id: Option<u64>,
}

#[derive(Clone, Default)]
struct GenerationStats {
    attempts: u32,
    repairs: u32,
    downgraded_segments: u32,
    pruned_branches: u32,
    overlap_issues: usize,
    clearance_issues: usize,
    reachability_issues: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ModuleKind {
    StairRun,
    MantleStack,
    WallRunHall,
    LiftChasm,
    CrumbleBridge,
    SweepHall,
    PistonGate,
    WindTunnel,
    IceSpine,
    WaterGarden,
    TimedDoor,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BranchKind {
    TreasureAlcove,
    PropCache,
    ShortcutLever,
    RiskDetour,
}

#[derive(Clone, Copy)]
enum Theme {
    Stone,
    Overgrown,
    Frost,
    Ember,
}

#[derive(Clone, Copy)]
struct ModuleTemplate {
    kind: ModuleKind,
    entry: SocketMask,
    exit: SocketMask,
    min_difficulty: f32,
    max_difficulty: f32,
    weight: u32,
    min_rise: f32,
    max_rise: f32,
    min_gap: f32,
    max_gap: f32,
    shortcut_eligible: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum OwnerTag {
    Room(usize),
    Segment(usize),
    Branch(usize),
    Summit,
}

#[derive(Clone)]
struct SolidSpec {
    owner: OwnerTag,
    label: String,
    center: Vec3,
    size: Vec3,
    paint: PaintStyle,
    body: SolidBody,
    friction: Option<f32>,
    extra: ExtraKind,
}

#[derive(Clone)]
enum SolidBody {
    Static,
    Decoration,
    DynamicProp,
    Water { speed: f32 },
    Moving { end: Vec3, speed: f32, lethal: bool },
    Crumbling { delay: f32, sink_speed: f32 },
    ShortcutBridge { id: u64, active: bool },
}

#[derive(Clone, Copy)]
enum ExtraKind {
    None,
    Checkpoint { index: usize },
    SummitGoal,
    Treasure { id: u64 },
    ShortcutTrigger { id: u64 },
}

#[derive(Clone)]
enum FeatureSpec {
    WindZone {
        center: Vec3,
        size: Vec3,
        direction: Vec3,
        strength: f32,
        gust: f32,
    },
    PointLight {
        center: Vec3,
        intensity: f32,
        range: f32,
        color: Color,
    },
}

#[derive(Clone, Copy)]
enum PaintStyle {
    ThemeFloor(Theme),
    ThemeAccent(Theme),
    ThemeShadow(Theme),
    Prop(Theme),
    Summit,
    Checkpoint,
    Treasure,
    Hazard,
    Shortcut,
    Ice,
    Water,
}

#[derive(Clone)]
struct ClearanceProbe {
    owner: OwnerTag,
    center: Vec3,
    size: Vec3,
}

#[derive(Default, Clone)]
struct ModuleLayout {
    solids: Vec<SolidSpec>,
    features: Vec<FeatureSpec>,
    clearances: Vec<ClearanceProbe>,
}

struct RunSnapshot {
    checkpoints: Vec<Vec3>,
    total_treasures: usize,
}

#[derive(Clone, Copy)]
struct AabbVolume {
    owner: OwnerTag,
    center: Vec3,
    size: Vec3,
}

#[derive(Default)]
struct ValidationOutcome {
    overlap_issues: usize,
    clearance_issues: usize,
    reachability_issues: usize,
    first_overlap: Option<(OwnerTag, OwnerTag)>,
    first_clearance: Option<(OwnerTag, OwnerTag)>,
    first_unreachable_segment: Option<usize>,
}

fn build_run_blueprint(seed: u64) -> RunBlueprint {
    let mut best_blueprint = None;
    let mut best_score = usize::MAX;

    for attempt in 0..18 {
        let mut rng = RunRng::new(seed ^ (attempt as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut blueprint = draft_run_blueprint(seed, &mut rng);
        let repair_stats = repair_run_blueprint(&mut blueprint, &mut rng);
        let validation = validate_run_blueprint(&blueprint);

        blueprint.stats.attempts = attempt + 1;
        blueprint.stats.repairs = repair_stats.repairs;
        blueprint.stats.downgraded_segments = repair_stats.downgraded_segments;
        blueprint.stats.pruned_branches = repair_stats.pruned_branches;
        blueprint.stats.overlap_issues = validation.overlap_issues;
        blueprint.stats.clearance_issues = validation.clearance_issues;
        blueprint.stats.reachability_issues = validation.reachability_issues;

        let score = validation.overlap_issues * 10
            + validation.clearance_issues * 8
            + validation.reachability_issues * 6
            + blueprint
                .branches
                .len()
                .saturating_sub(repair_stats.pruned_branches as usize);

        if validation.overlap_issues == 0
            && validation.clearance_issues == 0
            && validation.reachability_issues == 0
        {
            return blueprint;
        }

        if score < best_score {
            best_score = score;
            best_blueprint = Some(blueprint);
        }
    }

    best_blueprint.unwrap_or_else(|| fallback_blueprint(seed))
}

#[derive(Default)]
struct RepairStats {
    repairs: u32,
    downgraded_segments: u32,
    pruned_branches: u32,
}

fn draft_run_blueprint(seed: u64, rng: &mut RunRng) -> RunBlueprint {
    let floors = rng.range_usize(12, 17);
    let mut rooms = Vec::with_capacity(floors);
    let mut segments = Vec::with_capacity(floors.saturating_sub(1));
    let mut occupied_rooms = HashSet::default();
    let mut occupied_branches = HashSet::default();
    let theme_offset = (rng.next_u64() % 4) as usize;

    let mut cell = IVec2::ZERO;
    let mut heading = IVec2::NEG_Y;
    let mut current_socket = SOCKET_SAFE_REST;
    let mut current_height = 2.0;
    let mut next_checkpoint_slot = 1_usize;

    occupied_rooms.insert(cell);
    rooms.push(RoomPlan {
        index: 0,
        cell,
        top: Vec3::new(0.0, current_height, 0.0),
        size: Vec2::splat(13.8),
        theme: theme_for(0, floors, theme_offset),
        checkpoint_slot: Some(0),
    });

    for index in 1..floors {
        let difficulty = index as f32 / (floors.saturating_sub(1).max(1)) as f32;
        let next_cell = choose_next_cell(rng, cell, heading, &occupied_rooms);
        heading = next_cell - cell;
        cell = next_cell;
        occupied_rooms.insert(cell);

        let room_size = Vec2::splat(lerp(13.6, 9.1, difficulty)).max(Vec2::splat(8.8));
        let projected_gap = projected_gap(rooms.last().unwrap().size, room_size);
        let template = choose_module_template(rng, current_socket, difficulty, projected_gap);
        let rise = rng.range_f32(template.min_rise, template.max_rise) + difficulty * 0.4;
        current_height += rise;

        let is_checkpoint = index % CHECKPOINT_INTERVAL == 0 || index == floors - 1;
        let checkpoint_slot = if is_checkpoint {
            let slot = next_checkpoint_slot;
            next_checkpoint_slot += 1;
            Some(slot)
        } else {
            None
        };

        rooms.push(RoomPlan {
            index,
            cell,
            top: Vec3::new(
                cell.x as f32 * CELL_SIZE,
                current_height,
                cell.y as f32 * CELL_SIZE,
            ),
            size: if index == floors - 1 {
                Vec2::splat(14.2)
            } else {
                room_size
            },
            theme: theme_for(index, floors, theme_offset),
            checkpoint_slot,
        });

        let shortcut_id = if template.shortcut_eligible && difficulty > 0.45 && rng.chance(0.45) {
            Some(seed ^ (index as u64 + 1).wrapping_mul(0xA5A5_5A5A_1234_5678))
        } else {
            None
        };

        segments.push(SegmentPlan {
            index: index - 1,
            from: index - 1,
            to: index,
            kind: template.kind,
            difficulty,
            seed: rng.next_u64(),
            shortcut_id,
            exit_socket: template.exit,
        });
        current_socket = template.exit;
    }

    let branches = generate_side_branches(seed, rng, &rooms, &segments, &mut occupied_branches);
    let spawn = rooms[0].top + Vec3::new(0.0, PLAYER_SPAWN_CLEARANCE, rooms[0].size.y * 0.18);
    let summit = rooms.last().unwrap().top + Vec3::new(0.0, 1.4, 0.0);
    let death_plane = -18.0;

    RunBlueprint {
        seed,
        floors,
        rooms,
        segments,
        branches,
        spawn,
        summit,
        death_plane,
        stats: GenerationStats::default(),
    }
}

fn repair_run_blueprint(blueprint: &mut RunBlueprint, rng: &mut RunRng) -> RepairStats {
    let mut stats = RepairStats::default();

    for index in 1..blueprint.rooms.len() {
        let difficulty = index as f32 / blueprint.rooms.len().max(1) as f32;
        let clamped = lerp(13.6, 9.1, difficulty).clamp(8.8, 14.2);
        blueprint.rooms[index].size = Vec2::splat(clamped);
    }

    for segment in &mut blueprint.segments {
        if !segment_reachable(segment, &blueprint.rooms) {
            segment.kind = safe_fallback_kind(segment.difficulty);
            segment.exit_socket = module_template(segment.kind).exit;
            stats.repairs += 1;
            stats.downgraded_segments += 1;
        }
    }

    for _ in 0..10 {
        let validation = validate_run_blueprint(blueprint);
        if validation.overlap_issues == 0
            && validation.clearance_issues == 0
            && validation.reachability_issues == 0
        {
            break;
        }

        if let Some(segment_index) = validation.first_unreachable_segment {
            if let Some(segment) = blueprint.segments.get_mut(segment_index) {
                segment.kind = safe_fallback_kind(segment.difficulty);
                segment.exit_socket = module_template(segment.kind).exit;
                stats.repairs += 1;
                stats.downgraded_segments += 1;
                continue;
            }
        }

        if let Some((a, b)) = validation.first_overlap.or(validation.first_clearance) {
            if prune_branch_owner(blueprint, a) || prune_branch_owner(blueprint, b) {
                stats.repairs += 1;
                stats.pruned_branches += 1;
                continue;
            }

            if downgrade_segment_owner(blueprint, a, rng)
                || downgrade_segment_owner(blueprint, b, rng)
            {
                stats.repairs += 1;
                stats.downgraded_segments += 1;
                continue;
            }

            if lift_room_owner(blueprint, a) || lift_room_owner(blueprint, b) {
                stats.repairs += 1;
                continue;
            }
        }

        break;
    }

    stats
}

fn fallback_blueprint(seed: u64) -> RunBlueprint {
    let mut rng = RunRng::new(seed ^ 0xFEED_FACE_CAFE_BEEF);
    let mut blueprint = draft_run_blueprint(seed, &mut rng);
    blueprint.branches.clear();
    for segment in &mut blueprint.segments {
        segment.kind = if segment.difficulty > 0.55 {
            ModuleKind::LiftChasm
        } else {
            ModuleKind::StairRun
        };
        segment.exit_socket = module_template(segment.kind).exit;
        segment.shortcut_id = None;
    }
    let validation = validate_run_blueprint(&blueprint);
    blueprint.stats.attempts = 1;
    blueprint.stats.overlap_issues = validation.overlap_issues;
    blueprint.stats.clearance_issues = validation.clearance_issues;
    blueprint.stats.reachability_issues = validation.reachability_issues;
    blueprint
}

fn generate_side_branches(
    seed: u64,
    rng: &mut RunRng,
    rooms: &[RoomPlan],
    segments: &[SegmentPlan],
    occupied_branches: &mut HashSet<IVec2>,
) -> Vec<BranchPlan> {
    let mut branches = Vec::new();
    let occupied_rooms = rooms.iter().map(|room| room.cell).collect::<HashSet<_>>();

    for room in rooms.iter().skip(1).take(rooms.len().saturating_sub(2)) {
        let difficulty = room.index as f32 / rooms.len().max(1) as f32;
        let main_forward = if room.index < rooms.len() - 1 {
            rooms[room.index + 1].cell - room.cell
        } else {
            room.cell - rooms[room.index - 1].cell
        };
        let branch_dirs = [
            IVec2::new(-main_forward.y, main_forward.x),
            IVec2::new(main_forward.y, -main_forward.x),
        ];

        for dir in branch_dirs {
            if dir == IVec2::ZERO {
                continue;
            }
            let branch_cell = room.cell + dir;
            if occupied_rooms.contains(&branch_cell) || occupied_branches.contains(&branch_cell) {
                continue;
            }
            if !rng.chance(if difficulty > 0.3 { 0.34 } else { 0.2 }) {
                continue;
            }

            let incoming = segments.get(room.index.saturating_sub(1));
            let linked_shortcut = incoming.and_then(|segment| segment.shortcut_id);
            let kind = if let Some(shortcut_id) = linked_shortcut {
                if !branches
                    .iter()
                    .any(|branch: &BranchPlan| branch.shortcut_id == Some(shortcut_id))
                    && rng.chance(0.58)
                {
                    BranchKind::ShortcutLever
                } else if difficulty > 0.55 && rng.chance(0.45) {
                    BranchKind::RiskDetour
                } else if rng.chance(0.5) {
                    BranchKind::TreasureAlcove
                } else {
                    BranchKind::PropCache
                }
            } else if difficulty > 0.55 && rng.chance(0.45) {
                BranchKind::RiskDetour
            } else if rng.chance(0.55) {
                BranchKind::TreasureAlcove
            } else {
                BranchKind::PropCache
            };

            let branch_top = room.top
                + Vec3::new(dir.x as f32, 0.0, dir.y as f32) * (CELL_SIZE * 0.68)
                + Vec3::Y * rng.range_f32(0.2, 1.8);
            let size = Vec2::new(rng.range_f32(5.0, 6.8), rng.range_f32(5.0, 6.8));
            let treasure_id = matches!(kind, BranchKind::TreasureAlcove | BranchKind::RiskDetour)
                .then_some(seed ^ rng.next_u64());
            let shortcut_id = matches!(kind, BranchKind::ShortcutLever).then_some(
                linked_shortcut.unwrap_or_else(|| seed ^ ((room.index as u64 + 1) << 9)),
            );

            occupied_branches.insert(branch_cell);
            branches.push(BranchPlan {
                room_index: room.index,
                dir,
                top: branch_top,
                size,
                theme: room.theme,
                kind,
                seed: rng.next_u64(),
                treasure_id,
                shortcut_id,
            });
            break;
        }
    }

    branches
}

fn validate_run_blueprint(blueprint: &RunBlueprint) -> ValidationOutcome {
    let mut volumes = Vec::new();
    let mut clearances = Vec::new();

    for room in &blueprint.rooms {
        let layout = build_room_layout(room);
        collect_layout_validation(&layout, &mut volumes, &mut clearances);
    }
    for segment in &blueprint.segments {
        let layout = build_segment_layout(segment, &blueprint.rooms, &HashSet::default());
        collect_layout_validation(&layout, &mut volumes, &mut clearances);
    }
    for (index, branch) in blueprint.branches.iter().enumerate() {
        let layout = build_branch_layout(index, branch, &blueprint.rooms, &HashSet::default());
        collect_layout_validation(&layout, &mut volumes, &mut clearances);
    }
    let layout = build_summit_layout(blueprint.rooms.last().unwrap(), blueprint.summit);
    collect_layout_validation(&layout, &mut volumes, &mut clearances);

    let mut outcome = ValidationOutcome::default();

    for i in 0..volumes.len() {
        for j in i + 1..volumes.len() {
            if volumes[i].owner == volumes[j].owner {
                continue;
            }
            if intersects(volumes[i], volumes[j], 0.04) {
                outcome.overlap_issues += 1;
                outcome
                    .first_overlap
                    .get_or_insert((volumes[i].owner, volumes[j].owner));
            }
        }
    }

    for clearance in &clearances {
        for volume in &volumes {
            if clearance.owner == volume.owner {
                continue;
            }
            if intersects(
                AabbVolume {
                    owner: clearance.owner,
                    center: clearance.center,
                    size: clearance.size,
                },
                *volume,
                0.02,
            ) {
                outcome.clearance_issues += 1;
                outcome
                    .first_clearance
                    .get_or_insert((clearance.owner, volume.owner));
                break;
            }
        }
    }

    for segment in &blueprint.segments {
        if !segment_reachable(segment, &blueprint.rooms) {
            outcome.reachability_issues += 1;
            outcome
                .first_unreachable_segment
                .get_or_insert(segment.index);
        }
    }

    outcome
}

fn collect_layout_validation(
    layout: &ModuleLayout,
    volumes: &mut Vec<AabbVolume>,
    clearances: &mut Vec<ClearanceProbe>,
) {
    for solid in &layout.solids {
        if let Some(aabb) = solid.preview_volume() {
            volumes.push(aabb);
        }
    }
    clearances.extend(layout.clearances.iter().cloned());
}

impl SolidSpec {
    fn preview_volume(&self) -> Option<AabbVolume> {
        let size = match &self.body {
            SolidBody::Static | SolidBody::Crumbling { .. } | SolidBody::ShortcutBridge { .. } => {
                self.size
            }
            SolidBody::Moving { end, .. } => {
                let min = (self.center - self.size * 0.5).min(*end - self.size * 0.5);
                let max = (self.center + self.size * 0.5).max(*end + self.size * 0.5);
                return Some(AabbVolume {
                    owner: self.owner,
                    center: (min + max) * 0.5,
                    size: max - min,
                });
            }
            SolidBody::Decoration | SolidBody::DynamicProp | SolidBody::Water { .. } => {
                return None;
            }
        };

        Some(AabbVolume {
            owner: self.owner,
            center: self.center,
            size,
        })
    }
}

fn prune_branch_owner(blueprint: &mut RunBlueprint, owner: OwnerTag) -> bool {
    let OwnerTag::Branch(index) = owner else {
        return false;
    };
    if index < blueprint.branches.len() {
        blueprint.branches.remove(index);
        return true;
    }
    false
}

fn downgrade_segment_owner(
    blueprint: &mut RunBlueprint,
    owner: OwnerTag,
    _rng: &mut RunRng,
) -> bool {
    let OwnerTag::Segment(index) = owner else {
        return false;
    };
    let Some(segment) = blueprint.segments.get_mut(index) else {
        return false;
    };
    segment.kind = safe_fallback_kind(segment.difficulty);
    segment.exit_socket = module_template(segment.kind).exit;
    segment.shortcut_id = None;
    true
}

fn lift_room_owner(blueprint: &mut RunBlueprint, owner: OwnerTag) -> bool {
    let OwnerTag::Room(index) = owner else {
        return false;
    };
    if index == 0 || index >= blueprint.rooms.len() {
        return false;
    }

    for room in blueprint.rooms.iter_mut().skip(index) {
        room.top.y += 0.9;
    }
    if let Some(last) = blueprint.rooms.last() {
        blueprint.summit = last.top + Vec3::Y * 1.4;
    }
    true
}

fn segment_reachable(segment: &SegmentPlan, rooms: &[RoomPlan]) -> bool {
    let from = &rooms[segment.from];
    let to = &rooms[segment.to];
    let template = module_template(segment.kind);
    let rise = to.top.y - from.top.y;
    let gap = edge_gap(from, to);
    rise >= template.min_rise - 0.2
        && rise <= template.max_rise + 0.9
        && gap >= template.min_gap - 1.4
        && gap <= template.max_gap + 1.8
}

fn choose_next_cell(
    rng: &mut RunRng,
    cell: IVec2,
    heading: IVec2,
    occupied: &HashSet<IVec2>,
) -> IVec2 {
    let dirs = [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y];
    let mut weighted = Vec::with_capacity(dirs.len());

    for dir in dirs {
        if dir == -heading {
            continue;
        }

        let next = cell + dir;
        let radius = next.x.abs().max(next.y.abs());
        let mut weight = 3_u32;
        if dir == heading {
            weight += 4;
        }
        if !occupied.contains(&next) {
            weight += 5;
        }
        if radius <= 5 {
            weight += 2;
        }
        if radius >= 7 {
            weight = weight.saturating_sub(2);
        }
        weighted.push((next, weight));
    }

    if weighted.is_empty() {
        return cell + heading;
    }

    rng.weighted_choice(&weighted)
}

fn choose_module_template(
    rng: &mut RunRng,
    current_socket: SocketMask,
    difficulty: f32,
    projected_gap: f32,
) -> ModuleTemplate {
    let mut weighted = Vec::new();

    for template in all_templates() {
        if template.entry & current_socket == 0 {
            continue;
        }
        if difficulty < template.min_difficulty || difficulty > template.max_difficulty {
            continue;
        }
        if projected_gap < template.min_gap - 1.5 || projected_gap > template.max_gap + 1.5 {
            continue;
        }

        let mut weight = template.weight;
        if difficulty > 0.55
            && matches!(
                template.kind,
                ModuleKind::CrumbleBridge
                    | ModuleKind::SweepHall
                    | ModuleKind::WindTunnel
                    | ModuleKind::PistonGate
                    | ModuleKind::TimedDoor
            )
        {
            weight += 3;
        }
        if difficulty < 0.35
            && matches!(
                template.kind,
                ModuleKind::StairRun | ModuleKind::MantleStack | ModuleKind::WaterGarden
            )
        {
            weight += 4;
        }
        weighted.push((template, weight));
    }

    if weighted.is_empty() {
        return module_template(safe_fallback_kind(difficulty));
    }

    rng.weighted_choice(&weighted)
}

fn all_templates() -> [ModuleTemplate; 11] {
    [
        module_template(ModuleKind::StairRun),
        module_template(ModuleKind::MantleStack),
        module_template(ModuleKind::WallRunHall),
        module_template(ModuleKind::LiftChasm),
        module_template(ModuleKind::CrumbleBridge),
        module_template(ModuleKind::SweepHall),
        module_template(ModuleKind::PistonGate),
        module_template(ModuleKind::WindTunnel),
        module_template(ModuleKind::IceSpine),
        module_template(ModuleKind::WaterGarden),
        module_template(ModuleKind::TimedDoor),
    ]
}

fn module_template(kind: ModuleKind) -> ModuleTemplate {
    match kind {
        ModuleKind::StairRun => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_SHORTCUT_ANCHOR,
            exit: SOCKET_SAFE_REST | SOCKET_MANTLE_ENTRY,
            min_difficulty: 0.0,
            max_difficulty: 0.45,
            weight: 7,
            min_rise: 3.6,
            max_rise: 5.5,
            min_gap: 7.0,
            max_gap: 10.5,
            shortcut_eligible: false,
        },
        ModuleKind::MantleStack => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_MANTLE_ENTRY,
            exit: SOCKET_SAFE_REST | SOCKET_WALLRUN_READY,
            min_difficulty: 0.15,
            max_difficulty: 0.72,
            weight: 5,
            min_rise: 4.2,
            max_rise: 6.8,
            min_gap: 7.0,
            max_gap: 10.5,
            shortcut_eligible: false,
        },
        ModuleKind::WallRunHall => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_WALLRUN_READY,
            exit: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            min_difficulty: 0.25,
            max_difficulty: 1.0,
            weight: 5,
            min_rise: 4.4,
            max_rise: 6.8,
            min_gap: 8.2,
            max_gap: 11.5,
            shortcut_eligible: true,
        },
        ModuleKind::LiftChasm => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH | SOCKET_SHORTCUT_ANCHOR,
            exit: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            min_difficulty: 0.22,
            max_difficulty: 1.0,
            weight: 4,
            min_rise: 4.8,
            max_rise: 7.8,
            min_gap: 8.0,
            max_gap: 11.8,
            shortcut_eligible: true,
        },
        ModuleKind::CrumbleBridge => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            exit: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH | SOCKET_SHORTCUT_ANCHOR,
            min_difficulty: 0.34,
            max_difficulty: 1.0,
            weight: 4,
            min_rise: 4.6,
            max_rise: 7.0,
            min_gap: 8.0,
            max_gap: 11.6,
            shortcut_eligible: true,
        },
        ModuleKind::SweepHall => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            exit: SOCKET_SAFE_REST | SOCKET_SHORTCUT_ANCHOR,
            min_difficulty: 0.42,
            max_difficulty: 1.0,
            weight: 4,
            min_rise: 4.8,
            max_rise: 7.2,
            min_gap: 7.8,
            max_gap: 11.2,
            shortcut_eligible: true,
        },
        ModuleKind::PistonGate => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            exit: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            min_difficulty: 0.48,
            max_difficulty: 1.0,
            weight: 3,
            min_rise: 5.2,
            max_rise: 8.2,
            min_gap: 8.0,
            max_gap: 11.6,
            shortcut_eligible: true,
        },
        ModuleKind::WindTunnel => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_WALLRUN_READY | SOCKET_HAZARD_BRANCH,
            exit: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH | SOCKET_SHORTCUT_ANCHOR,
            min_difficulty: 0.55,
            max_difficulty: 1.0,
            weight: 3,
            min_rise: 5.4,
            max_rise: 8.4,
            min_gap: 8.4,
            max_gap: 12.0,
            shortcut_eligible: true,
        },
        ModuleKind::IceSpine => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            exit: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            min_difficulty: 0.44,
            max_difficulty: 1.0,
            weight: 4,
            min_rise: 4.8,
            max_rise: 7.4,
            min_gap: 8.0,
            max_gap: 11.4,
            shortcut_eligible: false,
        },
        ModuleKind::WaterGarden => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_MANTLE_ENTRY,
            exit: SOCKET_SAFE_REST,
            min_difficulty: 0.18,
            max_difficulty: 0.68,
            weight: 3,
            min_rise: 3.8,
            max_rise: 5.8,
            min_gap: 7.0,
            max_gap: 10.5,
            shortcut_eligible: false,
        },
        ModuleKind::TimedDoor => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            exit: SOCKET_SAFE_REST | SOCKET_SHORTCUT_ANCHOR,
            min_difficulty: 0.4,
            max_difficulty: 1.0,
            weight: 3,
            min_rise: 4.8,
            max_rise: 7.6,
            min_gap: 7.8,
            max_gap: 11.2,
            shortcut_eligible: true,
        },
    }
}

fn safe_fallback_kind(difficulty: f32) -> ModuleKind {
    if difficulty > 0.55 {
        ModuleKind::LiftChasm
    } else if difficulty > 0.25 {
        ModuleKind::MantleStack
    } else {
        ModuleKind::StairRun
    }
}

fn spawn_run_world(
    blueprint: &RunBlueprint,
    collected_treasures: &HashSet<u64>,
    unlocked_shortcuts: &HashSet<u64>,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> RunSnapshot {
    spawn_box_spec(
        &SolidSpec {
            owner: OwnerTag::Summit,
            label: "Abyss Floor".into(),
            center: Vec3::new(0.0, -2.0, 0.0),
            size: Vec3::new(260.0, 2.0, 260.0),
            paint: PaintStyle::ThemeShadow(Theme::Stone),
            body: SolidBody::Static,
            friction: None,
            extra: ExtraKind::None,
        },
        collected_treasures,
        unlocked_shortcuts,
        commands,
        meshes,
        materials,
    );

    let mut checkpoints = Vec::new();
    let mut total_treasures = 0;

    for room in &blueprint.rooms {
        let layout = build_room_layout(room);
        total_treasures += spawn_layout(
            &layout,
            collected_treasures,
            unlocked_shortcuts,
            commands,
            meshes,
            materials,
        );
        if room.checkpoint_slot.is_some() {
            checkpoints.push(room.top + Vec3::new(0.0, PLAYER_SPAWN_CLEARANCE, 0.0));
        }
    }

    for segment in &blueprint.segments {
        let layout = build_segment_layout(segment, &blueprint.rooms, unlocked_shortcuts);
        total_treasures += spawn_layout(
            &layout,
            collected_treasures,
            unlocked_shortcuts,
            commands,
            meshes,
            materials,
        );
    }

    for (index, branch) in blueprint.branches.iter().enumerate() {
        let layout = build_branch_layout(index, branch, &blueprint.rooms, unlocked_shortcuts);
        total_treasures += spawn_layout(
            &layout,
            collected_treasures,
            unlocked_shortcuts,
            commands,
            meshes,
            materials,
        );
    }

    let summit_layout = build_summit_layout(blueprint.rooms.last().unwrap(), blueprint.summit);
    total_treasures += spawn_layout(
        &summit_layout,
        collected_treasures,
        unlocked_shortcuts,
        commands,
        meshes,
        materials,
    );

    commands.spawn((
        GeneratedWorld,
        Name::new("Sun"),
        Transform::from_xyz(
            blueprint.summit.x - 34.0,
            blueprint.summit.y + 42.0,
            blueprint.summit.z + 28.0,
        )
        .looking_at(Vec3::new(0.0, blueprint.summit.y * 0.5, 0.0), Vec3::Y),
        DirectionalLight {
            shadows_enabled: true,
            illuminance: 28_000.0,
            ..default()
        },
    ));

    RunSnapshot {
        checkpoints,
        total_treasures,
    }
}

fn spawn_layout(
    layout: &ModuleLayout,
    collected_treasures: &HashSet<u64>,
    unlocked_shortcuts: &HashSet<u64>,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> usize {
    let mut treasure_count = 0;
    for solid in &layout.solids {
        if matches!(solid.extra, ExtraKind::Treasure { .. }) {
            treasure_count += 1;
        }
        spawn_box_spec(
            solid,
            collected_treasures,
            unlocked_shortcuts,
            commands,
            meshes,
            materials,
        );
    }

    for feature in &layout.features {
        match feature {
            FeatureSpec::WindZone {
                center,
                size,
                direction,
                strength,
                gust,
            } => {
                commands.spawn((
                    GeneratedWorld,
                    Name::new("Wind Zone"),
                    Mesh3d(meshes.add(Cuboid::new(size.x, size.y, size.z))),
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color: Color::srgba(0.4, 0.75, 1.0, 0.14),
                        alpha_mode: AlphaMode::Blend,
                        unlit: true,
                        ..default()
                    })),
                    Transform::from_translation(*center),
                    WindZone {
                        size: *size,
                        direction: *direction,
                        strength: *strength,
                        gust: *gust,
                    },
                ));
            }
            FeatureSpec::PointLight {
                center,
                intensity,
                range,
                color,
            } => {
                commands.spawn((
                    GeneratedWorld,
                    Name::new("Beacon Light"),
                    PointLight {
                        intensity: *intensity,
                        range: *range,
                        color: *color,
                        shadows_enabled: true,
                        ..default()
                    },
                    Transform::from_translation(*center),
                ));
            }
        }
    }

    treasure_count
}

fn spawn_box_spec(
    spec: &SolidSpec,
    collected_treasures: &HashSet<u64>,
    unlocked_shortcuts: &HashSet<u64>,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    if let ExtraKind::Treasure { id } = spec.extra {
        if collected_treasures.contains(&id) {
            return;
        }
    }

    let mesh = meshes.add(Cuboid::new(spec.size.x, spec.size.y, spec.size.z));
    let material = materials.add(material_for_paint(
        spec.paint,
        matches!(spec.body, SolidBody::ShortcutBridge { active: false, .. }),
    ));

    let mut entity = commands.spawn((
        GeneratedWorld,
        Name::new(spec.label.clone()),
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::from_translation(spec.center),
    ));

    match &spec.body {
        SolidBody::Static => {
            entity.insert((
                RigidBody::Static,
                Collider::cuboid(spec.size.x, spec.size.y, spec.size.z),
                CollisionLayers::new(CollisionLayer::Default, LayerMask::ALL),
            ));
        }
        SolidBody::Decoration => {}
        SolidBody::DynamicProp => {
            entity.insert((
                RigidBody::Dynamic,
                Collider::cuboid(spec.size.x, spec.size.y, spec.size.z),
                CollisionLayers::new(CollisionLayer::Prop, LayerMask::ALL),
                Mass(165.0),
                LinearDamping(1.8),
                AngularDamping(2.4),
                Friction::new(1.25),
            ));
        }
        SolidBody::Water { speed } => {
            entity.insert((
                RigidBody::Static,
                Sensor,
                Collider::cuboid(spec.size.x, spec.size.y, spec.size.z),
                CollisionLayers::new(CollisionLayer::Default, LayerMask::ALL),
                Water { speed: *speed },
            ));
        }
        SolidBody::Moving { end, speed, lethal } => {
            entity.insert((
                RigidBody::Kinematic,
                TransformInterpolation,
                Collider::cuboid(spec.size.x, spec.size.y, spec.size.z),
                CollisionLayers::new(CollisionLayer::Default, LayerMask::ALL),
                Mover {
                    start: spec.center,
                    end: *end,
                    speed: *speed,
                    direction: 1.0,
                },
            ));
            if *lethal {
                entity.insert(LethalHazard);
            }
        }
        SolidBody::Crumbling { delay, sink_speed } => {
            entity.insert((
                RigidBody::Static,
                Collider::cuboid(spec.size.x, spec.size.y, spec.size.z),
                CollisionLayers::new(CollisionLayer::Default, LayerMask::ALL),
                CrumblingPlatform {
                    timer: Timer::from_seconds(*delay, TimerMode::Once),
                    sink_speed: *sink_speed,
                    armed: false,
                    collapsed: false,
                },
            ));
        }
        SolidBody::ShortcutBridge { id, active } => {
            let enabled = *active || unlocked_shortcuts.contains(id);
            entity.insert((
                RigidBody::Static,
                ShortcutBridge {
                    id: *id,
                    size: spec.size,
                },
            ));
            if enabled {
                entity.insert((
                    Collider::cuboid(spec.size.x, spec.size.y, spec.size.z),
                    CollisionLayers::new(CollisionLayer::Default, LayerMask::ALL),
                ));
            }
        }
    }

    if let Some(friction) = spec.friction {
        entity.insert(Friction::new(friction));
    }

    match spec.extra {
        ExtraKind::None => {}
        ExtraKind::Checkpoint { index } => {
            entity.insert(CheckpointPad { index });
        }
        ExtraKind::SummitGoal => {
            entity.insert(SummitGoal);
        }
        ExtraKind::Treasure { id } => {
            entity.insert(TreasurePickup { id });
        }
        ExtraKind::ShortcutTrigger { id } => {
            entity.insert(ShortcutTrigger { id });
        }
    }
}

fn build_room_layout(room: &RoomPlan) -> ModuleLayout {
    let mut layout = ModuleLayout::default();
    let owner = OwnerTag::Room(room.index);
    let size = Vec3::new(room.size.x, ROOM_HEIGHT, room.size.y);

    layout.solids.push(SolidSpec {
        owner,
        label: format!("Room {}", room.index),
        center: top_to_center(room.top, ROOM_HEIGHT),
        size,
        paint: PaintStyle::ThemeFloor(room.theme),
        body: SolidBody::Static,
        friction: None,
        extra: ExtraKind::None,
    });

    let support_top = room.top.y - ROOM_HEIGHT;
    if support_top > -0.2 {
        layout.solids.push(SolidSpec {
            owner,
            label: format!("Room {} Support", room.index),
            center: Vec3::new(room.top.x, (support_top - 1.0) * 0.5, room.top.z),
            size: Vec3::new(2.3, support_top + 1.0, 2.3),
            paint: PaintStyle::ThemeShadow(room.theme),
            body: SolidBody::Static,
            friction: None,
            extra: ExtraKind::None,
        });
    }

    if let Some(index) = room.checkpoint_slot {
        layout.solids.push(SolidSpec {
            owner,
            label: format!("Checkpoint {}", room.index),
            center: top_to_center(room.top + Vec3::Y * 0.22, 0.22),
            size: Vec3::new(3.4, 0.22, 3.4),
            paint: PaintStyle::Checkpoint,
            body: SolidBody::Decoration,
            friction: None,
            extra: ExtraKind::Checkpoint { index },
        });
    }

    layout.clearances.push(ClearanceProbe {
        owner,
        center: room.top + Vec3::Y * (ROOM_CLEARANCE_HEIGHT * 0.5),
        size: Vec3::new(
            (room.size.x - 1.0).max(3.0),
            ROOM_CLEARANCE_HEIGHT,
            (room.size.y - 1.0).max(3.0),
        ),
    });

    layout
}

fn build_segment_layout(
    segment: &SegmentPlan,
    rooms: &[RoomPlan],
    unlocked_shortcuts: &HashSet<u64>,
) -> ModuleLayout {
    let mut layout = ModuleLayout::default();
    let owner = OwnerTag::Segment(segment.index);
    let from = &rooms[segment.from];
    let to = &rooms[segment.to];
    let template = module_template(segment.kind);
    let mut rng = RunRng::new(segment.seed);
    let forward = direction_from_delta(to.top - from.top);
    let right = Vec3::new(-forward.z, 0.0, forward.x);
    let along_x = forward.x.abs() > 0.5;
    let start = room_edge(from, forward);
    let end = room_edge(to, -forward);
    let rise = to.top.y - from.top.y;

    match segment.kind {
        ModuleKind::StairRun => {
            let steps = 5 + usize::from(segment.difficulty > 0.18);
            for step in 0..steps {
                let t = (step + 1) as f32 / (steps + 1) as f32;
                let mut top = start.lerp(end, t);
                top.y = from.top.y + rise * t - 0.12;
                layout.solids.push(SolidSpec {
                    owner,
                    label: "Stair Step".into(),
                    center: top_to_center(top, 0.78 + t * 0.2),
                    size: axis_box_size(along_x, 3.4, 0.78 + t * 0.2, 3.5),
                    paint: PaintStyle::ThemeAccent(to.theme),
                    body: SolidBody::Static,
                    friction: None,
                    extra: ExtraKind::None,
                });
            }
        }
        ModuleKind::MantleStack => {
            let ledges = [(0.28, 1.6_f32), (0.54, 3.3), (0.82, 5.0)];
            for (index, (t, local_rise)) in ledges.into_iter().enumerate() {
                let mut top = start.lerp(end, t);
                top.y = (from.top.y + local_rise).min(to.top.y - 0.35);
                top += right * ((index as f32 - 1.0) * 1.1);
                layout.solids.push(SolidSpec {
                    owner,
                    label: "Mantle Ledge".into(),
                    center: top_to_center(top, 1.0 + index as f32 * 0.28),
                    size: axis_box_size(along_x, 3.3, 1.0 + index as f32 * 0.28, 3.6),
                    paint: PaintStyle::ThemeAccent(to.theme),
                    body: SolidBody::Static,
                    friction: None,
                    extra: ExtraKind::None,
                });
            }

            let wall_height = rise + 4.8;
            let wall_mid = start.lerp(end, 0.68) + right * 2.4;
            layout.solids.push(SolidSpec {
                owner,
                label: "Mantle Wall".into(),
                center: Vec3::new(
                    wall_mid.x,
                    underside_y(from.top.y, ROOM_HEIGHT) + wall_height * 0.5,
                    wall_mid.z,
                ),
                size: axis_box_size(along_x, 5.4, wall_height, 1.0),
                paint: PaintStyle::ThemeShadow(to.theme),
                body: SolidBody::Static,
                friction: None,
                extra: ExtraKind::None,
            });
        }
        ModuleKind::WallRunHall => {
            let wall_side = if rng.chance(0.5) { 1.0 } else { -1.0 };
            let wall_height = rise + 7.6;
            let wall_mid = start.lerp(end, 0.52) + right * wall_side * 3.9;
            layout.solids.push(SolidSpec {
                owner,
                label: "Wall Run Wall".into(),
                center: Vec3::new(
                    wall_mid.x,
                    underside_y(from.top.y, ROOM_HEIGHT) + wall_height * 0.5,
                    wall_mid.z,
                ),
                size: axis_box_size(
                    along_x,
                    (end - start).xz().length() + 5.0,
                    wall_height,
                    1.12,
                ),
                paint: PaintStyle::ThemeShadow(to.theme),
                body: SolidBody::Static,
                friction: None,
                extra: ExtraKind::None,
            });
            layout.solids.push(SolidSpec {
                owner,
                label: "Kick Pad".into(),
                center: top_to_center(start + right * wall_side * 1.1 + Vec3::Y * 0.58, 0.75),
                size: axis_box_size(along_x, 2.7, 0.75, 2.7),
                paint: PaintStyle::ThemeAccent(to.theme),
                body: SolidBody::Static,
                friction: None,
                extra: ExtraKind::None,
            });
            layout.solids.push(SolidSpec {
                owner,
                label: "Wall Run Rest".into(),
                center: top_to_center(
                    start.lerp(end, 0.6) - right * wall_side * 2.5 + Vec3::Y * 1.0,
                    0.82,
                ),
                size: axis_box_size(along_x, 3.0, 0.82, 3.2),
                paint: PaintStyle::ThemeFloor(to.theme),
                body: SolidBody::Static,
                friction: None,
                extra: ExtraKind::None,
            });
        }
        ModuleKind::LiftChasm => {
            let mover_size = Vec3::new(3.2, 0.72, 3.2);
            let start_top = start.lerp(end, 0.24) + Vec3::Y * (rise * 0.25 + 0.9);
            let end_top = start.lerp(end, 0.74) + Vec3::Y * (rise * 0.72 + 1.1);
            layout.solids.push(SolidSpec {
                owner,
                label: "Sky Lift".into(),
                center: top_to_center(start_top, mover_size.y),
                size: mover_size,
                paint: PaintStyle::ThemeAccent(to.theme),
                body: SolidBody::Moving {
                    end: top_to_center(end_top, mover_size.y),
                    speed: lerp(2.6, 4.4, segment.difficulty),
                    lethal: false,
                },
                friction: None,
                extra: ExtraKind::None,
            });
            for anchor in [start_top, end_top] {
                let support_top = anchor.y - mover_size.y;
                layout.solids.push(SolidSpec {
                    owner,
                    label: "Lift Support".into(),
                    center: Vec3::new(
                        anchor.x - right.x * 3.2,
                        (support_top - 1.0) * 0.5,
                        anchor.z - right.z * 3.2,
                    ),
                    size: Vec3::new(1.4, support_top + 1.0, 1.4),
                    paint: PaintStyle::ThemeShadow(to.theme),
                    body: SolidBody::Static,
                    friction: None,
                    extra: ExtraKind::None,
                });
            }
        }
        ModuleKind::CrumbleBridge => {
            let pieces = 5;
            for step in 0..pieces {
                let t = (step + 1) as f32 / (pieces + 1) as f32;
                let mut top = start.lerp(end, t);
                top.y = from.top.y + rise * t - 0.12;
                layout.solids.push(SolidSpec {
                    owner,
                    label: "Crumble Span".into(),
                    center: top_to_center(top, 0.55),
                    size: axis_box_size(along_x, 2.1, 0.55, 2.2),
                    paint: PaintStyle::Hazard,
                    body: SolidBody::Crumbling {
                        delay: lerp(0.9, 0.45, segment.difficulty),
                        sink_speed: lerp(2.8, 5.0, segment.difficulty),
                    },
                    friction: None,
                    extra: ExtraKind::None,
                });
            }
        }
        ModuleKind::SweepHall => {
            for perch in [0.22, 0.5, 0.78] {
                let mut top = start.lerp(end, perch);
                top.y = from.top.y + rise * perch - 0.15;
                layout.solids.push(SolidSpec {
                    owner,
                    label: "Sweep Perch".into(),
                    center: top_to_center(top, 0.72),
                    size: axis_box_size(along_x, 2.6, 0.72, 2.8),
                    paint: PaintStyle::ThemeFloor(to.theme),
                    body: SolidBody::Static,
                    friction: None,
                    extra: ExtraKind::None,
                });
            }
            let beam_start = start.lerp(end, 0.5) + right * 3.0 + Vec3::Y * (rise * 0.5 + 1.1);
            let beam_end = beam_start - right * 6.0;
            layout.solids.push(SolidSpec {
                owner,
                label: "Sweep Beam".into(),
                center: beam_start,
                size: axis_box_size(along_x, 1.1, 1.1, 6.2),
                paint: PaintStyle::Hazard,
                body: SolidBody::Moving {
                    end: beam_end,
                    speed: lerp(2.2, 4.4, segment.difficulty),
                    lethal: true,
                },
                friction: None,
                extra: ExtraKind::None,
            });
        }
        ModuleKind::PistonGate => {
            for perch in [0.2, 0.45, 0.72] {
                let mut top = start.lerp(end, perch);
                top.y = from.top.y + rise * perch - 0.15;
                layout.solids.push(SolidSpec {
                    owner,
                    label: "Piston Perch".into(),
                    center: top_to_center(top, 0.7),
                    size: axis_box_size(along_x, 2.4, 0.7, 2.8),
                    paint: PaintStyle::ThemeFloor(to.theme),
                    body: SolidBody::Static,
                    friction: None,
                    extra: ExtraKind::None,
                });
            }
            for side in [-1.0, 1.0] {
                let center =
                    start.lerp(end, 0.55) + right * side * 3.6 + Vec3::Y * (rise * 0.55 + 1.3);
                layout.solids.push(SolidSpec {
                    owner,
                    label: "Piston Wall".into(),
                    center,
                    size: axis_box_size(along_x, 1.3, 2.8, 2.2),
                    paint: PaintStyle::Hazard,
                    body: SolidBody::Moving {
                        end: center - right * side * 3.2,
                        speed: lerp(2.0, 3.8, segment.difficulty),
                        lethal: true,
                    },
                    friction: None,
                    extra: ExtraKind::None,
                });
            }
        }
        ModuleKind::WindTunnel => {
            for perch in [0.18, 0.5, 0.84] {
                let mut top = start.lerp(end, perch);
                top.y = from.top.y + rise * perch - 0.18;
                layout.solids.push(SolidSpec {
                    owner,
                    label: "Wind Perch".into(),
                    center: top_to_center(top, 0.58),
                    size: axis_box_size(along_x, 2.3, 0.58, 1.8),
                    paint: PaintStyle::ThemeFloor(to.theme),
                    body: SolidBody::Static,
                    friction: None,
                    extra: ExtraKind::None,
                });
            }
            layout.features.push(FeatureSpec::WindZone {
                center: start.lerp(end, 0.52) + Vec3::Y * (rise * 0.55 + 1.5),
                size: axis_box_size(along_x, (end - start).xz().length() + 2.0, 3.6, 7.0),
                direction: right * if rng.chance(0.5) { 1.0 } else { -1.0 } + Vec3::Y * 0.1,
                strength: lerp(6.0, 11.0, segment.difficulty),
                gust: lerp(1.2, 2.8, segment.difficulty),
            });
        }
        ModuleKind::IceSpine => {
            for span in [0.2, 0.5, 0.8] {
                let mut top = start.lerp(end, span);
                top.y = from.top.y + rise * span - 0.15;
                layout.solids.push(SolidSpec {
                    owner,
                    label: "Ice Spine".into(),
                    center: top_to_center(top, 0.58),
                    size: axis_box_size(along_x, 3.0, 0.58, 1.7),
                    paint: PaintStyle::Ice,
                    body: SolidBody::Static,
                    friction: Some(0.02),
                    extra: ExtraKind::None,
                });
            }
        }
        ModuleKind::WaterGarden => {
            let basin_top = from.top.y.min(to.top.y) - 2.3;
            let basin_mid = start.lerp(end, 0.5);
            let basin_size = axis_box_size(along_x, (end - start).xz().length() + 6.5, 0.78, 9.2);
            let water_size = Vec3::new(
                (basin_size.x - 0.7).max(2.5),
                2.1,
                (basin_size.z - 0.7).max(2.5),
            );
            layout.solids.push(SolidSpec {
                owner,
                label: "Water Basin".into(),
                center: top_to_center(Vec3::new(basin_mid.x, basin_top, basin_mid.z), basin_size.y),
                size: basin_size,
                paint: PaintStyle::ThemeShadow(to.theme),
                body: SolidBody::Static,
                friction: None,
                extra: ExtraKind::None,
            });
            layout.solids.push(SolidSpec {
                owner,
                label: "Water".into(),
                center: Vec3::new(basin_mid.x, basin_top + water_size.y * 0.5, basin_mid.z),
                size: water_size,
                paint: PaintStyle::Water,
                body: SolidBody::Water { speed: 0.72 },
                friction: None,
                extra: ExtraKind::None,
            });
            for pillar in [0.2, 0.5, 0.8] {
                let mut top = start.lerp(end, pillar);
                top.y = from.top.y + rise * pillar - 0.85;
                layout.solids.push(SolidSpec {
                    owner,
                    label: "Water Pillar".into(),
                    center: Vec3::new(top.x, (top.y + basin_top) * 0.5, top.z),
                    size: Vec3::new(2.4, top.y - basin_top, 2.4),
                    paint: PaintStyle::ThemeFloor(to.theme),
                    body: SolidBody::Static,
                    friction: None,
                    extra: ExtraKind::None,
                });
            }
        }
        ModuleKind::TimedDoor => {
            for perch in [0.2, 0.55, 0.84] {
                let mut top = start.lerp(end, perch);
                top.y = from.top.y + rise * perch - 0.15;
                layout.solids.push(SolidSpec {
                    owner,
                    label: "Door Perch".into(),
                    center: top_to_center(top, 0.7),
                    size: axis_box_size(along_x, 2.6, 0.7, 2.9),
                    paint: PaintStyle::ThemeFloor(to.theme),
                    body: SolidBody::Static,
                    friction: None,
                    extra: ExtraKind::None,
                });
            }
            for offset in [0.34, 0.68] {
                let center = start.lerp(end, offset) + Vec3::Y * (rise * offset + 1.4);
                layout.solids.push(SolidSpec {
                    owner,
                    label: "Timed Door".into(),
                    center,
                    size: axis_box_size(along_x, 1.2, 3.0, 4.2),
                    paint: PaintStyle::Hazard,
                    body: SolidBody::Moving {
                        end: center + Vec3::Y * 3.5,
                        speed: lerp(1.6, 3.1, segment.difficulty),
                        lethal: true,
                    },
                    friction: None,
                    extra: ExtraKind::None,
                });
            }
        }
    }

    if let Some(id) = segment.shortcut_id {
        let mid = start.lerp(end, 0.5);
        let bridge_top = Vec3::new(mid.x, from.top.y + rise * 0.45 + 0.55, mid.z);
        layout.solids.push(SolidSpec {
            owner,
            label: "Shortcut Bridge".into(),
            center: top_to_center(bridge_top, 0.5),
            size: axis_box_size(along_x, (end - start).xz().length() - 1.6, 0.5, 2.6),
            paint: PaintStyle::Shortcut,
            body: SolidBody::ShortcutBridge {
                id,
                active: unlocked_shortcuts.contains(&id),
            },
            friction: None,
            extra: ExtraKind::None,
        });
    }

    layout.clearances.push(ClearanceProbe {
        owner,
        center: start.lerp(end, 0.5) + Vec3::Y * (rise * 0.5 + 2.5),
        size: axis_box_size(along_x, (end - start).xz().length() * 0.6, 2.8, 3.4),
    });
    let _ = template;
    layout
}

fn build_branch_layout(
    index: usize,
    branch: &BranchPlan,
    rooms: &[RoomPlan],
    _unlocked_shortcuts: &HashSet<u64>,
) -> ModuleLayout {
    let mut layout = ModuleLayout::default();
    let owner = OwnerTag::Branch(index);
    let room = &rooms[branch.room_index];
    let branch_dir = Vec3::new(branch.dir.x as f32, 0.0, branch.dir.y as f32).normalize_or_zero();
    let along_x = branch_dir.x.abs() > 0.5;
    let start = room_edge(room, branch_dir);
    let bridge_top = room.top + branch_dir * (CELL_SIZE * 0.34) + Vec3::Y * 0.22;
    let platform_top = branch.top;

    let bridge_body = if matches!(branch.kind, BranchKind::RiskDetour) {
        SolidBody::Crumbling {
            delay: 0.7,
            sink_speed: 3.8,
        }
    } else {
        SolidBody::Static
    };
    let bridge_paint = if matches!(branch.kind, BranchKind::RiskDetour) {
        PaintStyle::Hazard
    } else {
        PaintStyle::ThemeAccent(branch.theme)
    };
    layout.solids.push(SolidSpec {
        owner,
        label: "Branch Bridge".into(),
        center: top_to_center(bridge_top, 0.6),
        size: axis_box_size(along_x, CELL_SIZE * 0.58, 0.6, 2.2),
        paint: bridge_paint,
        body: bridge_body,
        friction: if matches!(branch.kind, BranchKind::RiskDetour) {
            Some(0.04)
        } else {
            None
        },
        extra: ExtraKind::None,
    });

    layout.solids.push(SolidSpec {
        owner,
        label: "Branch Platform".into(),
        center: top_to_center(platform_top, 0.82),
        size: Vec3::new(branch.size.x, 0.82, branch.size.y),
        paint: PaintStyle::ThemeFloor(branch.theme),
        body: SolidBody::Static,
        friction: if matches!(branch.kind, BranchKind::RiskDetour) {
            Some(0.05)
        } else {
            None
        },
        extra: ExtraKind::None,
    });

    let support_top = platform_top.y - 0.82;
    if support_top > -0.2 {
        layout.solids.push(SolidSpec {
            owner,
            label: "Branch Support".into(),
            center: Vec3::new(platform_top.x, (support_top - 1.0) * 0.5, platform_top.z),
            size: Vec3::new(1.7, support_top + 1.0, 1.7),
            paint: PaintStyle::ThemeShadow(branch.theme),
            body: SolidBody::Static,
            friction: None,
            extra: ExtraKind::None,
        });
    }

    match branch.kind {
        BranchKind::TreasureAlcove => {
            if let Some(id) = branch.treasure_id {
                layout.solids.push(SolidSpec {
                    owner,
                    label: "Treasure".into(),
                    center: top_to_center(platform_top + Vec3::Y * 0.68, 0.85),
                    size: Vec3::splat(0.85),
                    paint: PaintStyle::Treasure,
                    body: SolidBody::Decoration,
                    friction: None,
                    extra: ExtraKind::Treasure { id },
                });
            }
        }
        BranchKind::PropCache => {
            let mut rng = RunRng::new(branch.seed);
            for prop_index in 0..2 {
                let offset = Vec3::new(
                    rng.range_f32(-branch.size.x * 0.18, branch.size.x * 0.18),
                    0.08,
                    rng.range_f32(-branch.size.y * 0.18, branch.size.y * 0.18),
                );
                layout.solids.push(SolidSpec {
                    owner,
                    label: format!("Cache Crate {}", prop_index),
                    center: top_to_center(platform_top + offset + Vec3::Y * 0.75, 1.35),
                    size: Vec3::splat(1.35),
                    paint: PaintStyle::Prop(branch.theme),
                    body: SolidBody::DynamicProp,
                    friction: None,
                    extra: ExtraKind::None,
                });
            }
        }
        BranchKind::ShortcutLever => {
            if let Some(id) = branch.shortcut_id {
                layout.solids.push(SolidSpec {
                    owner,
                    label: "Shortcut Switch".into(),
                    center: top_to_center(platform_top + Vec3::Y * 0.6, 1.0),
                    size: Vec3::new(1.0, 1.0, 1.0),
                    paint: PaintStyle::Shortcut,
                    body: SolidBody::Decoration,
                    friction: None,
                    extra: ExtraKind::ShortcutTrigger { id },
                });
            }
        }
        BranchKind::RiskDetour => {
            if let Some(id) = branch.treasure_id {
                layout.solids.push(SolidSpec {
                    owner,
                    label: "Risk Treasure".into(),
                    center: top_to_center(platform_top + Vec3::Y * 0.75, 0.9),
                    size: Vec3::splat(0.9),
                    paint: PaintStyle::Treasure,
                    body: SolidBody::Decoration,
                    friction: None,
                    extra: ExtraKind::Treasure { id },
                });
            }
            layout.features.push(FeatureSpec::WindZone {
                center: start.lerp(platform_top, 0.5) + Vec3::Y * 1.5,
                size: axis_box_size(along_x, CELL_SIZE * 0.48, 2.8, 5.0),
                direction: Vec3::Y * 0.15 + Vec3::new(-branch_dir.z, 0.0, branch_dir.x),
                strength: 6.5,
                gust: 2.0,
            });
        }
    }

    layout.clearances.push(ClearanceProbe {
        owner,
        center: platform_top + Vec3::Y * (ROOM_CLEARANCE_HEIGHT * 0.45),
        size: Vec3::new(
            (branch.size.x - 0.8).max(2.6),
            ROOM_CLEARANCE_HEIGHT,
            (branch.size.y - 0.8).max(2.6),
        ),
    });

    layout
}

fn build_summit_layout(room: &RoomPlan, summit: Vec3) -> ModuleLayout {
    let mut layout = ModuleLayout::default();
    let owner = OwnerTag::Summit;

    layout.solids.push(SolidSpec {
        owner,
        label: "Summit Dais".into(),
        center: top_to_center(room.top + Vec3::Y * 0.8, ROOM_HEIGHT),
        size: Vec3::new(room.size.x + 3.0, ROOM_HEIGHT, room.size.y + 3.0),
        paint: PaintStyle::Summit,
        body: SolidBody::Static,
        friction: None,
        extra: ExtraKind::None,
    });
    layout.solids.push(SolidSpec {
        owner,
        label: "Summit Goal".into(),
        center: top_to_center(summit, 1.0),
        size: Vec3::new(3.8, 1.0, 3.8),
        paint: PaintStyle::Summit,
        body: SolidBody::Decoration,
        friction: None,
        extra: ExtraKind::SummitGoal,
    });
    layout.solids.push(SolidSpec {
        owner,
        label: "Beacon Column".into(),
        center: Vec3::new(room.top.x, room.top.y + 4.4, room.top.z),
        size: Vec3::new(1.35, 7.2, 1.35),
        paint: PaintStyle::Shortcut,
        body: SolidBody::Static,
        friction: None,
        extra: ExtraKind::None,
    });
    layout.features.push(FeatureSpec::PointLight {
        center: room.top + Vec3::Y * 9.4,
        intensity: 500_000.0,
        range: 140.0,
        color: tailwind::AMBER_200.into(),
    });
    layout
}

fn material_for_paint(paint: PaintStyle, ghost: bool) -> StandardMaterial {
    match paint {
        PaintStyle::ThemeFloor(theme) => StandardMaterial {
            base_color: theme_floor_color(theme),
            perceptual_roughness: 0.9,
            ..default()
        },
        PaintStyle::ThemeAccent(theme) => StandardMaterial {
            base_color: theme_accent_color(theme),
            perceptual_roughness: 0.65,
            ..default()
        },
        PaintStyle::ThemeShadow(theme) => StandardMaterial {
            base_color: theme_shadow_color(theme),
            perceptual_roughness: 0.98,
            ..default()
        },
        PaintStyle::Prop(theme) => StandardMaterial {
            base_color: theme_prop_color(theme),
            perceptual_roughness: 0.84,
            ..default()
        },
        PaintStyle::Summit => StandardMaterial {
            base_color: tailwind::YELLOW_200.into(),
            emissive: LinearRgba::from(Color::from(tailwind::AMBER_200)) * 0.06,
            perceptual_roughness: 0.32,
            ..default()
        },
        PaintStyle::Checkpoint => StandardMaterial {
            base_color: tailwind::EMERALD_300.into(),
            emissive: LinearRgba::from(Color::from(tailwind::EMERALD_200)) * 0.08,
            perceptual_roughness: 0.25,
            ..default()
        },
        PaintStyle::Treasure => StandardMaterial {
            base_color: tailwind::AMBER_300.into(),
            emissive: LinearRgba::from(Color::from(tailwind::YELLOW_100)) * 0.18,
            perceptual_roughness: 0.22,
            metallic: 0.15,
            ..default()
        },
        PaintStyle::Hazard => StandardMaterial {
            base_color: tailwind::ROSE_400.into(),
            emissive: LinearRgba::from(Color::from(tailwind::ROSE_200)) * 0.09,
            perceptual_roughness: 0.48,
            ..default()
        },
        PaintStyle::Shortcut => StandardMaterial {
            base_color: if ghost {
                Color::srgba(0.4, 0.86, 0.95, 0.28)
            } else {
                tailwind::CYAN_300.into()
            },
            alpha_mode: if ghost {
                AlphaMode::Blend
            } else {
                AlphaMode::Opaque
            },
            emissive: LinearRgba::from(Color::from(tailwind::CYAN_200)) * 0.06,
            perceptual_roughness: 0.28,
            ..default()
        },
        PaintStyle::Ice => StandardMaterial {
            base_color: tailwind::SKY_200.into(),
            alpha_mode: AlphaMode::Blend,
            reflectance: 0.56,
            perceptual_roughness: 0.12,
            ..default()
        },
        PaintStyle::Water => StandardMaterial {
            base_color: Color::srgba(0.15, 0.45, 0.95, 0.42),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.15,
            reflectance: 0.5,
            ..default()
        },
    }
}

fn theme_for(index: usize, total: usize, offset: usize) -> Theme {
    let bands = [Theme::Stone, Theme::Overgrown, Theme::Frost, Theme::Ember];
    let band = (index * bands.len() / total.max(1) + offset) % bands.len();
    bands[band]
}

fn theme_floor_color(theme: Theme) -> Color {
    match theme {
        Theme::Stone => tailwind::STONE_300.into(),
        Theme::Overgrown => tailwind::LIME_300.into(),
        Theme::Frost => tailwind::SKY_200.into(),
        Theme::Ember => tailwind::ORANGE_300.into(),
    }
}

fn theme_accent_color(theme: Theme) -> Color {
    match theme {
        Theme::Stone => tailwind::STONE_500.into(),
        Theme::Overgrown => tailwind::GREEN_400.into(),
        Theme::Frost => tailwind::CYAN_300.into(),
        Theme::Ember => tailwind::ROSE_300.into(),
    }
}

fn theme_shadow_color(theme: Theme) -> Color {
    match theme {
        Theme::Stone => tailwind::SLATE_700.into(),
        Theme::Overgrown => tailwind::GREEN_700.into(),
        Theme::Frost => tailwind::SKY_700.into(),
        Theme::Ember => tailwind::AMBER_800.into(),
    }
}

fn theme_prop_color(theme: Theme) -> Color {
    match theme {
        Theme::Stone => tailwind::STONE_400.into(),
        Theme::Overgrown => tailwind::LIME_400.into(),
        Theme::Frost => tailwind::SKY_300.into(),
        Theme::Ember => tailwind::ORANGE_400.into(),
    }
}

fn projected_gap(from_size: Vec2, to_size: Vec2) -> f32 {
    CELL_SIZE - (from_size.x + to_size.x) * 0.5
}

fn edge_gap(from: &RoomPlan, to: &RoomPlan) -> f32 {
    let dir = to.cell - from.cell;
    if dir.x != 0 {
        CELL_SIZE - (from.size.x + to.size.x) * 0.5
    } else {
        CELL_SIZE - (from.size.y + to.size.y) * 0.5
    }
}

fn room_edge(room: &RoomPlan, forward: Vec3) -> Vec3 {
    let extent = if forward.x.abs() > 0.5 {
        room.size.x
    } else {
        room.size.y
    };
    room.top + forward * (extent * 0.5 - 1.05)
}

fn underside_y(top: f32, thickness: f32) -> f32 {
    top - thickness
}

fn direction_from_delta(delta: Vec3) -> Vec3 {
    if delta.x.abs() >= delta.z.abs() {
        Vec3::new(delta.x.signum(), 0.0, 0.0)
    } else {
        Vec3::new(0.0, 0.0, delta.z.signum())
    }
}

fn axis_box_size(along_x: bool, length: f32, height: f32, width: f32) -> Vec3 {
    if along_x {
        Vec3::new(length, height, width)
    } else {
        Vec3::new(width, height, length)
    }
}

fn top_to_center(top: Vec3, height: f32) -> Vec3 {
    Vec3::new(top.x, top.y - height * 0.5, top.z)
}

fn intersects(a: AabbVolume, b: AabbVolume, epsilon: f32) -> bool {
    let delta = (a.center - b.center).abs();
    let limit = (a.size + b.size) * 0.5 - Vec3::splat(epsilon);
    delta.x < limit.x && delta.y < limit.y && delta.z < limit.z
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn current_run_seed() -> u64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seed = elapsed.as_secs() ^ ((elapsed.subsec_nanos() as u64) << 32);
    seed ^ seed.rotate_left(17) ^ 0xA5A5_5A5A_DEAD_BEEF
}

struct RunRng {
    state: u64,
}

impl RunRng {
    fn new(seed: u64) -> Self {
        Self {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn next_f32(&mut self) -> f32 {
        let sample = (self.next_u64() >> 40) as u32;
        sample as f32 / 16_777_215.0
    }

    fn range_f32(&mut self, min: f32, max: f32) -> f32 {
        min + (max - min) * self.next_f32()
    }

    fn range_usize(&mut self, min: usize, max_exclusive: usize) -> usize {
        min + (self.next_u64() as usize % (max_exclusive - min))
    }

    fn chance(&mut self, probability: f32) -> bool {
        self.next_f32() < probability
    }

    fn weighted_choice<T: Copy>(&mut self, weighted: &[(T, u32)]) -> T {
        let total: u32 = weighted.iter().map(|(_, weight)| *weight).sum();
        let mut roll = (self.next_u64() % total as u64) as u32;
        for (item, weight) in weighted {
            if roll < *weight {
                return *item;
            }
            roll -= *weight;
        }
        weighted.last().unwrap().0
    }
}

#[derive(Debug, PhysicsLayer, Default, Clone, Copy)]
enum CollisionLayer {
    #[default]
    Default,
    Player,
    Prop,
}
