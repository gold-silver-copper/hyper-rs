use std::{
    collections::HashMap,
    f32::consts::{PI, TAU},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use avian3d::prelude::*;
use bevy::{
    asset::RenderAssetUsages,
    input::common_conditions::input_just_pressed,
    math::primitives::Cuboid,
    mesh::Indices,
    prelude::*,
    render::render_resource::PrimitiveTopology,
    window::{CursorGrabMode, CursorOptions, WindowResolution},
};
use bevy_ahoy::{CharacterControllerOutput, CharacterLook, input::AccumulatedInput, prelude::*};
use bevy_ecs::{lifecycle::HookContext, world::DeferredWorld};
use bevy_enhanced_input::prelude::*;
use bevy_time::Stopwatch;

use crate::util::{ControlsOverlay, ExampleUtilPlugin, StableGround};

mod util;

const PLAYER_SPAWN_CLEARANCE: f32 = 2.4;
const PLAYER_GRAVITY: f32 = 29.0;
const PLAYER_STEP_SIZE: f32 = 1.0;
const PLAYER_GROUND_DISTANCE: f32 = 0.05;
const PLAYER_STEP_DOWN_DETECTION_DISTANCE: f32 = 0.2;
const PLAYER_SKIN_WIDTH: f32 = 0.008;
const SURF_STEP_SIZE: f32 = 0.0;
const SURF_GROUND_DISTANCE: f32 = 0.012;
const SURF_STEP_DOWN_DETECTION_DISTANCE: f32 = 0.03;
const SURF_SKIN_WIDTH: f32 = 0.003;
const PLAYER_MOVE_AND_SLIDE_ITERATIONS: usize = 8;
const PLAYER_DEPENETRATION_ITERATIONS: usize = 8;
const PHYSICS_SUBSTEPS: u32 = 12;
const CHECKPOINT_RADIUS: f32 = 3.2;
const CHECKPOINT_DEATH_MARGIN: f32 = 90.0;
const INITIAL_ROOM_COUNT: usize = 16;
const APPEND_TRIGGER_ROOMS: usize = 4;
const APPEND_BATCH_ROOMS: usize = 8;
const MAX_SECTION_TURN_RADIANS: f32 = 18.0_f32.to_radians();
const BHOP_OBJECT_SCALE: f32 = 5.0;
const BHOP_CADENCE_SCALE: f32 = 4.1;
const SURF_WEDGE_THICKNESS: f32 = 0.16;
const SURF_COLLIDER_OVERLAP_MIN: f32 = 0.14;
const SURF_COLLIDER_OVERLAP_MAX: f32 = 0.42;
const SURF_TRIMESH_MARGIN: f32 = 0.015;
const SURF_MAX_SEAM_TURN_RADIANS: f32 = 2.25_f32.to_radians();
const SURF_MAX_SEGMENT_LENGTH: f32 = 3.8;
const SURF_MAX_RENDER_SEGMENTS: usize = 224;
const SURF_COLLIDER_SAMPLE_LENGTH: f32 = 0.85;
const SURF_MAX_COLLIDER_SEGMENTS: usize = 640;
const SURF_COLLIDER_COLUMNS: usize = 5;
const ROLLOVER_FORWARD_SPEED_THRESHOLD: f32 = 3.5;
const ROLLOVER_CATCH_DROP_MIN: f32 = 44.0;
const ROLLOVER_CATCH_DROP_MAX: f32 = 72.0;
const ROLLOVER_CATCH_DROP_SPEED_SCALE: f32 = 0.36;
const ROLLOVER_ENTRY_LEAD: f32 = 10.0;

struct BasicGamePlugin;

impl Plugin for BasicGamePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(Color::srgb(0.75, 0.78, 0.82)))
            .insert_resource(RunDirector::default())
            .insert_resource(WorldAssetCache::default())
            .insert_resource(SubstepCount(PHYSICS_SUBSTEPS))
            .insert_resource(NarrowPhaseConfig {
                default_speculative_margin: 0.0,
                contact_tolerance: 0.001,
                match_contacts: true,
            })
            .add_systems(Startup, (setup_scene, setup_hud).chain())
            .add_systems(
                PostStartup,
                (tune_player_camera, configure_controls_overlay).chain(),
            )
            .add_systems(
                Update,
                (
                    capture_cursor.run_if(input_just_pressed(MouseButton::Left)),
                    release_cursor.run_if(input_just_pressed(KeyCode::Escape)),
                ),
            )
            .add_systems(
                Update,
                (
                    tick_run_timer,
                    queue_run_controls,
                    activate_checkpoints,
                    extend_course_ahead,
                    detect_failures,
                    process_run_request,
                    update_hud,
                )
                    .chain(),
            )
            .add_systems(
                FixedPostUpdate,
                normalize_surfing_motion.before(AhoySystems::MoveCharacters),
            );
    }
}

fn main() -> AppExit {
    App::new()
        .register_type::<SpawnPlayer>()
        .add_plugins((
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: basic_game_asset_path(),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Window {
                        title: "Bevy Ahoy Basic Game".into(),
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
            BasicGamePlugin,
        ))
        .add_input_context::<PlayerInput>()
        .run()
}

fn basic_game_asset_path() -> String {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let local_assets = manifest_dir.join("assets");
    if local_assets.exists() {
        return local_assets.to_string_lossy().into_owned();
    }

    manifest_dir
        .join("../assets")
        .to_string_lossy()
        .into_owned()
}

fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut asset_cache: ResMut<WorldAssetCache>,
) {
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.72, 0.74, 0.77),
        brightness: 16.0,
        affects_lightmapped_meshes: true,
    });

    let blueprint = build_run_blueprint(current_run_seed());
    let initial_look = respawn_look_for_checkpoint(&blueprint, 0);
    spawn_world(
        &blueprint,
        0,
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut asset_cache,
    );
    commands.insert_resource(RunState::new(blueprint.clone()));

    commands.spawn((
        Name::new("Spawn Point"),
        SpawnPlayer,
        Transform::from_translation(blueprint.spawn).with_rotation(initial_look.to_quat()),
        GlobalTransform::default(),
    ));

    let player = commands
        .spawn((
            Name::new("Player"),
            Player,
            PlayerInput,
            SurfMovementState::default(),
            CharacterController {
                speed: 9.8,
                air_speed: 4.2,
                air_acceleration_hz: 1000.0,
                jump_height: 1.9,
                max_speed: 3000.0,
                gravity: PLAYER_GRAVITY,
                step_size: PLAYER_STEP_SIZE,
                mantle_height: 3.4,
                crane_height: 4.1,
                mantle_speed: 2.2,
                crane_speed: 3.5,
                ground_distance: PLAYER_GROUND_DISTANCE,
                step_down_detection_distance: PLAYER_STEP_DOWN_DETECTION_DISTANCE,
                min_mantle_ledge_space: 0.28,
                min_crane_ledge_space: 0.22,
                min_ledge_grab_space: Cuboid::new(0.18, 0.08, 0.22),
                max_ledge_grab_distance: 0.72,
                climb_pull_up_height: 0.48,
                min_mantle_cos: 24.0_f32.to_radians().cos(),
                min_crane_cos: 18.0_f32.to_radians().cos(),
                move_and_slide: MoveAndSlideConfig {
                    skin_width: PLAYER_SKIN_WIDTH,
                    move_and_slide_iterations: PLAYER_MOVE_AND_SLIDE_ITERATIONS,
                    depenetration_iterations: PLAYER_DEPENETRATION_ITERATIONS,
                    ..default()
                },
                ..default()
            },
            RigidBody::Kinematic,
            Collider::cylinder(0.7, 1.8),
            CollisionLayers::new(CollisionLayer::Player, LayerMask::ALL),
            StableGround::default(),
            Transform::from_translation(blueprint.spawn),
            Position(blueprint.spawn),
            initial_look.clone(),
        ))
        .id();

    commands.spawn((
        Name::new("Player Camera"),
        Camera3d::default(),
        CharacterControllerCameraOf::new(player),
        Transform::from_rotation(initial_look.to_quat()),
    ));
}

fn spawn_world(
    blueprint: &RunBlueprint,
    epoch: u64,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_cache: &mut WorldAssetCache,
) {
    spawn_basic_lighting(epoch, commands);
    spawn_world_range(
        blueprint,
        0,
        0,
        epoch,
        commands,
        meshes,
        materials,
        asset_cache,
    );
}

fn spawn_world_range(
    blueprint: &RunBlueprint,
    room_start: usize,
    segment_start: usize,
    epoch: u64,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_cache: &mut WorldAssetCache,
) {
    for room in &blueprint.rooms[room_start.min(blueprint.rooms.len())..] {
        let layout = build_room_layout(room);
        spawn_layout(&layout, epoch, commands, meshes, materials, asset_cache);
    }
    for segment in &blueprint.segments[segment_start.min(blueprint.segments.len())..] {
        let layout = build_segment_layout(segment, &blueprint.rooms);
        spawn_layout(&layout, epoch, commands, meshes, materials, asset_cache);
    }
}

fn spawn_basic_lighting(epoch: u64, commands: &mut Commands) {
    commands.spawn((
        GeneratedWorld { epoch },
        Name::new("Sun"),
        Transform::from_xyz(80.0, 180.0, 60.0).looking_at(Vec3::new(0.0, 40.0, 0.0), Vec3::Y),
        DirectionalLight {
            shadows_enabled: true,
            illuminance: 18_000.0,
            color: Color::srgb(1.0, 0.98, 0.95),
            ..default()
        },
    ));
}

fn setup_hud(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: px(16.0),
            left: px(16.0),
            max_width: px(420.0),
            padding: UiRect::axes(px(14.0), px(12.0)),
            ..default()
        },
        Text::new("Basic Game\nPreparing course..."),
        TextFont {
            font_size: 17.0,
            ..default()
        },
        TextColor(Color::srgb(0.08, 0.1, 0.14)),
        BackgroundColor(Color::srgba(0.94, 0.96, 0.98, 0.72)),
        RunHud,
    ));
}

fn configure_controls_overlay(mut overlay: Single<&mut Text, With<ControlsOverlay>>) {
    overlay.0 = "Controls:\n\
WASD: move\n\
Space: jump\n\
Ctrl: crouch\n\
F5: rerun seed\n\
N: new seed\n\
R: reset to checkpoint\n\
Esc: free mouse\n\
Backtick: toggle debug"
        .into();
}

fn update_hud(
    run: Res<RunState>,
    players: Query<(&Transform, &LinearVelocity), With<Player>>,
    mut hud: Single<&mut Text, With<RunHud>>,
) {
    let Ok((player, velocity)) = players.single() else {
        return;
    };

    hud.0 = format!(
        "Basic Game\n\
         Seed: {seed:016x}\n\
         Sections: {rooms} | Checkpoint: {checkpoint}/{checkpoint_total}\n\
         Height: {height:.1} | Speed: {speed:.1}\n\
         Time: {time:.1}s | Deaths: {deaths}\n\
         Mode: Endless",
        seed = run.blueprint.seed,
        rooms = run.blueprint.rooms.len(),
        checkpoint = run.current_checkpoint + 1,
        checkpoint_total = run.checkpoint_count(),
        height = player.translation.y,
        speed = velocity.length(),
        time = run.timer.elapsed_secs(),
        deaths = run.deaths,
    );
}

fn tick_run_timer(time: Res<Time>, mut run: ResMut<RunState>) {
    run.timer.tick(time.delta());
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
            seed: run.blueprint.seed,
        });
    } else if keys.just_pressed(KeyCode::KeyR) {
        director.pending = Some(RunRequest {
            kind: RunRequestKind::Respawn,
            seed: run.blueprint.seed,
        });
    } else if keys.just_pressed(KeyCode::KeyN) {
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
    mut spawn_marker: Single<&mut Transform, (With<SpawnPlayer>, Without<Player>)>,
) {
    let Ok(player) = players.single() else {
        return;
    };

    for (transform, checkpoint) in &pads {
        if checkpoint.epoch != run.epoch {
            continue;
        }
        let delta = transform.translation - player.translation;
        if delta.y.abs() < 2.0
            && delta.xz().length() <= CHECKPOINT_RADIUS
            && checkpoint.index > run.current_checkpoint
        {
            run.current_checkpoint = checkpoint.index;
            spawn_marker.translation = run.checkpoint_position(checkpoint.index);
            spawn_marker.rotation =
                respawn_look_for_checkpoint(&run.blueprint, checkpoint.index).to_quat();
            run.death_plane = checkpoint_death_plane(&run.blueprint, checkpoint.index);
        }
    }
}

fn extend_course_ahead(
    mut commands: Commands,
    director: Res<RunDirector>,
    mut run: ResMut<RunState>,
    players: Query<&Transform, With<Player>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut asset_cache: ResMut<WorldAssetCache>,
) {
    if director.pending.is_some() {
        return;
    }

    let Ok(player) = players.single() else {
        return;
    };

    let checkpoint_room = run.blueprint.checkpoint_room_index(run.current_checkpoint);
    let focus_room = nearest_room_index(&run.blueprint, player.translation).max(checkpoint_room);
    let rooms_remaining = run.blueprint.rooms.len().saturating_sub(focus_room + 1);
    if rooms_remaining > APPEND_TRIGGER_ROOMS {
        return;
    }

    let room_start = run.blueprint.rooms.len();
    let segment_start = run.blueprint.segments.len();
    append_run_blueprint(&mut run.blueprint, APPEND_BATCH_ROOMS);
    spawn_world_range(
        &run.blueprint,
        room_start,
        segment_start,
        run.epoch,
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut asset_cache,
    );
    run.death_plane = checkpoint_death_plane(&run.blueprint, run.current_checkpoint);
}

fn nearest_room_index(blueprint: &RunBlueprint, player_position: Vec3) -> usize {
    blueprint
        .rooms
        .iter()
        .min_by(|left, right| {
            room_focus_score(left, player_position)
                .total_cmp(&room_focus_score(right, player_position))
        })
        .map(|room| room.index)
        .unwrap_or(0)
}

fn room_focus_score(room: &RoomPlan, player_position: Vec3) -> f32 {
    let delta = room.top - player_position;
    delta.xz().length() + delta.y.abs() * 0.28
}

fn detect_failures(
    players: Query<(&Transform, &LinearVelocity, &CharacterLook), With<Player>>,
    run: Res<RunState>,
    mut director: ResMut<RunDirector>,
) {
    if director.pending.is_some() {
        return;
    }

    let Ok((player, velocity, look)) = players.single() else {
        return;
    };
    if player.translation.y < run.death_plane {
        let snapshot = FallSnapshot::new(player.translation, velocity.0, look_forward(look));
        director.pending = Some(RunRequest {
            kind: RunRequestKind::Rollover(snapshot.clone()),
            seed: rollover_seed(&run, &snapshot),
        });
    }
}

fn process_run_request(
    mut commands: Commands,
    mut director: ResMut<RunDirector>,
    mut run: ResMut<RunState>,
    generated: Query<(Entity, &GeneratedWorld)>,
    mut players: Query<
        (
            &mut Position,
            &mut Transform,
            &mut LinearVelocity,
            &mut CharacterLook,
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
    mut asset_cache: ResMut<WorldAssetCache>,
) {
    let Some(request) = director.pending.take() else {
        return;
    };

    let mut reset_player = false;
    let mut reset_velocity = true;
    let mut reset_look = true;

    match request.kind {
        RunRequestKind::Respawn => {
            reset_player = true;
        }
        RunRequestKind::RestartSameSeed | RunRequestKind::RestartNewSeed => {
            let old_epoch = run.epoch;
            let next_epoch = run.reserve_epoch();
            let blueprint = build_run_blueprint(request.seed);
            spawn_world(
                &blueprint,
                next_epoch,
                &mut commands,
                &mut meshes,
                &mut materials,
                &mut asset_cache,
            );
            despawn_generated_epoch(&mut commands, &generated, old_epoch);
            run.apply_restart_blueprint(blueprint, next_epoch);
            reset_player = true;
        }
        RunRequestKind::Rollover(snapshot) => {
            let old_epoch = run.epoch;
            let next_epoch = run.reserve_epoch();
            let blueprint = build_rollover_blueprint(request.seed, &snapshot);
            spawn_world(
                &blueprint,
                next_epoch,
                &mut commands,
                &mut meshes,
                &mut materials,
                &mut asset_cache,
            );
            despawn_generated_epoch(&mut commands, &generated, old_epoch);
            run.apply_rollover_blueprint(blueprint, next_epoch);
            reset_velocity = false;
            reset_look = false;
        }
    }

    let spawn = run.checkpoint_position(run.current_checkpoint);
    let look = respawn_look_for_checkpoint(&run.blueprint, run.current_checkpoint);
    spawn_marker.translation = spawn;
    spawn_marker.rotation = look.to_quat();

    if reset_player
        && let Ok((mut position, mut transform, mut velocity, mut character_look)) =
            players.single_mut()
    {
        position.0 = spawn;
        transform.translation = spawn;
        if reset_velocity {
            velocity.0 = Vec3::ZERO;
        }
        if reset_look {
            *character_look = look.clone();
        }
    }

    if reset_look && let Ok(mut camera_transform) = camera.single_mut() {
        camera_transform.rotation = look.to_quat();
    }
}

fn despawn_generated_epoch(
    commands: &mut Commands,
    generated: &Query<(Entity, &GeneratedWorld)>,
    epoch: u64,
) {
    for (entity, generated) in generated.iter() {
        if generated.epoch == epoch {
            commands.entity(entity).despawn();
        }
    }
}

fn respawn_look_for_checkpoint(blueprint: &RunBlueprint, checkpoint_index: usize) -> CharacterLook {
    let current_room = blueprint.checkpoint_room_index(checkpoint_index);
    let next_room = (current_room + 1).min(blueprint.rooms.len().saturating_sub(1));
    let mut facing = blueprint.rooms[next_room].top - blueprint.rooms[current_room].top;
    facing.y = 0.0;
    let facing = facing.normalize_or_zero();
    if facing == Vec3::ZERO {
        return CharacterLook::default();
    }

    CharacterLook {
        yaw: facing.x.atan2(facing.z),
        pitch: 0.0,
    }
}

fn checkpoint_death_plane(blueprint: &RunBlueprint, checkpoint_index: usize) -> f32 {
    let checkpoint_room = blueprint.checkpoint_room_index(checkpoint_index);
    let end_room = (checkpoint_room + 4).min(blueprint.rooms.len().saturating_sub(1));
    let min_y = blueprint.rooms[checkpoint_room..=end_room]
        .iter()
        .map(|room| room.top.y)
        .fold(f32::INFINITY, f32::min);
    min_y - CHECKPOINT_DEATH_MARGIN
}

fn tune_player_camera(mut cameras: Query<&mut Projection, With<Camera3d>>) {
    for mut projection in &mut cameras {
        if let Projection::Perspective(perspective) = &mut *projection {
            perspective.near = 0.03;
            perspective.fov = 82.0_f32.to_radians();
            perspective.far = 1_600.0;
        }
    }
}

fn capture_cursor(
    mut cursor: Single<&mut CursorOptions>,
    hovered_buttons: Query<&Interaction, With<Button>>,
) {
    if hovered_buttons
        .iter()
        .any(|interaction| *interaction != Interaction::None)
    {
        return;
    }
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
struct GeneratedWorld {
    epoch: u64,
}

#[derive(Component)]
struct CheckpointPad {
    index: usize,
    epoch: u64,
}

#[derive(Component)]
struct SurfRampSurface;

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
                    bindings![KeyCode::Space, GamepadButton::South],
                ),
                (
                    Action::<Crouch>::new(),
                    bindings![KeyCode::ControlLeft, GamepadButton::LeftTrigger2],
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
struct SurfMovementState {
    jump_lock: f32,
}

fn normalize_surfing_motion(
    time: Res<Time>,
    surf_surfaces: Query<(), With<SurfRampSurface>>,
    mut players: Query<
        (
            &CharacterControllerOutput,
            &mut CharacterController,
            &mut AccumulatedInput,
            &mut SurfMovementState,
        ),
        With<Player>,
    >,
) {
    let dt = time.delta_secs();
    for (output, mut controller, mut input, mut surf_state) in &mut players {
        let touching_surf = output
            .touching_entities
            .iter()
            .any(|touch| surf_surfaces.contains(touch.entity) && is_surf_touch(touch.normal));

        if touching_surf {
            surf_state.jump_lock = 0.18;
        } else {
            surf_state.jump_lock = (surf_state.jump_lock - dt).max(0.0);
        }

        if surf_state.jump_lock > 0.0 {
            input.jumped = None;
            input.tac = None;
            input.craned = None;
            input.mantled = None;
        }

        if touching_surf || surf_state.jump_lock > 0.0 {
            controller.step_size = SURF_STEP_SIZE;
            controller.ground_distance = SURF_GROUND_DISTANCE;
            controller.step_down_detection_distance = SURF_STEP_DOWN_DETECTION_DISTANCE;
            controller.move_and_slide.skin_width = SURF_SKIN_WIDTH;
        } else {
            controller.step_size = PLAYER_STEP_SIZE;
            controller.ground_distance = PLAYER_GROUND_DISTANCE;
            controller.step_down_detection_distance = PLAYER_STEP_DOWN_DETECTION_DISTANCE;
            controller.move_and_slide.skin_width = PLAYER_SKIN_WIDTH;
        }
    }
}

fn is_surf_touch(normal: Dir3) -> bool {
    normal.y > 0.18 && normal.y < 0.93 && Vec2::new(normal.x, normal.z).length_squared() > 0.08
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
    Rollover(FallSnapshot),
}

#[derive(Resource)]
struct RunState {
    blueprint: RunBlueprint,
    epoch: u64,
    next_epoch: u64,
    death_plane: f32,
    current_checkpoint: usize,
    deaths: u32,
    timer: Stopwatch,
}

impl RunState {
    fn new(blueprint: RunBlueprint) -> Self {
        Self {
            death_plane: checkpoint_death_plane(&blueprint, 0),
            blueprint,
            epoch: 0,
            next_epoch: 1,
            current_checkpoint: 0,
            deaths: 0,
            timer: Stopwatch::new(),
        }
    }

    fn reserve_epoch(&mut self) -> u64 {
        let epoch = self.next_epoch;
        self.next_epoch += 1;
        epoch
    }

    fn apply_restart_blueprint(&mut self, blueprint: RunBlueprint, epoch: u64) {
        self.blueprint = blueprint;
        self.epoch = epoch;
        self.current_checkpoint = 0;
        self.death_plane = checkpoint_death_plane(&self.blueprint, 0);
        self.deaths = 0;
        self.timer = Stopwatch::new();
    }

    fn apply_rollover_blueprint(&mut self, blueprint: RunBlueprint, epoch: u64) {
        self.blueprint = blueprint;
        self.epoch = epoch;
        self.current_checkpoint = 0;
        self.death_plane = checkpoint_death_plane(&self.blueprint, 0);
    }

    fn checkpoint_count(&self) -> usize {
        self.blueprint.checkpoint_count()
    }

    fn checkpoint_position(&self, checkpoint_index: usize) -> Vec3 {
        self.blueprint.checkpoint_position(checkpoint_index)
    }
}

#[derive(Clone)]
struct RunBlueprint {
    seed: u64,
    rooms: Vec<RoomPlan>,
    segments: Vec<SegmentPlan>,
    spawn: Vec3,
    tail_forward: Vec3,
    next_segment_kind: SegmentKind,
    next_checkpoint_slot: usize,
    generator: RunRng,
}

impl RunBlueprint {
    fn checkpoint_count(&self) -> usize {
        self.rooms
            .iter()
            .filter(|room| room.checkpoint_slot.is_some())
            .count()
            .max(1)
    }

    fn checkpoint_room_index(&self, checkpoint_index: usize) -> usize {
        self.rooms
            .iter()
            .enumerate()
            .filter(|(_, room)| room.checkpoint_slot.is_some())
            .nth(checkpoint_index.min(self.checkpoint_count().saturating_sub(1)))
            .map(|(index, _)| index)
            .unwrap_or(0)
    }

    fn checkpoint_position(&self, checkpoint_index: usize) -> Vec3 {
        if checkpoint_index == 0 {
            return self.spawn;
        }
        let room = &self.rooms[self.checkpoint_room_index(checkpoint_index)];
        room.top + Vec3::new(0.0, PLAYER_SPAWN_CLEARANCE, 0.0)
    }
}

#[derive(Clone)]
struct RoomPlan {
    index: usize,
    top: Vec3,
    checkpoint_slot: Option<usize>,
}

#[derive(Clone)]
struct SegmentPlan {
    index: usize,
    from: usize,
    to: usize,
    kind: SegmentKind,
    seed: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SegmentKind {
    SurfRamp,
    SquareBhop,
}

#[derive(Clone, Debug)]
struct FallSnapshot {
    position: Vec3,
    velocity: Vec3,
    forward: Vec3,
}

impl FallSnapshot {
    fn new(position: Vec3, velocity: Vec3, fallback_forward: Vec3) -> Self {
        let horizontal_velocity = Vec3::new(velocity.x, 0.0, velocity.z);
        let forward = if horizontal_velocity.length_squared()
            >= ROLLOVER_FORWARD_SPEED_THRESHOLD * ROLLOVER_FORWARD_SPEED_THRESHOLD
        {
            horizontal_velocity.normalize()
        } else {
            direction_from_delta(fallback_forward)
        };

        Self {
            position,
            velocity,
            forward,
        }
    }
}

fn look_forward(look: &CharacterLook) -> Vec3 {
    direction_from_delta(Vec3::new(look.yaw.sin(), 0.0, look.yaw.cos()))
}

fn rollover_seed(run: &RunState, snapshot: &FallSnapshot) -> u64 {
    let position_mix = (snapshot.position.x.to_bits() as u64).rotate_left(7)
        ^ (snapshot.position.y.to_bits() as u64).rotate_left(23)
        ^ (snapshot.position.z.to_bits() as u64).rotate_left(41);
    let velocity_mix = (snapshot.velocity.x.to_bits() as u64).rotate_left(13)
        ^ (snapshot.velocity.y.to_bits() as u64).rotate_left(29)
        ^ (snapshot.velocity.z.to_bits() as u64).rotate_left(47);

    run.blueprint.seed.rotate_left(11)
        ^ run.epoch.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ position_mix
        ^ velocity_mix
        ^ 0xC6A4_A793_5BD1_E995
}

fn build_run_blueprint(seed: u64) -> RunBlueprint {
    let spawn_room = RoomPlan {
        index: 0,
        top: Vec3::new(0.0, 160.0, 0.0),
        checkpoint_slot: Some(0),
    };
    let mut blueprint = RunBlueprint {
        seed,
        rooms: vec![spawn_room],
        segments: Vec::with_capacity(INITIAL_ROOM_COUNT.saturating_sub(1)),
        spawn: Vec3::ZERO,
        tail_forward: Vec3::X,
        next_segment_kind: SegmentKind::SquareBhop,
        next_checkpoint_slot: 1,
        generator: RunRng::new(seed),
    };
    append_run_blueprint(&mut blueprint, INITIAL_ROOM_COUNT.saturating_sub(1));
    blueprint.spawn = spawn_on_first_bhop_platform(&blueprint);
    blueprint
}

fn build_rollover_blueprint(seed: u64, snapshot: &FallSnapshot) -> RunBlueprint {
    let catch_room = RoomPlan {
        index: 0,
        top: rollover_catch_room_top(snapshot),
        checkpoint_slot: Some(0),
    };
    let spawn = catch_room.top + Vec3::new(0.0, PLAYER_SPAWN_CLEARANCE, 0.0);
    let mut blueprint = RunBlueprint {
        seed,
        rooms: vec![catch_room],
        segments: Vec::with_capacity(INITIAL_ROOM_COUNT.saturating_sub(1)),
        spawn,
        tail_forward: snapshot.forward,
        next_segment_kind: SegmentKind::SurfRamp,
        next_checkpoint_slot: 1,
        generator: RunRng::new(seed),
    };
    append_run_blueprint(&mut blueprint, INITIAL_ROOM_COUNT.saturating_sub(1));
    blueprint
}

fn rollover_catch_room_top(snapshot: &FallSnapshot) -> Vec3 {
    let drop = rollover_drop_distance(snapshot.velocity.y);
    let fall_time = solve_fall_time(snapshot.velocity.y, drop);
    let horizontal_velocity = Vec3::new(snapshot.velocity.x, 0.0, snapshot.velocity.z);

    snapshot.position + horizontal_velocity * fall_time + snapshot.forward * ROLLOVER_ENTRY_LEAD
        - Vec3::Y * drop
}

fn rollover_drop_distance(vertical_velocity: f32) -> f32 {
    (ROLLOVER_CATCH_DROP_MIN + (-vertical_velocity).max(0.0) * ROLLOVER_CATCH_DROP_SPEED_SCALE)
        .clamp(ROLLOVER_CATCH_DROP_MIN, ROLLOVER_CATCH_DROP_MAX)
}

fn solve_fall_time(vertical_velocity: f32, drop: f32) -> f32 {
    let discriminant = (vertical_velocity * vertical_velocity + 2.0 * PLAYER_GRAVITY * drop).sqrt();
    ((vertical_velocity + discriminant) / PLAYER_GRAVITY).max(0.35)
}

fn append_run_blueprint(blueprint: &mut RunBlueprint, additional_rooms: usize) {
    if additional_rooms == 0 || blueprint.rooms.is_empty() {
        return;
    }

    for _ in 0..additional_rooms {
        let from_index = blueprint.rooms.len() - 1;
        let from_top = blueprint.rooms[from_index].top;
        let kind = blueprint.next_segment_kind;
        let turn = blueprint
            .generator
            .range_f32(-MAX_SECTION_TURN_RADIANS, MAX_SECTION_TURN_RADIANS);
        let forward = (Quat::from_rotation_y(turn) * blueprint.tail_forward).normalize_or_zero();
        let right = Vec3::new(-forward.z, 0.0, forward.x).normalize_or_zero();
        let gap = match kind {
            SegmentKind::SurfRamp => blueprint.generator.range_f32(88.0, 116.0),
            SegmentKind::SquareBhop => blueprint.generator.range_f32(72.0, 96.0),
        };
        let drop = match kind {
            SegmentKind::SurfRamp => blueprint.generator.range_f32(18.0, 28.0),
            SegmentKind::SquareBhop => blueprint.generator.range_f32(12.0, 22.0),
        };
        let lateral_jitter = right * blueprint.generator.range_f32(-4.0, 4.0);
        let next_top = from_top + forward * gap + lateral_jitter - Vec3::Y * drop;
        let next_index = blueprint.rooms.len();
        let checkpoint_slot = checkpoint_slot_for_room(blueprint, next_index);

        blueprint.segments.push(SegmentPlan {
            index: blueprint.segments.len(),
            from: from_index,
            to: next_index,
            kind,
            seed: blueprint.generator.next_u64(),
        });
        blueprint.rooms.push(RoomPlan {
            index: next_index,
            top: next_top,
            checkpoint_slot,
        });
        blueprint.tail_forward = forward;
        blueprint.next_segment_kind = next_segment_kind(kind);
    }
}

fn checkpoint_slot_for_room(blueprint: &mut RunBlueprint, index: usize) -> Option<usize> {
    if should_assign_checkpoint(index) {
        let slot = blueprint.next_checkpoint_slot;
        blueprint.next_checkpoint_slot += 1;
        Some(slot)
    } else {
        None
    }
}

fn next_segment_kind(kind: SegmentKind) -> SegmentKind {
    match kind {
        SegmentKind::SurfRamp => SegmentKind::SquareBhop,
        SegmentKind::SquareBhop => SegmentKind::SurfRamp,
    }
}

fn should_assign_checkpoint(index: usize) -> bool {
    index == 0 || index == 1 || index % 3 == 0
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct MeshSizeKey {
    x: u32,
    y: u32,
    z: u32,
}

impl MeshSizeKey {
    fn from_size(size: Vec3) -> Self {
        Self {
            x: size.x.to_bits(),
            y: size.y.to_bits(),
            z: size.z.to_bits(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum MaterialKey {
    BhopPlatform,
    SurfVertex,
}

#[derive(Resource, Default)]
struct WorldAssetCache {
    cuboid_meshes: HashMap<MeshSizeKey, Handle<Mesh>>,
    materials: HashMap<MaterialKey, Handle<StandardMaterial>>,
}

#[derive(Clone)]
struct SolidSpec {
    label: String,
    center: Vec3,
    size: Vec3,
    paint: PaintStyle,
    body: SolidBody,
    friction: Option<f32>,
}

#[derive(Clone)]
enum SolidBody {
    Static,
    StaticSurfWedge {
        #[cfg_attr(not(test), allow(dead_code))]
        wall_side: f32,
        render_points: Vec<Vec3>,
    },
    StaticSurfStrip {
        #[cfg_attr(not(test), allow(dead_code))]
        wall_side: f32,
        collider_strip_points: Vec<Vec3>,
        columns: usize,
    },
}

#[derive(Clone)]
enum FeatureSpec {
    CheckpointPad { center: Vec3, index: usize },
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum PaintStyle {
    BhopPlatform,
    SurfRamp,
}

#[derive(Default, Clone)]
struct ModuleLayout {
    solids: Vec<SolidSpec>,
    features: Vec<FeatureSpec>,
}

fn build_room_layout(room: &RoomPlan) -> ModuleLayout {
    let mut layout = ModuleLayout::default();
    if let Some(index) = room.checkpoint_slot {
        layout.features.push(FeatureSpec::CheckpointPad {
            center: room.top,
            index,
        });
    }

    layout
}

fn build_segment_layout(segment: &SegmentPlan, rooms: &[RoomPlan]) -> ModuleLayout {
    let from = &rooms[segment.from];
    let to = &rooms[segment.to];
    let mut layout = ModuleLayout::default();
    let mut rng = RunRng::new(segment.seed);
    let forward = direction_from_delta(to.top - from.top);
    let right = Vec3::new(-forward.z, 0.0, forward.x).normalize_or_zero();
    let start = from.top;
    let end = to.top;

    match segment.kind {
        SegmentKind::SurfRamp => append_surf_sequence(
            &mut layout,
            &mut rng,
            start,
            end,
            forward,
            right,
            segment.index == 0,
        ),
        SegmentKind::SquareBhop => {
            append_square_bhop_sequence(&mut layout, &mut rng, start, end, forward, right)
        }
    }

    layout
}

fn spawn_on_first_bhop_platform(blueprint: &RunBlueprint) -> Vec3 {
    let Some(segment) = blueprint
        .segments
        .iter()
        .find(|segment| segment.kind == SegmentKind::SquareBhop)
    else {
        return blueprint.rooms[0].top + Vec3::new(0.0, PLAYER_SPAWN_CLEARANCE, 0.0);
    };
    let layout = build_segment_layout(segment, &blueprint.rooms);
    let Some(platform) = layout
        .solids
        .iter()
        .find(|solid| matches!(solid.body, SolidBody::Static))
    else {
        return blueprint.rooms[0].top + Vec3::new(0.0, PLAYER_SPAWN_CLEARANCE, 0.0);
    };
    let top = platform.center + Vec3::Y * (platform.size.y * 0.5);
    top + Vec3::new(0.0, PLAYER_SPAWN_CLEARANCE, 0.0)
}

fn spawn_layout(
    layout: &ModuleLayout,
    epoch: u64,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_cache: &mut WorldAssetCache,
) {
    for solid in &layout.solids {
        spawn_solid(solid, epoch, commands, meshes, materials, asset_cache);
    }
    for feature in &layout.features {
        match feature {
            FeatureSpec::CheckpointPad { center, index } => {
                commands.spawn((
                    GeneratedWorld { epoch },
                    Name::new("Checkpoint Pad"),
                    Transform::from_translation(*center),
                    CheckpointPad {
                        index: *index,
                        epoch,
                    },
                ));
            }
        }
    }
}

fn spawn_solid(
    spec: &SolidSpec,
    epoch: u64,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_cache: &mut WorldAssetCache,
) {
    if let Err(reason) = validate_solid_spec(spec) {
        eprintln!("Skipping invalid solid '{}': {}", spec.label, reason);
        return;
    }

    match &spec.body {
        SolidBody::Static => {
            let mut entity = commands.spawn((
                GeneratedWorld { epoch },
                Name::new(spec.label.clone()),
                Mesh3d(cached_cuboid_mesh(asset_cache, meshes, spec.size)),
                MeshMaterial3d(cached_material(
                    asset_cache,
                    materials,
                    material_key_for_paint(spec.paint),
                )),
                Transform::from_translation(spec.center),
                RigidBody::Static,
                Collider::cuboid(spec.size.x, spec.size.y, spec.size.z),
                CollisionLayers::new(CollisionLayer::Default, LayerMask::ALL),
            ));
            if let Some(friction) = spec.friction {
                entity.insert(Friction::new(friction));
            }
        }
        SolidBody::StaticSurfWedge { render_points, .. } => {
            commands.spawn((
                GeneratedWorld { epoch },
                Name::new(spec.label.clone()),
                Mesh3d(meshes.add(build_surf_wedge_mesh(
                    render_points,
                    paint_base_color(spec.paint),
                    paint_stripe_color(spec.paint),
                ))),
                MeshMaterial3d(cached_material(
                    asset_cache,
                    materials,
                    MaterialKey::SurfVertex,
                )),
                Transform::from_translation(spec.center),
            ));
        }
        SolidBody::StaticSurfStrip {
            collider_strip_points,
            columns,
            ..
        } => {
            let mut entity = commands.spawn((
                GeneratedWorld { epoch },
                Name::new(spec.label.clone()),
                Transform::from_translation(spec.center),
                GlobalTransform::default(),
            ));
            if let Some(mesh) = build_surf_strip_collider_mesh(collider_strip_points, *columns)
                && let Some(collider) =
                    Collider::trimesh_from_mesh_with_config(&mesh, TrimeshFlags::FIX_INTERNAL_EDGES)
            {
                entity.insert((
                    RigidBody::Static,
                    collider,
                    CollisionMargin(SURF_TRIMESH_MARGIN),
                    SurfRampSurface,
                    CollisionLayers::new(CollisionLayer::Default, LayerMask::ALL),
                ));
                if let Some(friction) = spec.friction {
                    entity.insert(Friction::new(friction));
                }
            }
        }
    }
}

fn cached_cuboid_mesh(
    cache: &mut WorldAssetCache,
    meshes: &mut Assets<Mesh>,
    size: Vec3,
) -> Handle<Mesh> {
    let key = MeshSizeKey::from_size(size);
    if let Some(mesh) = cache.cuboid_meshes.get(&key) {
        return mesh.clone();
    }

    let mesh = meshes.add(Cuboid::new(size.x, size.y, size.z));
    cache.cuboid_meshes.insert(key, mesh.clone());
    mesh
}

fn cached_material(
    cache: &mut WorldAssetCache,
    materials: &mut Assets<StandardMaterial>,
    key: MaterialKey,
) -> Handle<StandardMaterial> {
    if let Some(handle) = cache.materials.get(&key) {
        return handle.clone();
    }

    let material = match key {
        MaterialKey::BhopPlatform => StandardMaterial {
            base_color: Color::srgb(0.48, 0.54, 0.62),
            reflectance: 0.58,
            clearcoat: 0.18,
            clearcoat_perceptual_roughness: 0.5,
            perceptual_roughness: 0.64,
            ..default()
        },
        MaterialKey::SurfVertex => StandardMaterial {
            base_color: Color::WHITE,
            cull_mode: None,
            reflectance: 0.76,
            clearcoat: 0.88,
            clearcoat_perceptual_roughness: 0.08,
            perceptual_roughness: 0.12,
            ..default()
        },
    };
    let handle = materials.add(material);
    cache.materials.insert(key, handle.clone());
    handle
}

fn material_key_for_paint(paint: PaintStyle) -> MaterialKey {
    match paint {
        PaintStyle::BhopPlatform => MaterialKey::BhopPlatform,
        PaintStyle::SurfRamp => MaterialKey::SurfVertex,
    }
}

fn paint_base_color(paint: PaintStyle) -> Color {
    match paint {
        PaintStyle::BhopPlatform => Color::srgb(0.48, 0.54, 0.62),
        PaintStyle::SurfRamp => Color::srgb(0.42, 0.58, 0.8),
    }
}

fn paint_stripe_color(paint: PaintStyle) -> Color {
    match paint {
        PaintStyle::BhopPlatform => Color::linear_rgb(0.82, 0.88, 0.96),
        PaintStyle::SurfRamp => Color::linear_rgb(1.0, 1.0, 1.0),
    }
}

#[derive(Clone, Copy, Debug)]
enum PathLateralStyle {
    Straight,
    Serpentine,
    Switchback,
    Arc,
    OneSidedArc,
}

fn append_square_bhop_sequence(
    layout: &mut ModuleLayout,
    rng: &mut RunRng,
    start: Vec3,
    end: Vec3,
    forward: Vec3,
    right: Vec3,
) {
    let _ = forward;
    let distance = start.distance(end).max(18.0);
    let start_margin = bhop_path_margin(distance, 0.15, 0.85);
    let end_margin = bhop_path_margin(distance, 0.12, 0.75);
    let path_start = start + direction_from_delta(end - start) * start_margin + Vec3::Y * 0.24;
    let path_end = end - direction_from_delta(end - start) * end_margin + Vec3::Y * 0.18;
    let requested_count =
        ((distance / scaled_bhop_cadence(4.8, 6.6, rng)).round() as usize).clamp(6, 16);
    let pad_count =
        clamp_platform_count_for_spacing(distance, requested_count, scaled_bhop_size(4.0), 4);
    let tops = sample_descending_platform_tops(
        path_start,
        path_end,
        right,
        pad_count,
        choose_bhop_path_style(rng),
        rng.range_f32(1.6, 3.2),
        rng.range_f32(0.0, TAU),
        scaled_bhop_size(rng.range_f32(1.0, 1.8)),
        scaled_bhop_size(rng.range_f32(0.06, 0.22)),
    );

    for (step, top) in tops.into_iter().enumerate() {
        let catch_platform = step == 0 || step + 1 == pad_count || step % 5 == 0;
        let square_side = scaled_bhop_size(if catch_platform {
            rng.range_f32(3.8, 5.4)
        } else if step % 3 == 0 {
            rng.range_f32(2.8, 3.6)
        } else {
            rng.range_f32(1.9, 2.8)
        });
        let pad_height = scaled_bhop_size(if catch_platform {
            rng.range_f32(0.9, 1.15)
        } else {
            rng.range_f32(0.72, 0.94)
        }) * 0.45;
        layout.solids.push(SolidSpec {
            label: if catch_platform {
                format!("Square Bhop Catch {step}")
            } else {
                format!("Square Bhop Pad {step}")
            },
            center: top_to_center(top, pad_height),
            size: Vec3::new(square_side, pad_height, square_side),
            paint: PaintStyle::BhopPlatform,
            body: SolidBody::Static,
            friction: None,
        });
    }
}

fn append_surf_sequence(
    layout: &mut ModuleLayout,
    rng: &mut RunRng,
    start: Vec3,
    end: Vec3,
    forward: Vec3,
    right: Vec3,
    intense: bool,
) {
    let direct_distance = start.distance(end).max(12.0);
    let entry_margin = if intense {
        (direct_distance * 0.01).clamp(0.7, 2.2)
    } else {
        (direct_distance * 0.008).clamp(0.5, 1.8)
    };
    let exit_margin = if intense {
        (direct_distance * 0.009).clamp(0.6, 2.0)
    } else {
        (direct_distance * 0.007).clamp(0.45, 1.5)
    };
    let surf_start = start + forward * entry_margin + Vec3::Y * 0.4;
    let surf_end = end - forward * exit_margin + Vec3::Y * 0.18;
    let total_distance = surf_start.distance(surf_end).max(12.0);
    let style = choose_surf_path_style(rng, intense);
    let weave_cycles = match style {
        PathLateralStyle::Straight => rng.range_f32(0.25, 0.8),
        PathLateralStyle::Arc => rng.range_f32(0.45, 1.15),
        PathLateralStyle::OneSidedArc => rng.range_f32(0.08, 0.4),
        PathLateralStyle::Serpentine | PathLateralStyle::Switchback => rng.range_f32(0.35, 0.9),
    };
    let phase = rng.range_f32(0.0, TAU);
    let lateral_amplitude = if intense {
        rng.range_f32(8.0, 14.0)
    } else {
        rng.range_f32(6.0, 10.0)
    };
    let vertical_wave = if intense {
        rng.range_f32(0.6, 1.8)
    } else {
        rng.range_f32(0.3, 1.1)
    };
    let ramp_span = if intense {
        rng.range_f32(9.6, 13.8)
    } else {
        rng.range_f32(7.6, 10.6)
    };
    let ramp_drop = if intense {
        rng.range_f32(16.0, 24.0)
    } else {
        rng.range_f32(12.0, 18.0)
    };
    let ridge_lift = if intense {
        rng.range_f32(3.8, 5.8)
    } else {
        rng.range_f32(2.6, 4.2)
    };

    let centerline_point = |t: f32| {
        let envelope = (t * PI).sin().max(0.0).powf(0.85);
        let offset =
            path_lateral_offset(style, t, envelope, phase, weave_cycles, lateral_amplitude);
        let lift = ridge_lift * envelope + (t * TAU * 1.1 + phase * 0.7).sin() * vertical_wave;
        surf_start.lerp(surf_end, t) + right * offset + Vec3::Y * lift
    };

    let mut segment_count = if intense {
        ((total_distance / 4.6).ceil() as usize).clamp(44, 132)
    } else {
        ((total_distance / 5.2).ceil() as usize).clamp(36, 116)
    };
    let fallback_tangent = (surf_end - surf_start).normalize_or_zero();
    let fallback_tangent = if fallback_tangent == Vec3::ZERO {
        forward
    } else {
        fallback_tangent
    };
    let mut centerline = Vec::new();

    loop {
        centerline.clear();
        centerline.reserve(segment_count + 1);
        for sample in 0..=segment_count {
            let t = sample as f32 / segment_count as f32;
            centerline.push(centerline_point(t));
        }

        let seam_tangents = sample_curve_tangents(&centerline, fallback_tangent);
        let mut max_segment_length: f32 = 0.0;
        let mut max_turn: f32 = 0.0;
        for index in 0..segment_count {
            max_segment_length =
                max_segment_length.max(centerline[index].distance(centerline[index + 1]));
            max_turn = max_turn.max(seam_tangents[index].angle_between(seam_tangents[index + 1]));
        }

        if (max_segment_length <= SURF_MAX_SEGMENT_LENGTH && max_turn <= SURF_MAX_SEAM_TURN_RADIANS)
            || segment_count >= SURF_MAX_RENDER_SEGMENTS
        {
            break;
        }

        let length_scale = (max_segment_length / SURF_MAX_SEGMENT_LENGTH).ceil() as usize;
        let turn_scale = (max_turn / SURF_MAX_SEAM_TURN_RADIANS).ceil() as usize;
        let scale = length_scale.max(turn_scale).clamp(2, 4);
        segment_count = (segment_count * scale).min(SURF_MAX_RENDER_SEGMENTS);
    }

    let seam_tangents = sample_curve_tangents(&centerline, fallback_tangent);
    let outward_hint = if right == Vec3::ZERO {
        perpendicular_to(seam_tangents[0], Vec3::X)
    } else {
        right.normalize_or_zero()
    };
    let seam_outwards = sample_curve_outwards(&seam_tangents, outward_hint);

    for index in 0..segment_count {
        let section_start = centerline[index];
        let section_end = centerline[index + 1];
        for side in [-1.0_f32, 1.0] {
            let wedge = surf_wedge_from_seams(
                section_start,
                section_end,
                seam_tangents[index],
                seam_tangents[index + 1],
                seam_outwards[index] * side,
                seam_outwards[index + 1] * side,
                ramp_span,
                ramp_drop,
            );
            layout.solids.push(SolidSpec {
                label: format!("Surf Wedge {index} {side}"),
                center: wedge.center,
                size: wedge.bounds,
                paint: PaintStyle::SurfRamp,
                body: SolidBody::StaticSurfWedge {
                    wall_side: side,
                    render_points: wedge.render_points,
                },
                friction: Some(0.0),
            });
        }
    }

    let collider_segment_count = if intense {
        ((total_distance / SURF_COLLIDER_SAMPLE_LENGTH).ceil() as usize)
            .clamp(segment_count, SURF_MAX_COLLIDER_SEGMENTS)
    } else {
        ((total_distance / (SURF_COLLIDER_SAMPLE_LENGTH * 1.12)).ceil() as usize)
            .clamp(segment_count, SURF_MAX_COLLIDER_SEGMENTS)
    };
    let mut collider_centerline = Vec::with_capacity(collider_segment_count + 1);
    for sample in 0..=collider_segment_count {
        let t = sample as f32 / collider_segment_count as f32;
        collider_centerline.push(centerline_point(t));
    }
    let collider_tangents = sample_curve_tangents(&collider_centerline, fallback_tangent);
    let collider_outwards = sample_curve_outwards(&collider_tangents, outward_hint);

    for side in [-1.0_f32, 1.0] {
        let strip = surf_strip_from_path(
            &collider_centerline,
            &collider_tangents,
            &collider_outwards,
            side,
            ramp_span,
            ramp_drop,
        );
        layout.solids.push(SolidSpec {
            label: format!("Surf Strip Collider {side}"),
            center: strip.center,
            size: strip.bounds,
            paint: PaintStyle::SurfRamp,
            body: SolidBody::StaticSurfStrip {
                wall_side: side,
                collider_strip_points: strip.collider_strip_points,
                columns: SURF_COLLIDER_COLUMNS,
            },
            friction: Some(0.0),
        });
    }
}

fn sample_descending_platform_tops(
    start: Vec3,
    end: Vec3,
    right: Vec3,
    count: usize,
    style: PathLateralStyle,
    weave_cycles: f32,
    phase: f32,
    lateral_amplitude: f32,
    vertical_wave: f32,
) -> Vec<Vec3> {
    if count == 0 {
        return Vec::new();
    }

    let mut points = Vec::with_capacity(count);
    let last = count.saturating_sub(1);
    for step in 0..count {
        let t = if count == 1 {
            0.5
        } else {
            step as f32 / last as f32
        };
        let envelope = (t * PI).sin().max(0.0).powf(0.72);
        let endpoint_factor = match step.min(last - step) {
            0 => 0.0,
            1 => 0.22,
            _ => 1.0,
        };
        let lateral = path_lateral_offset(
            style,
            t,
            envelope * endpoint_factor,
            phase,
            weave_cycles,
            lateral_amplitude,
        );
        let vertical =
            (t * TAU * 1.35 + phase * 0.7).sin() * vertical_wave * envelope * endpoint_factor;
        points.push(start.lerp(end, t) + right * lateral + Vec3::Y * vertical);
    }
    points
}

fn choose_bhop_path_style(rng: &mut RunRng) -> PathLateralStyle {
    rng.weighted_choice(&[
        (PathLateralStyle::Straight, 2),
        (PathLateralStyle::Serpentine, 6),
        (PathLateralStyle::Switchback, 6),
        (PathLateralStyle::Arc, 4),
    ])
}

fn choose_surf_path_style(rng: &mut RunRng, intense: bool) -> PathLateralStyle {
    rng.weighted_choice(&[
        (PathLateralStyle::Straight, if intense { 1 } else { 2 }),
        (PathLateralStyle::OneSidedArc, if intense { 8 } else { 7 }),
        (PathLateralStyle::Arc, 4),
    ])
}

fn path_lateral_offset(
    style: PathLateralStyle,
    t: f32,
    envelope: f32,
    phase: f32,
    weave_cycles: f32,
    amplitude: f32,
) -> f32 {
    let wave = (t * weave_cycles * TAU + phase).sin();
    let arc = (t * 2.0 - 1.0) + (phase * 0.35).sin() * 0.24;
    let scalar = match style {
        PathLateralStyle::Straight => (wave * 0.2 + arc * 0.18).clamp(-0.36, 0.36),
        PathLateralStyle::Serpentine => wave * 1.12,
        PathLateralStyle::Switchback => wave.signum() * wave.abs().powf(0.18) * 1.08,
        PathLateralStyle::Arc => (arc * 1.08 + wave * 0.24).clamp(-1.25, 1.25),
        PathLateralStyle::OneSidedArc => {
            let direction = if phase.sin() >= 0.0 { 1.0 } else { -1.0 };
            let bias = (phase.cos() * 0.5 + 0.5).clamp(0.18, 0.82);
            let lead = t.powf(lerp(0.72, 1.5, bias));
            let trail = (1.0 - t).powf(lerp(1.5, 0.72, bias));
            let hump = (lead * trail).powf(0.4) * 2.18;
            direction * hump.clamp(0.0, 1.22)
        }
    };
    scalar * amplitude * envelope
}

fn scaled_bhop_size(value: f32) -> f32 {
    value * BHOP_OBJECT_SCALE
}

fn scaled_bhop_cadence(min: f32, max: f32, rng: &mut RunRng) -> f32 {
    rng.range_f32(min * BHOP_CADENCE_SCALE, max * BHOP_CADENCE_SCALE)
}

fn bhop_path_margin(distance: f32, min: f32, max: f32) -> f32 {
    (distance * 0.0045).clamp(min, max)
}

fn clamp_platform_count_for_spacing(
    distance: f32,
    requested_count: usize,
    min_spacing: f32,
    minimum_count: usize,
) -> usize {
    let max_by_spacing = ((distance / min_spacing.max(1.0)).floor() as usize).saturating_add(1);
    requested_count
        .min(max_by_spacing.max(minimum_count))
        .max(minimum_count)
}

#[derive(Default)]
struct ColoredMeshBuilder {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
}

impl ColoredMeshBuilder {
    fn push_triangle(&mut self, a: Vec3, b: Vec3, c: Vec3, color: Color) {
        let normal = (b - a).cross(c - a).normalize_or_zero();
        let normal = if normal == Vec3::ZERO {
            Vec3::Y
        } else {
            normal
        };
        let color = LinearRgba::from(color).to_f32_array();
        for point in [a, b, c] {
            self.positions.push([point.x, point.y, point.z]);
            self.normals.push([normal.x, normal.y, normal.z]);
            self.colors.push(color);
        }
    }

    fn push_quad(&mut self, a: Vec3, b: Vec3, c: Vec3, d: Vec3, color: Color) {
        self.push_triangle(a, b, c, color);
        self.push_triangle(a, c, d, color);
    }

    fn build(self) -> Mesh {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colors)
    }
}

fn append_surf_wedge_render_geometry(
    builder: &mut ColoredMeshBuilder,
    center: Vec3,
    local_points: &[Vec3],
    base_color: Color,
    stripe_color: Color,
) {
    if local_points.len() < 8 {
        return;
    }

    let points = local_points
        .iter()
        .map(|point| center + *point)
        .collect::<Vec<_>>();
    let a = points[0];
    let b = points[1];
    let c = points[2];
    let d = points[3];
    let e = points[4];
    let f = points[5];
    let g = points[6];
    let h = points[7];

    let ridge_face = brighten(base_color, 0.14);
    let outer_face = brighten(base_color, 0.08);
    let underside = deepen(base_color, 0.28);
    let side_shadow = deepen(base_color, 0.18);

    builder.push_quad(e, g, h, f, underside);
    builder.push_quad(a, c, g, e, outer_face);
    builder.push_quad(b, f, h, d, side_shadow);
    builder.push_quad(a, e, f, b, deepen(base_color, 0.08));
    builder.push_quad(c, d, h, g, deepen(base_color, 0.14));

    let front_ridge = a;
    let back_ridge = b;
    let front_outer = c;
    let back_outer = d;

    let stripe_core = mix_color(ridge_face, dim_linear(stripe_color, 0.78), 0.82);
    let stripe_glow = mix_color(ridge_face, dim_linear(stripe_color, 0.52), 0.52);
    let stripe_specs: [(f32, f32, f32); 2] = [(0.18, 0.16, 0.32), (0.64, 0.11, 0.22)];
    let mut band_breaks = vec![0.0_f32, 1.0_f32];
    for &(t, stripe_width, glow_width) in &stripe_specs {
        band_breaks.push((t - glow_width * 0.5_f32).clamp(0.0_f32, 1.0_f32));
        band_breaks.push((t - stripe_width * 0.5_f32).clamp(0.0_f32, 1.0_f32));
        band_breaks.push((t + stripe_width * 0.5_f32).clamp(0.0_f32, 1.0_f32));
        band_breaks.push((t + glow_width * 0.5_f32).clamp(0.0_f32, 1.0_f32));
    }
    band_breaks.sort_by(|a, b| a.total_cmp(b));
    band_breaks.dedup_by(|a, b| (*a - *b).abs() < 0.001);

    for pair in band_breaks.windows(2) {
        let start_t = pair[0];
        let end_t = pair[1];
        if end_t - start_t < 0.001 {
            continue;
        }
        let mid_t = (start_t + end_t) * 0.5;
        let band_color = surf_wedge_stripe_band_color(
            mid_t,
            ridge_face,
            stripe_glow,
            stripe_core,
            &stripe_specs,
        );
        builder.push_quad(
            front_ridge.lerp(front_outer, start_t),
            back_ridge.lerp(back_outer, start_t),
            back_ridge.lerp(back_outer, end_t),
            front_ridge.lerp(front_outer, end_t),
            band_color,
        );
    }
}

fn surf_wedge_stripe_band_color(
    t: f32,
    base_color: Color,
    glow_color: Color,
    stripe_color: Color,
    stripe_specs: &[(f32, f32, f32)],
) -> Color {
    for &(stripe_t, stripe_width, glow_width) in stripe_specs {
        let stripe_min = stripe_t - stripe_width * 0.5;
        let stripe_max = stripe_t + stripe_width * 0.5;
        if (stripe_min..=stripe_max).contains(&t) {
            return stripe_color;
        }

        let glow_min = stripe_t - glow_width * 0.5;
        let glow_max = stripe_t + glow_width * 0.5;
        if (glow_min..=glow_max).contains(&t) {
            return glow_color;
        }
    }

    base_color
}

fn build_surf_wedge_mesh(points: &[Vec3], base_color: Color, stripe_color: Color) -> Mesh {
    let mut builder = ColoredMeshBuilder::default();
    append_surf_wedge_render_geometry(&mut builder, Vec3::ZERO, points, base_color, stripe_color);
    builder.build()
}

fn build_surf_strip_collider_mesh(points: &[Vec3], columns: usize) -> Option<Mesh> {
    if columns < 2 || points.len() < columns * 2 || points.len() % columns != 0 {
        return None;
    }

    let seam_count = points.len() / columns;
    let mut indices = Vec::with_capacity((seam_count - 1) * (columns - 1) * 6);
    for seam in 0..(seam_count - 1) {
        for column in 0..(columns - 1) {
            let a = (seam * columns + column) as u32;
            let b = a + 1;
            let c = ((seam + 1) * columns + column) as u32;
            let d = c + 1;
            let split_across = triangle_pair_alignment(
                points[a as usize],
                points[b as usize],
                points[c as usize],
                points[d as usize],
            );
            if split_across {
                indices.extend_from_slice(&[a, b, d, a, d, c]);
            } else {
                indices.extend_from_slice(&[a, b, c, b, d, c]);
            }
        }
    }

    Some(
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            points
                .iter()
                .map(|point| [point.x, point.y, point.z])
                .collect::<Vec<_>>(),
        )
        .with_inserted_indices(Indices::U32(indices)),
    )
}

fn triangle_pair_alignment(a: Vec3, b: Vec3, c: Vec3, d: Vec3) -> bool {
    let split_abdc_1 = (b - a).cross(d - a).normalize_or_zero();
    let split_abdc_2 = (d - a).cross(c - a).normalize_or_zero();
    let split_abcd_1 = (b - a).cross(c - a).normalize_or_zero();
    let split_abcd_2 = (d - b).cross(c - b).normalize_or_zero();

    split_abdc_1.dot(split_abdc_2) >= split_abcd_1.dot(split_abcd_2)
}

struct SurfWedgeGeometry {
    center: Vec3,
    render_points: Vec<Vec3>,
    bounds: Vec3,
}

struct SurfStripGeometry {
    center: Vec3,
    collider_strip_points: Vec<Vec3>,
    bounds: Vec3,
}

fn project_onto_plane(vector: Vec3, normal: Vec3) -> Vec3 {
    vector - normal * vector.dot(normal)
}

fn sample_curve_tangents(points: &[Vec3], fallback_tangent: Vec3) -> Vec<Vec3> {
    let mut tangents = Vec::with_capacity(points.len());
    for index in 0..points.len() {
        let prev = if index == 0 {
            points[1] - points[0]
        } else {
            points[index] - points[index - 1]
        };
        let next = if index + 1 == points.len() {
            points[index] - points[index - 1]
        } else {
            points[index + 1] - points[index]
        };
        let tangent = (prev + next).normalize_or_zero();
        tangents.push(if tangent == Vec3::ZERO {
            fallback_tangent
        } else {
            tangent
        });
    }
    tangents
}

fn sample_curve_outwards(tangents: &[Vec3], outward_hint: Vec3) -> Vec<Vec3> {
    let mut outwards = Vec::with_capacity(tangents.len());
    let mut previous_outward = project_onto_plane(outward_hint, tangents[0]).normalize_or_zero();
    if previous_outward == Vec3::ZERO {
        previous_outward = perpendicular_to(tangents[0], outward_hint);
    }
    if previous_outward.dot(outward_hint) < 0.0 {
        previous_outward = -previous_outward;
    }
    outwards.push(previous_outward);

    for tangent in tangents.iter().skip(1) {
        let hint = project_onto_plane(outward_hint, *tangent).normalize_or_zero();
        let mut outward = project_onto_plane(previous_outward, *tangent).normalize_or_zero();
        if outward == Vec3::ZERO {
            outward = if hint != Vec3::ZERO {
                hint
            } else {
                perpendicular_to(*tangent, outward_hint)
            };
        }
        if hint != Vec3::ZERO {
            let aligned_hint = if outward.dot(hint) < 0.0 { -hint } else { hint };
            outward = outward.lerp(aligned_hint, 0.16).normalize_or_zero();
        }
        if outward == Vec3::ZERO {
            outward = perpendicular_to(*tangent, outward_hint);
        }
        if outward.dot(outward_hint) < 0.0 {
            outward = -outward;
        }
        outwards.push(outward);
        previous_outward = outward;
    }

    outwards
}

fn perpendicular_to(normal: Vec3, hint: Vec3) -> Vec3 {
    let mut perpendicular = project_onto_plane(hint, normal).normalize_or_zero();
    if perpendicular == Vec3::ZERO {
        perpendicular = if normal.y.abs() < 0.95 {
            normal.cross(Vec3::Y).normalize_or_zero()
        } else {
            normal.cross(Vec3::X).normalize_or_zero()
        };
    }
    if perpendicular == Vec3::ZERO {
        Vec3::X
    } else {
        perpendicular
    }
}

fn surf_face_offset(tangent: Vec3, outward_hint: Vec3, ramp_span: f32, ramp_drop: f32) -> Vec3 {
    let tangent = tangent.normalize_or_zero();
    let mut outward = project_onto_plane(outward_hint, tangent).normalize_or_zero();
    if outward == Vec3::ZERO {
        outward = perpendicular_to(tangent, outward_hint);
    }
    if outward.dot(outward_hint) < 0.0 {
        outward = -outward;
    }
    let mut down = tangent.cross(outward).normalize_or_zero();
    if down.y > 0.0 {
        down = -down;
    }
    if down == Vec3::ZERO {
        down = -Vec3::Y;
    }

    outward * ramp_span + down * ramp_drop
}

fn wedge_local_points(
    start_ridge: Vec3,
    end_ridge: Vec3,
    start_outer: Vec3,
    end_outer: Vec3,
    thickness: f32,
) -> [Vec3; 8] {
    [
        start_ridge,
        end_ridge,
        start_outer,
        end_outer,
        start_ridge - Vec3::Y * thickness,
        end_ridge - Vec3::Y * thickness,
        start_outer - Vec3::Y * thickness,
        end_outer - Vec3::Y * thickness,
    ]
}

fn surf_wedge_from_seams(
    start_ridge: Vec3,
    end_ridge: Vec3,
    start_tangent: Vec3,
    end_tangent: Vec3,
    start_outward: Vec3,
    end_outward: Vec3,
    ramp_span: f32,
    ramp_drop: f32,
) -> SurfWedgeGeometry {
    let start_face = surf_face_offset(start_tangent, start_outward, ramp_span, ramp_drop);
    let end_face = surf_face_offset(end_tangent, end_outward, ramp_span, ramp_drop);
    let render_world_points = wedge_local_points(
        start_ridge,
        end_ridge,
        start_ridge + start_face,
        end_ridge + end_face,
        SURF_WEDGE_THICKNESS,
    );

    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut center = Vec3::ZERO;
    for point in render_world_points.iter().copied() {
        min = min.min(point);
        max = max.max(point);
        center += point;
    }
    center /= render_world_points.len() as f32;

    SurfWedgeGeometry {
        center,
        render_points: render_world_points
            .into_iter()
            .map(|point| point - center)
            .collect(),
        bounds: max - min,
    }
}

fn surf_strip_from_path(
    centerline: &[Vec3],
    seam_tangents: &[Vec3],
    seam_outwards: &[Vec3],
    side: f32,
    ramp_span: f32,
    ramp_drop: f32,
) -> SurfStripGeometry {
    let mut collider_world_points = Vec::with_capacity(centerline.len() * SURF_COLLIDER_COLUMNS);
    let start_overlap = if centerline.len() > 1 {
        (centerline[0].distance(centerline[1]) * 0.08)
            .clamp(SURF_COLLIDER_OVERLAP_MIN, SURF_COLLIDER_OVERLAP_MAX)
    } else {
        SURF_COLLIDER_OVERLAP_MIN
    };
    let end_overlap = if centerline.len() > 1 {
        (centerline[centerline.len() - 2].distance(centerline[centerline.len() - 1]) * 0.08)
            .clamp(SURF_COLLIDER_OVERLAP_MIN, SURF_COLLIDER_OVERLAP_MAX)
    } else {
        SURF_COLLIDER_OVERLAP_MIN
    };

    for index in 0..centerline.len() {
        let mut ridge = centerline[index];
        let tangent = seam_tangents[index].normalize_or_zero();
        if index == 0 {
            ridge -= tangent * start_overlap;
        } else if index + 1 == centerline.len() {
            ridge += tangent * end_overlap;
        }
        let face = surf_face_offset(
            seam_tangents[index],
            seam_outwards[index] * side,
            ramp_span,
            ramp_drop,
        );
        let outer = ridge + face;
        for column in 0..SURF_COLLIDER_COLUMNS {
            let t = column as f32 / (SURF_COLLIDER_COLUMNS - 1) as f32;
            collider_world_points.push(ridge.lerp(outer, t));
        }
    }

    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut center = Vec3::ZERO;
    for point in collider_world_points.iter().copied() {
        min = min.min(point);
        max = max.max(point);
        center += point;
    }
    center /= collider_world_points.len().max(1) as f32;

    SurfStripGeometry {
        center,
        collider_strip_points: collider_world_points
            .into_iter()
            .map(|point| point - center)
            .collect(),
        bounds: max - min,
    }
}

fn validate_solid_spec(spec: &SolidSpec) -> Result<(), String> {
    if !spec.center.is_finite() {
        return Err(format!("non-finite center {:?}", spec.center));
    }
    if !spec.size.is_finite() || spec.size.min_element() <= 0.0 {
        return Err(format!("non-finite or non-positive size {:?}", spec.size));
    }

    match &spec.body {
        SolidBody::Static => {
            let aabb = Collider::cuboid(spec.size.x, spec.size.y, spec.size.z)
                .aabb(spec.center, Quat::IDENTITY);
            if !aabb.min.is_finite() || !aabb.max.is_finite() {
                return Err("invalid cuboid collider bounds".into());
            }
        }
        SolidBody::StaticSurfWedge { render_points, .. } => {
            if render_points.len() < 8 {
                return Err("surf wedge did not have enough render points".into());
            }
            if render_points.iter().any(|point| !point.is_finite()) {
                return Err("surf wedge contained non-finite render points".into());
            }
        }
        SolidBody::StaticSurfStrip {
            collider_strip_points,
            columns,
            ..
        } => {
            let Some(mesh) = build_surf_strip_collider_mesh(collider_strip_points, *columns) else {
                return Err("surf strip collider mesh generation failed".into());
            };
            let Some(collider) =
                Collider::trimesh_from_mesh_with_config(&mesh, TrimeshFlags::FIX_INTERNAL_EDGES)
            else {
                return Err("surf strip collider generation failed".into());
            };
            let aabb = collider.aabb(spec.center, Quat::IDENTITY);
            if !aabb.min.is_finite() || !aabb.max.is_finite() {
                return Err("invalid surf strip collider bounds".into());
            }
        }
    }

    Ok(())
}

fn direction_from_delta(delta: Vec3) -> Vec3 {
    let forward = Vec3::new(delta.x, 0.0, delta.z).normalize_or_zero();
    if forward == Vec3::ZERO {
        Vec3::X
    } else {
        forward
    }
}

fn top_to_center(top: Vec3, height: f32) -> Vec3 {
    Vec3::new(top.x, top.y - height * 0.5, top.z)
}

fn mix_color(from: Color, to: Color, amount: f32) -> Color {
    Color::from(LinearRgba::from(from).mix(&LinearRgba::from(to), amount.clamp(0.0, 1.0)))
}

fn brighten(color: Color, amount: f32) -> Color {
    mix_color(color, Color::WHITE, amount)
}

fn deepen(color: Color, amount: f32) -> Color {
    mix_color(color, Color::BLACK, amount)
}

fn dim_linear(color: Color, scale: f32) -> Color {
    let linear = LinearRgba::from(color);
    Color::from(LinearRgba::new(
        linear.red * scale,
        linear.green * scale,
        linear.blue * scale,
        1.0,
    ))
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

#[derive(Clone)]
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn test_room(index: usize, top: Vec3, checkpoint_slot: Option<usize>) -> RoomPlan {
        RoomPlan {
            index,
            top,
            checkpoint_slot,
        }
    }

    fn test_segment(index: usize, kind: SegmentKind, seed: u64) -> SegmentPlan {
        SegmentPlan {
            index,
            from: 0,
            to: 1,
            kind,
            seed,
        }
    }

    fn test_fall_snapshot(position: Vec3, velocity: Vec3, fallback_forward: Vec3) -> FallSnapshot {
        FallSnapshot::new(position, velocity, fallback_forward)
    }

    #[test]
    fn blueprint_only_generates_surf_and_square_bhop_sections() {
        let blueprint = build_run_blueprint(42);

        assert!(blueprint.rooms.len() >= 2);
        assert_eq!(blueprint.segments.len(), blueprint.rooms.len() - 1);
        assert_eq!(
            blueprint.segments.first().map(|segment| segment.kind),
            Some(SegmentKind::SquareBhop)
        );
        assert!(blueprint.segments.iter().all(|segment| matches!(
            segment.kind,
            SegmentKind::SurfRamp | SegmentKind::SquareBhop
        )));
        assert!(
            blueprint
                .segments
                .iter()
                .any(|segment| segment.kind == SegmentKind::SquareBhop)
        );
        assert!(
            blueprint
                .segments
                .iter()
                .any(|segment| segment.kind == SegmentKind::SurfRamp)
        );
    }

    #[test]
    fn initial_blueprint_spawns_on_first_bhop_platform() {
        let blueprint = build_run_blueprint(0xA11C_E123);
        let first_segment = blueprint
            .segments
            .first()
            .expect("initial blueprint should have a first segment");
        assert_eq!(first_segment.kind, SegmentKind::SquareBhop);

        let layout = build_segment_layout(first_segment, &blueprint.rooms);
        let first_platform = layout
            .solids
            .first()
            .expect("first bhop segment should emit platforms");
        let platform_top = first_platform.center + Vec3::Y * (first_platform.size.y * 0.5);

        assert_eq!(
            blueprint.spawn,
            platform_top + Vec3::new(0.0, PLAYER_SPAWN_CLEARANCE, 0.0)
        );
    }

    #[test]
    fn rollover_blueprint_starts_below_fall_snapshot() {
        let snapshot = test_fall_snapshot(
            Vec3::new(180.0, -24.0, 32.0),
            Vec3::new(18.0, -58.0, 6.0),
            Vec3::Z,
        );
        let blueprint = build_rollover_blueprint(0xCAFE_BABE, &snapshot);

        assert!(blueprint.rooms[0].top.y <= snapshot.position.y - ROLLOVER_CATCH_DROP_MIN);
        assert_eq!(blueprint.rooms[0].checkpoint_slot, Some(0));
    }

    #[test]
    fn rollover_blueprint_first_segment_is_surf_and_append_stays_minimal() {
        let snapshot = test_fall_snapshot(
            Vec3::new(0.0, 16.0, 0.0),
            Vec3::new(22.0, -44.0, 4.0),
            Vec3::X,
        );
        let mut blueprint = build_rollover_blueprint(0x1234_ABCD, &snapshot);
        append_run_blueprint(&mut blueprint, 6);

        assert_eq!(
            blueprint.segments.first().map(|segment| segment.kind),
            Some(SegmentKind::SurfRamp)
        );
        assert!(blueprint.segments.iter().all(|segment| matches!(
            segment.kind,
            SegmentKind::SurfRamp | SegmentKind::SquareBhop
        )));
        assert!(
            blueprint
                .segments
                .iter()
                .any(|segment| segment.kind == SegmentKind::SquareBhop)
        );
    }

    #[test]
    fn rollover_blueprint_preserves_forward_continuity() {
        let snapshot = test_fall_snapshot(
            Vec3::new(-80.0, 42.0, 24.0),
            Vec3::new(28.0, -50.0, 2.0),
            Vec3::new(1.0, 0.0, 0.0),
        );
        let blueprint = build_rollover_blueprint(0x5EED, &snapshot);
        let first_forward = direction_from_delta(blueprint.rooms[1].top - blueprint.rooms[0].top);

        assert!(
            first_forward.dot(snapshot.forward) > 0.98,
            "expected rollover path to follow fall direction, got {:?} vs {:?}",
            first_forward,
            snapshot.forward
        );
    }

    #[test]
    fn rollover_state_replaces_old_course_epoch_without_old_state() {
        let mut run = RunState::new(build_run_blueprint(7));
        run.current_checkpoint = 3;
        run.deaths = 4;
        run.timer.tick(Duration::from_secs_f32(18.0));
        let previous_elapsed = run.timer.elapsed_secs();

        let snapshot = test_fall_snapshot(
            Vec3::new(240.0, -80.0, 10.0),
            Vec3::new(14.0, -62.0, -3.0),
            Vec3::new(1.0, 0.0, 0.0),
        );
        let next_epoch = run.reserve_epoch();
        let blueprint = build_rollover_blueprint(0xBADC_0FFE, &snapshot);
        let expected_spawn = blueprint.checkpoint_position(0);
        let expected_death_plane = checkpoint_death_plane(&blueprint, 0);

        run.apply_rollover_blueprint(blueprint, next_epoch);

        assert_eq!(run.epoch, next_epoch);
        assert_eq!(run.current_checkpoint, 0);
        assert_eq!(run.checkpoint_position(0), expected_spawn);
        assert_eq!(run.death_plane, expected_death_plane);
        assert_eq!(run.deaths, 4);
        assert!((run.timer.elapsed_secs() - previous_elapsed).abs() < 0.001);
    }

    #[test]
    fn room_layout_keeps_only_checkpoint_feature() {
        let room = test_room(3, Vec3::new(0.0, 32.0, 0.0), Some(2));
        let layout = build_room_layout(&room);

        assert!(layout.solids.is_empty());
        assert!(matches!(
            layout.features.as_slice(),
            [FeatureSpec::CheckpointPad { index: 2, .. }]
        ));
    }

    #[test]
    fn square_bhop_layout_emits_only_square_static_platforms() {
        let rooms = vec![
            test_room(0, Vec3::new(0.0, 120.0, 0.0), Some(0)),
            test_room(1, Vec3::new(92.0, 92.0, 18.0), Some(1)),
        ];
        let layout = build_segment_layout(
            &test_segment(0, SegmentKind::SquareBhop, 0xBEEF_CAFE),
            &rooms,
        );

        assert!(!layout.solids.is_empty());
        assert!(layout.features.is_empty());
        for solid in &layout.solids {
            assert!(matches!(solid.paint, PaintStyle::BhopPlatform));
            assert!(matches!(solid.body, SolidBody::Static));
            assert!(
                (solid.size.x - solid.size.z).abs() < 0.001,
                "bhop platform was not square: {:?}",
                solid.size
            );
        }
    }

    #[test]
    fn surf_layout_emits_only_wedges_and_collider_strips() {
        let rooms = vec![
            test_room(0, Vec3::new(0.0, 120.0, 0.0), Some(0)),
            test_room(1, Vec3::new(110.0, 94.0, 24.0), Some(1)),
        ];
        let layout =
            build_segment_layout(&test_segment(0, SegmentKind::SurfRamp, 0x1234_5678), &rooms);

        assert!(!layout.solids.is_empty());
        assert!(layout.features.is_empty());
        assert!(
            layout
                .solids
                .iter()
                .any(|solid| matches!(solid.body, SolidBody::StaticSurfWedge { .. }))
        );
        assert!(
            layout
                .solids
                .iter()
                .any(|solid| matches!(solid.body, SolidBody::StaticSurfStrip { .. }))
        );
        assert!(layout.solids.iter().all(|solid| matches!(
            solid.body,
            SolidBody::StaticSurfWedge { .. } | SolidBody::StaticSurfStrip { .. }
        )));
    }

    #[test]
    fn generated_solids_validate_across_multiple_seeds() {
        for seed in 0_u64..64 {
            let blueprint = build_run_blueprint(seed);

            for room in &blueprint.rooms {
                let layout = build_room_layout(room);
                for solid in &layout.solids {
                    assert!(
                        validate_solid_spec(solid).is_ok(),
                        "seed {seed} room {} solid '{}' invalid: {}",
                        room.index,
                        solid.label,
                        validate_solid_spec(solid).unwrap_err()
                    );
                }
            }

            for segment in &blueprint.segments {
                let layout = build_segment_layout(segment, &blueprint.rooms);
                for solid in &layout.solids {
                    assert!(
                        validate_solid_spec(solid).is_ok(),
                        "seed {seed} segment {} {:?} solid '{}' invalid: {}",
                        segment.index,
                        segment.kind,
                        solid.label,
                        validate_solid_spec(solid).unwrap_err()
                    );
                }
            }
        }
    }

    #[test]
    fn append_run_blueprint_adds_more_rooms_and_segments() {
        let mut blueprint = build_run_blueprint(7);
        let original_room_count = blueprint.rooms.len();
        let original_segment_count = blueprint.segments.len();
        let original_checkpoint_count = blueprint.checkpoint_count();
        let original_tail_y = blueprint.rooms.last().unwrap().top.y;

        append_run_blueprint(&mut blueprint, 6);

        assert_eq!(blueprint.rooms.len(), original_room_count + 6);
        assert_eq!(blueprint.segments.len(), original_segment_count + 6);
        assert_eq!(blueprint.segments.len(), blueprint.rooms.len() - 1);
        assert!(blueprint.checkpoint_count() > original_checkpoint_count);
        assert!(blueprint.rooms.last().unwrap().top.y < original_tail_y);
        assert_eq!(
            blueprint.segments[original_segment_count].from,
            original_room_count - 1
        );
    }

    #[test]
    fn checkpoint_death_plane_looks_ahead_downstream() {
        let blueprint = RunBlueprint {
            seed: 7,
            rooms: vec![
                test_room(0, Vec3::new(0.0, 420.0, 0.0), Some(0)),
                test_room(1, Vec3::new(120.0, 320.0, 0.0), None),
                test_room(2, Vec3::new(240.0, 180.0, 0.0), None),
                test_room(3, Vec3::new(360.0, 40.0, 0.0), Some(1)),
            ],
            segments: vec![
                test_segment(0, SegmentKind::SurfRamp, 1),
                test_segment(1, SegmentKind::SquareBhop, 2),
                test_segment(2, SegmentKind::SurfRamp, 3),
            ],
            spawn: Vec3::new(0.0, 422.0, 0.0),
            tail_forward: Vec3::X,
            next_segment_kind: SegmentKind::SquareBhop,
            next_checkpoint_slot: 2,
            generator: RunRng::new(7),
        };

        let death_plane = checkpoint_death_plane(&blueprint, 0);
        assert!(
            death_plane <= blueprint.rooms[3].top.y - CHECKPOINT_DEATH_MARGIN,
            "death plane {} did not look ahead to the deeper room",
            death_plane
        );
    }

    #[test]
    fn surf_material_is_opaque_and_double_sided() {
        let mut cache = WorldAssetCache::default();
        let mut materials = Assets::<StandardMaterial>::default();
        let handle = cached_material(&mut cache, &mut materials, MaterialKey::SurfVertex);
        let material = materials
            .get(&handle)
            .expect("cached surf material should exist");

        assert!(matches!(material.alpha_mode, AlphaMode::Opaque));
        assert_eq!(material.base_color.alpha(), 1.0);
        assert!(material.clearcoat >= 0.85);
        assert!(material.cull_mode.is_none());
    }

    #[test]
    fn surf_wedge_render_restores_internal_stripe_geometry() {
        let points = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(6.0, 0.0, 0.0),
            Vec3::new(0.0, -1.2, 4.0),
            Vec3::new(6.0, -1.2, 4.0),
            Vec3::new(0.0, -0.24, 0.0),
            Vec3::new(6.0, -0.24, 0.0),
            Vec3::new(0.0, -1.44, 4.0),
            Vec3::new(6.0, -1.44, 4.0),
        ];
        let mut builder = ColoredMeshBuilder::default();
        append_surf_wedge_render_geometry(
            &mut builder,
            Vec3::ZERO,
            &points,
            Color::srgb(0.2, 0.3, 0.5),
            Color::srgb(0.9, 0.9, 1.0),
        );

        assert_eq!(builder.positions.len(), 84);
    }

    #[test]
    fn surf_collider_strip_extends_past_render_seams() {
        let rooms = vec![
            test_room(0, Vec3::new(0.0, 40.0, 0.0), Some(0)),
            test_room(1, Vec3::new(120.0, 18.0, 26.0), Some(1)),
        ];
        let layout = build_segment_layout(
            &test_segment(0, SegmentKind::SurfRamp, 0x5eed_cafe_u64),
            &rooms,
        );

        let render_wedges = layout
            .solids
            .iter()
            .filter_map(|solid| match &solid.body {
                SolidBody::StaticSurfWedge {
                    wall_side,
                    render_points,
                } if *wall_side > 0.0 && !render_points.is_empty() => {
                    Some((solid.center, render_points))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let strips = layout
            .solids
            .iter()
            .filter_map(|solid| match &solid.body {
                SolidBody::StaticSurfStrip {
                    wall_side,
                    collider_strip_points,
                    columns,
                } if *wall_side > 0.0 && !collider_strip_points.is_empty() => {
                    Some((solid.center, collider_strip_points, *columns))
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        let render_start_ridge =
            render_wedges.first().unwrap().0 + render_wedges.first().unwrap().1[0];
        let render_end_ridge = render_wedges.last().unwrap().0 + render_wedges.last().unwrap().1[1];
        let (strip_center, strip_points, columns) = &strips[0];
        let strip_start_ridge = *strip_center + strip_points[0];
        let strip_end_ridge = *strip_center + strip_points[strip_points.len() - *columns];
        let start_dir =
            (render_wedges[0].0 + render_wedges[0].1[1] - render_start_ridge).normalize_or_zero();
        let end_dir = (render_end_ridge
            - (render_wedges[render_wedges.len() - 1].0
                + render_wedges[render_wedges.len() - 1].1[0]))
            .normalize_or_zero();

        assert!(
            (render_start_ridge - strip_start_ridge).dot(start_dir)
                > SURF_COLLIDER_OVERLAP_MIN * 0.75
        );
        assert!(
            (strip_end_ridge - render_end_ridge).dot(end_dir) > SURF_COLLIDER_OVERLAP_MIN * 0.75
        );
    }
}
