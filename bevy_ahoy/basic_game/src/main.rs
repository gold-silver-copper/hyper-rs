use std::{
    collections::{HashMap, HashSet},
    f32::consts::{PI, TAU},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use avian3d::prelude::*;
use bevy::{
    anti_alias::fxaa::Fxaa,
    asset::RenderAssetUsages,
    color::palettes::tailwind,
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    input::common_conditions::input_just_pressed,
    light::{CascadeShadowConfigBuilder, NotShadowCaster, NotShadowReceiver},
    math::primitives::{
        Capsule3d, Cone, ConicalFrustum, Cuboid, Cylinder, Sphere, Tetrahedron, Torus,
    },
    mesh::Indices,
    post_process::bloom::Bloom,
    prelude::*,
    render::render_resource::PrimitiveTopology,
    render::view::{ColorGrading, ColorGradingGlobal, ColorGradingSection, Hdr},
    window::{CursorGrabMode, CursorOptions, WindowResolution},
};
use bevy_ahoy::{
    CharacterControllerOutput, CharacterLook, PickupConfig, PickupHoldConfig, PickupPullConfig,
    input::AccumulatedInput, prelude::*,
};
use bevy_ecs::{lifecycle::HookContext, world::DeferredWorld};
use bevy_enhanced_input::prelude::{Hold, Press, *};
use bevy_time::Stopwatch;

use crate::util::{ControlsOverlay, ExampleUtilPlugin, StableGround};

mod util;

const ROOM_HEIGHT: f32 = 1.2;
const ROOM_CLEARANCE_HEIGHT: f32 = 3.4;
const CELL_SIZE: f32 = 22.0;
const ROOM_GRID_SIZE: f32 = 8.0;
const PLAYER_SPAWN_CLEARANCE: f32 = 2.4;
const WALL_RUN_SPEED: f32 = 11.5;
const WALL_RUN_STICK_SPEED: f32 = 2.0;
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
const WALL_RUN_FALL_SPEED: f32 = 2.25;
const WALL_RUN_MIN_SPEED: f32 = 4.0;
const WALL_RUN_DURATION: f32 = 0.95;
const WALL_RUN_COOLDOWN: f32 = 0.2;
const WALL_SHAFT_BOOST_SPEED: f32 = 8.8;
const WALL_SHAFT_REPEAT: f32 = 0.11;
const CHECKPOINT_RADIUS: f32 = 2.8;
const SUMMIT_RADIUS: f32 = 4.5;
const SKY_RADIUS: f32 = 950.0;
const STAR_SIZE_MULTIPLIER: f32 = 12.0;
const STAR_CLUSTER_SIZE_MULTIPLIER: f32 = 12.0;
const COMET_SIZE_MULTIPLIER: f32 = 12.0;
const MANUAL_MOON_SIZE_MULTIPLIER: f32 = 28.0;
const MAJOR_CELESTIAL_RADIUS_MULTIPLIER: f32 = 70.0;
const MOON_CELESTIAL_RADIUS_MULTIPLIER: f32 = 60.0;
const DECOR_CELESTIAL_RADIUS_MULTIPLIER: f32 = 50.0;
const MAX_SECTION_TURN_RADIANS: f32 = 22.0_f32.to_radians();
const STAR_COUNT: usize = 1100;
const STAR_CLUSTER_COUNT: usize = 6;
const COMET_COUNT: usize = 18;
const STREAM_BEHIND_ROOMS: usize = 2;
const STREAM_AHEAD_ROOMS: usize = 12;
const INFINITE_APPEND_TRIGGER_ROOMS: usize = 5;
const INFINITE_APPEND_BATCH_ROOMS: usize = 16;
const PHYSICS_SUBSTEPS: u32 = 12;
const PLAYER_GRAVITY: f32 = 29.0;
const PLAYER_STEP_SIZE: f32 = 1.0;
const PLAYER_GROUND_DISTANCE: f32 = 0.05;
const PLAYER_STEP_DOWN_DETECTION_DISTANCE: f32 = 0.2;
const PLAYER_SKIN_WIDTH: f32 = 0.008;
const SURF_STEP_SIZE: f32 = 0.0;
const SURF_GROUND_DISTANCE: f32 = 0.012;
const SURF_STEP_DOWN_DETECTION_DISTANCE: f32 = 0.03;
const SURF_SKIN_WIDTH: f32 = 0.003;
const BHOP_OBJECT_SCALE: f32 = 5.0;
const BHOP_CADENCE_SCALE: f32 = 4.1;
const PLAYER_MOVE_AND_SLIDE_ITERATIONS: usize = 8;
const PLAYER_DEPENETRATION_ITERATIONS: usize = 8;
const CHECKPOINT_DEATH_MARGIN: f32 = 180.0;
const ATMOSPHERE_PROGRESS_DEPTH: f32 = 1800.0;
const ROUTE_LINE_LATERAL_SPAN: f32 = 7.6;
const ROUTE_LINE_TRICK_VERTICAL_BIAS: f32 = 1.1;
const ROUTE_LINE_MIN_CORRIDOR_GAP: f32 = 4.2;
const ROUTE_LINE_EDGE_TAPER: f32 = 0.92;
const ENABLE_PARALLEL_ROUTE_GEOMETRY: bool = false;
const LANDMARK_OFFSET_RADIUS: f32 = 168.0;
const LANDMARK_VERTICAL_OFFSET: f32 = 78.0;
const ENABLE_MACRO_SPECTACLE: bool = false;

type SocketMask = u32;
const SOCKET_SAFE_REST: SocketMask = 1 << 0;
const SOCKET_MANTLE_ENTRY: SocketMask = 1 << 1;
const SOCKET_WALLRUN_READY: SocketMask = 1 << 2;
const SOCKET_HAZARD_BRANCH: SocketMask = 1 << 3;
const SOCKET_SHORTCUT_ANCHOR: SocketMask = 1 << 4;

#[derive(SystemSet, Debug, Clone, Copy, Hash, PartialEq, Eq)]
enum BasicGameUpdateSet {
    Input,
    Progress,
    Streaming,
    Presentation,
    Requests,
}

#[derive(SystemSet, Debug, Clone, Copy, Hash, PartialEq, Eq)]
enum BasicGameFixedSet {
    Environment,
}

struct BasicGamePlugin;

impl Plugin for BasicGamePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(Color::srgb(0.11, 0.14, 0.22)))
            .insert_resource(RunDirector::default())
            .insert_resource(WorldAssetCache::default())
            .insert_resource(SubstepCount(PHYSICS_SUBSTEPS))
            .insert_resource(NarrowPhaseConfig {
                default_speculative_margin: 0.0,
                contact_tolerance: 0.001,
                match_contacts: true,
            })
            .configure_sets(
                Update,
                (
                    BasicGameUpdateSet::Input,
                    BasicGameUpdateSet::Progress,
                    BasicGameUpdateSet::Streaming,
                    BasicGameUpdateSet::Presentation,
                    BasicGameUpdateSet::Requests,
                )
                    .chain(),
            )
            .configure_sets(Update, BasicGameUpdateSet::Requests.after(BasicGameUpdateSet::Input))
            .configure_sets(FixedUpdate, BasicGameFixedSet::Environment)
            .add_systems(Startup, (setup_scene, setup_hud).chain())
            .add_systems(
                PostStartup,
                (tune_player_camera, configure_controls_overlay).chain(),
            )
            .add_systems(
                Update,
                (
                    capture_cursor
                        .run_if(input_just_pressed(MouseButton::Left))
                        .in_set(BasicGameUpdateSet::Input),
                    release_cursor
                        .run_if(input_just_pressed(KeyCode::Escape))
                        .in_set(BasicGameUpdateSet::Input),
                    tick_run_timer.in_set(BasicGameUpdateSet::Progress),
                    queue_run_controls.in_set(BasicGameUpdateSet::Input),
                    activate_checkpoints.in_set(BasicGameUpdateSet::Progress),
                    detect_summit_completion.in_set(BasicGameUpdateSet::Progress),
                    detect_failures.in_set(BasicGameUpdateSet::Progress),
                    animate_sky_decor.in_set(BasicGameUpdateSet::Presentation),
                    evolve_atmosphere_with_progress.in_set(BasicGameUpdateSet::Presentation),
                    stream_world_chunks.in_set(BasicGameUpdateSet::Streaming),
                    update_hud.in_set(BasicGameUpdateSet::Presentation),
                    process_run_request.in_set(BasicGameUpdateSet::Requests),
                ),
            )
            .add_systems(
                FixedUpdate,
                (
                    move_movers,
                    update_crumbling_platforms,
                    apply_wind,
                )
                    .in_set(BasicGameFixedSet::Environment),
            )
            .add_systems(
                FixedPostUpdate,
                (
                    normalize_surfing_motion.before(AhoySystems::MoveCharacters),
                    apply_wall_run.after(AhoySystems::MoveCharacters),
                ),
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

fn night_bloom() -> Bloom {
    let mut bloom = Bloom::NATURAL;
    bloom.intensity = 0.035;
    bloom.low_frequency_boost = 0.14;
    bloom.low_frequency_boost_curvature = 0.8;
    bloom.high_pass_frequency = 0.86;
    bloom.prefilter.threshold = 0.88;
    bloom.prefilter.threshold_softness = 0.03;
    bloom
}

fn night_color_grading() -> ColorGrading {
    ColorGrading::with_identical_sections(
        ColorGradingGlobal {
            exposure: 1.12,
            post_saturation: 1.04,
            ..default()
        },
        ColorGradingSection {
            saturation: 1.03,
            contrast: 1.06,
            ..default()
        },
    )
}

fn night_distance_fog() -> DistanceFog {
    DistanceFog {
        color: Color::srgba(0.14, 0.16, 0.22, 1.0),
        directional_light_color: Color::srgba(0.97, 0.96, 0.92, 0.1),
        directional_light_exponent: 5.3,
        falloff: FogFalloff::from_visibility_colors(
            1_750.0,
            Color::srgb(0.14, 0.17, 0.23),
            Color::srgb(0.3, 0.34, 0.42),
        ),
    }
}

fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut asset_cache: ResMut<WorldAssetCache>,
) {
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.36, 0.38, 0.44),
        brightness: 42.0,
        affects_lightmapped_meshes: true,
    });

    let blueprint = build_run_blueprint(current_run_seed());
    let initial_look = respawn_look_for_checkpoint(&blueprint, 0);
    let snapshot = spawn_run_world(
        &blueprint,
        0,
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut asset_cache,
    );

    commands.insert_resource(RunState::new(&blueprint, snapshot));

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
            WallRunState::default(),
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
            Mass(45.0),
            StableGround::default(),
            Transform::from_translation(blueprint.spawn),
            Position(blueprint.spawn),
            initial_look.clone(),
        ))
        .id();

    commands.spawn((
        Name::new("Player Camera"),
        Camera3d::default(),
        AtmosphereCamera,
        Hdr,
        Msaa::Sample2,
        Fxaa::default(),
        Tonemapping::AgX,
        DebandDither::Enabled,
        night_bloom(),
        night_color_grading(),
        night_distance_fog(),
        CharacterControllerCameraOf::new(player),
        Transform::from_rotation(initial_look.to_quat()),
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
            top: px(16.0),
            left: px(16.0),
            max_width: px(500.0),
            padding: UiRect::axes(px(14.0), px(12.0)),
            ..default()
        },
        Text::new("Chronoclimb\nBooting run director..."),
        TextFont {
            font_size: 17.0,
            ..default()
        },
        TextColor(Color::srgb(0.9, 0.95, 1.0)),
        BackgroundColor(Color::srgba(0.012, 0.028, 0.07, 0.42)),
        RunHud,
    ));
}

fn configure_controls_overlay(mut overlay: Single<&mut Text, With<ControlsOverlay>>) {
    overlay.0 = "Controls:\n\
WASD: move\n\
Space (hold): bhop / jump / climb / ledge pull-up\n\
Ctrl: crouch / climbdown\n\
RMB: pull / drop props\n\
LMB: throw props\n\
F5: rerun seed\n\
N: new seed\n\
Esc: free mouse\n\
R: reset position\n\
Backtick: toggle debug menu"
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

    let current_height = player.translation.y;
    let speed = velocity.length();
    let start_height = run.start_height();
    let descended = (start_height - current_height).max(0.0);
    let elapsed = run.timer.elapsed_secs();

    hud.0 = format!(
        "Chronoclimb\n\
         Seed: {seed:016x}\n\
         Sections Generated: {floors} | Checkpoint: {checkpoint}/{checkpoint_total}\n\
         Altitude: {height:.1}m | Total Descent {descended:.1}m\n\
         Speed: {speed:.1} u/s\n\
         Time: {elapsed:.1}s | Deaths: {deaths}\n\
         Gen: attempts {attempts}, repairs {repairs}, overlaps {overlaps}, clearance {clearance}, reach {reach}",
        seed = run.blueprint.seed,
        floors = run.blueprint.rooms.len(),
        checkpoint = run.current_checkpoint + 1,
        checkpoint_total = run.checkpoint_count(),
        height = current_height,
        descended = descended,
        speed = speed,
        elapsed = elapsed,
        deaths = run.deaths,
        attempts = run.blueprint.stats.attempts,
        repairs = run.blueprint.stats.repairs,
        overlaps = run.blueprint.stats.overlap_issues,
        clearance = run.blueprint.stats.clearance_issues,
        reach = run.blueprint.stats.reachability_issues,
    );
}

fn run_descent_progress(run: &RunState, current_height: f32) -> f32 {
    let start_height = run.start_height().max(current_height);
    ((start_height - current_height) / ATMOSPHERE_PROGRESS_DEPTH).clamp(0.0, 1.0)
}

fn smoothstep01(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn evolve_atmosphere_with_progress(
    run: Res<RunState>,
    players: Query<&Transform, With<Player>>,
    mut clear_color: ResMut<ClearColor>,
    mut ambient: ResMut<GlobalAmbientLight>,
    camera: Option<
        Single<(&mut DistanceFog, &mut ColorGrading, &mut Bloom), With<AtmosphereCamera>>,
    >,
    atmosphere_layers: Query<(&AtmosphereMaterialKind, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Ok(player) = players.single() else {
        return;
    };

    let progress = smoothstep01(run_descent_progress(&run, player.translation.y));
    clear_color.0 = mix_color(
        Color::srgb(0.15, 0.14, 0.22),
        Color::srgb(0.1, 0.1, 0.16),
        progress,
    );

    ambient.color = mix_color(
        Color::srgb(0.36, 0.38, 0.44),
        Color::srgb(0.28, 0.3, 0.36),
        progress,
    );
    ambient.brightness = lerp(42.0, 29.0, progress);

    if let Some(camera) = camera {
        let (mut fog, mut grading, mut bloom) = camera.into_inner();
        fog.color = mix_color(
            Color::srgba(0.14, 0.16, 0.22, 1.0),
            Color::srgba(0.1, 0.11, 0.17, 1.0),
            progress,
        );
        fog.directional_light_color = mix_color(
            Color::srgba(0.97, 0.96, 0.92, 0.1),
            Color::srgba(0.82, 0.88, 0.96, 0.12),
            progress,
        );
        fog.directional_light_exponent = lerp(5.3, 6.6, progress);
        fog.falloff = FogFalloff::from_visibility_colors(
            lerp(1_850.0, 1_250.0, progress),
            mix_color(
                Color::srgb(0.14, 0.17, 0.23),
                Color::srgb(0.1, 0.11, 0.17),
                progress,
            ),
            mix_color(
                Color::srgb(0.3, 0.34, 0.42),
                Color::srgb(0.22, 0.24, 0.32),
                progress,
            ),
        );
        grading.global.exposure = lerp(1.12, 1.02, progress);
        grading.global.post_saturation = lerp(1.04, 1.0, progress);
        bloom.intensity = lerp(0.03, 0.045, progress);
        bloom.low_frequency_boost = lerp(0.12, 0.16, progress);
        bloom.prefilter.threshold = lerp(0.88, 0.82, progress);
        bloom.prefilter.threshold_softness = lerp(0.03, 0.04, progress);
    }

    for (kind, material_handle) in &atmosphere_layers {
        let Some(material) = materials.get_mut(&material_handle.0) else {
            continue;
        };

        match kind {
            AtmosphereMaterialKind::SkyDome => {
                material.base_color = mix_color(
                    Color::srgb(0.12, 0.14, 0.22),
                    Color::srgb(0.08, 0.09, 0.15),
                    progress,
                );
                material.emissive = LinearRgba::from(mix_color(
                    Color::srgb(0.1, 0.12, 0.2),
                    Color::srgb(0.06, 0.07, 0.12),
                    progress,
                )) * 0.74;
            }
            AtmosphereMaterialKind::CloudDeck => {
                material.base_color = mix_color(
                    Color::srgb(0.09, 0.1, 0.14),
                    Color::srgb(0.1, 0.08, 0.12),
                    progress,
                );
                material.emissive = LinearRgba::from(mix_color(
                    Color::srgb(0.03, 0.04, 0.06),
                    Color::srgb(0.04, 0.03, 0.05),
                    progress,
                )) * 0.32;
            }
            AtmosphereMaterialKind::Celestial => {
                material.emissive = LinearRgba::from(material.base_color.with_alpha(1.0))
                    * lerp(0.18, 0.3, progress);
            }
            AtmosphereMaterialKind::Megastructure => {
                material.emissive = LinearRgba::from(material.base_color.with_alpha(1.0))
                    * lerp(0.14, 0.22, progress);
            }
        }
    }
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
                run.focus_room = run.focus_room.max(checkpoint.index);
                let spawn = run.checkpoint_position(checkpoint.index);
                spawn_marker.translation = spawn;
                spawn_marker.rotation =
                    respawn_look_for_checkpoint(&run.blueprint, checkpoint.index).to_quat();
                run.death_plane =
                    checkpoint_death_plane(&run.blueprint, checkpoint.index, checkpoint.index);
            }
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
            seed: run.blueprint.seed,
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
            seed: run.blueprint.seed,
        });
    }
}

fn checkpoint_death_plane(
    blueprint: &RunBlueprint,
    checkpoint_index: usize,
    focus_room: usize,
) -> f32 {
    let Some(last_room) = blueprint.rooms.len().checked_sub(1) else {
        return blueprint.spawn.y - CHECKPOINT_DEATH_MARGIN;
    };

    let checkpoint_room = checkpoint_index.min(last_room);
    let end_room = focus_room
        .max(checkpoint_room)
        .saturating_add(STREAM_AHEAD_ROOMS + 2)
        .min(last_room);
    let min_room_y = blueprint.rooms[checkpoint_room..=end_room]
        .iter()
        .map(|room| room.top.y)
        .fold(f32::INFINITY, f32::min);

    min_room_y - CHECKPOINT_DEATH_MARGIN
}

fn stream_world_chunks(
    mut commands: Commands,
    director: Res<RunDirector>,
    mut run: ResMut<RunState>,
    players: Query<&Transform, With<Player>>,
    generated_chunks: Query<(Entity, &ChunkMember), With<GeneratedWorld>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut asset_cache: ResMut<WorldAssetCache>,
) {
    if director.pending.is_some() {
        return;
    }

    let focus_room = players
        .single()
        .map(|player| {
            stream_focus_room(
                &run.blueprint,
                run.current_checkpoint,
                run.focus_room,
                player.translation,
            )
        })
        .unwrap_or(run.focus_room.max(run.current_checkpoint));
    ensure_blueprint_ahead(&mut run, focus_room);
    run.focus_room = focus_room.max(run.current_checkpoint);
    run.death_plane = run.death_plane.min(checkpoint_death_plane(
        &run.blueprint,
        run.current_checkpoint,
        run.focus_room,
    ));
    let desired_order = desired_chunk_window(&run.blueprint, run.focus_room);
    let desired_chunks = desired_order.iter().copied().collect::<HashSet<_>>();
    if desired_chunks == run.spawned_chunks {
        return;
    }

    for (entity, member) in &generated_chunks {
        if !desired_chunks.contains(&member.0) {
            commands.entity(entity).despawn();
        }
    }

    for chunk in desired_order {
        if !run.spawned_chunks.contains(&chunk) {
            spawn_chunk(
                chunk,
                &run.blueprint,
                &mut commands,
                &mut meshes,
                &mut materials,
                &mut asset_cache,
            );
        }
    }

    run.spawned_chunks = desired_chunks;
}

fn ensure_blueprint_ahead(run: &mut RunState, focus_room: usize) {
    let generated_rooms = run.blueprint.rooms.len();
    let target_room_count = focus_room
        .saturating_add(STREAM_AHEAD_ROOMS)
        .saturating_add(INFINITE_APPEND_TRIGGER_ROOMS)
        .saturating_add(3);
    if generated_rooms >= target_room_count {
        return;
    }

    let append_rooms = target_room_count
        .saturating_add(INFINITE_APPEND_BATCH_ROOMS)
        .saturating_sub(generated_rooms);
    append_run_blueprint(&mut run.blueprint, append_rooms);
    run.sync_generated_geometry();
}

fn process_run_request(
    mut commands: Commands,
    mut director: ResMut<RunDirector>,
    mut run: ResMut<RunState>,
    generated: Query<Entity, With<GeneratedWorld>>,
    generated_chunks: Query<Entity, (With<GeneratedWorld>, With<ChunkMember>)>,
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
    mut asset_cache: ResMut<WorldAssetCache>,
) {
    let Some(request) = director.pending.take() else {
        return;
    };

    match request.kind {
        RunRequestKind::Respawn => {
            run.deaths += 1;
            run.finished = false;
            for entity in &generated_chunks {
                commands.entity(entity).despawn();
            }
            let snapshot = respawn_active_chunks(
                &run.blueprint,
                run.current_checkpoint,
                &mut commands,
                &mut meshes,
                &mut materials,
                &mut asset_cache,
            );
            run.spawned_chunks = snapshot.active_chunks;
            run.focus_room = run.current_checkpoint.min(run.blueprint.rooms.len().saturating_sub(1));
            run.death_plane =
                checkpoint_death_plane(&run.blueprint, run.current_checkpoint, run.focus_room);
        }
        RunRequestKind::RestartSameSeed | RunRequestKind::RestartNewSeed => {
            for entity in &generated {
                commands.entity(entity).despawn();
            }

            let blueprint = build_run_blueprint(request.seed);
            run.timer = Stopwatch::new();
            run.finished = false;
            run.deaths = 0;
            run.current_checkpoint = 0;
            run.focus_room = 0;

            let snapshot = spawn_run_world(
                &blueprint,
                0,
                &mut commands,
                &mut meshes,
                &mut materials,
                &mut asset_cache,
            );

            run.apply_blueprint(&blueprint, snapshot);
        }
    }

    let spawn = run.checkpoint_position(run.current_checkpoint);
    let respawn_look = respawn_look_for_checkpoint(&run.blueprint, run.current_checkpoint);
    spawn_marker.translation = spawn;
    spawn_marker.rotation = respawn_look.to_quat();

    if let Ok((mut position, mut transform, mut velocity, mut look, mut wall_run)) =
        players.single_mut()
    {
        position.0 = spawn;
        transform.translation = spawn;
        velocity.0 = Vec3::ZERO;
        *look = respawn_look.clone();
        *wall_run = WallRunState::default();
    }

    if let Ok(mut camera_transform) = camera.single_mut() {
        camera_transform.rotation = respawn_look.to_quat();
    }
}

fn respawn_look_for_checkpoint(blueprint: &RunBlueprint, checkpoint_index: usize) -> CharacterLook {
    let fallback = CharacterLook::default();
    let room_count = blueprint.rooms.len();
    if room_count < 2 {
        return fallback;
    }

    let current = checkpoint_index.min(room_count - 1);
    let mut facing = if current + 1 < room_count {
        blueprint.rooms[current + 1].top - blueprint.rooms[current].top
    } else {
        blueprint.rooms[current].top - blueprint.rooms[current.saturating_sub(1)].top
    };
    facing.y = 0.0;
    let facing = facing.normalize_or_zero();
    if facing == Vec3::ZERO {
        return fallback;
    }

    CharacterLook {
        yaw: facing.x.atan2(facing.z),
        pitch: 0.0,
    }
}

fn tune_player_camera(mut cameras: Query<&mut Projection, With<Camera3d>>) {
    for mut projection in &mut cameras {
        if let Projection::Perspective(perspective) = &mut *projection {
            perspective.near = 0.03;
            perspective.fov = 82.0_f32.to_radians();
            perspective.far = 2_400.0;
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
struct GeneratedWorld;

#[derive(Component)]
struct AtmosphereCamera;

#[derive(Component, Clone, Copy)]
enum AtmosphereMaterialKind {
    SkyDome,
    CloudDeck,
    Celestial,
    Megastructure,
}

#[derive(Component, Clone, Copy, PartialEq, Eq, Hash)]
struct ChunkMember(WorldChunkKey);

#[derive(Component)]
struct SurfRampSurface;

#[derive(Component)]
struct CheckpointPad {
    index: usize,
}

#[derive(Component)]
struct SummitGoal;

#[derive(Component)]
struct WindZone {
    size: Vec3,
    direction: Vec3,
    strength: f32,
    gust: f32,
}

#[derive(Component)]
struct SkyDrift {
    anchor: Vec3,
    primary_axis: Vec3,
    secondary_axis: Vec3,
    primary_amplitude: f32,
    secondary_amplitude: f32,
    vertical_amplitude: f32,
    speed: f32,
    rotation_speed: f32,
    phase: f32,
    base_rotation: Quat,
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

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum WorldChunkKey {
    Room(usize),
    Segment(usize),
    Summit,
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
struct GameMaterialKey {
    paint: PaintStyle,
    ghost: bool,
    surf: bool,
    vertex_colored: bool,
}

#[derive(Resource, Default)]
struct WorldAssetCache {
    cuboid_meshes: HashMap<MeshSizeKey, Handle<Mesh>>,
    gameplay_materials: HashMap<GameMaterialKey, Handle<StandardMaterial>>,
}

#[derive(Resource)]
struct RunState {
    blueprint: RunBlueprint,
    death_plane: f32,
    current_checkpoint: usize,
    focus_room: usize,
    deaths: u32,
    timer: Stopwatch,
    finished: bool,
    spawned_chunks: HashSet<WorldChunkKey>,
}

impl RunState {
    fn new(blueprint: &RunBlueprint, snapshot: RunSnapshot) -> Self {
        Self {
            blueprint: blueprint.clone(),
            death_plane: checkpoint_death_plane(blueprint, 0, 0),
            current_checkpoint: 0,
            focus_room: 0,
            deaths: 0,
            timer: Stopwatch::new(),
            finished: false,
            spawned_chunks: snapshot.active_chunks,
        }
    }

    fn apply_blueprint(&mut self, blueprint: &RunBlueprint, snapshot: RunSnapshot) {
        self.blueprint = blueprint.clone();
        self.spawned_chunks = snapshot.active_chunks;
        self.current_checkpoint = self.current_checkpoint.min(self.last_checkpoint_index());
        self.focus_room = self.current_checkpoint.min(self.blueprint.rooms.len().saturating_sub(1));
        self.death_plane =
            checkpoint_death_plane(&self.blueprint, self.current_checkpoint, self.focus_room);
    }

    fn sync_generated_geometry(&mut self) {
        self.current_checkpoint = self.current_checkpoint.min(self.last_checkpoint_index());
        self.focus_room = self
            .focus_room
            .max(self.current_checkpoint)
            .min(self.blueprint.rooms.len().saturating_sub(1));
        self.death_plane = self.death_plane.min(checkpoint_death_plane(
            &self.blueprint,
            self.current_checkpoint,
            self.focus_room,
        ));
    }

    fn checkpoint_count(&self) -> usize {
        self.blueprint
            .rooms
            .iter()
            .filter(|room| room.checkpoint_slot.is_some())
            .count()
            .max(1)
    }

    fn checkpoint_position(&self, checkpoint_index: usize) -> Vec3 {
        self.blueprint
            .rooms
            .iter()
            .filter(|room| room.checkpoint_slot.is_some())
            .nth(checkpoint_index)
            .map(|room| room.top + Vec3::new(0.0, PLAYER_SPAWN_CLEARANCE, 0.0))
            .unwrap_or(self.blueprint.spawn)
    }

    fn last_checkpoint_index(&self) -> usize {
        self.checkpoint_count().saturating_sub(1)
    }

    fn start_height(&self) -> f32 {
        self.checkpoint_position(0).y
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
                    Hold::new(0.1),
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
    shaft_cooldown: f32,
}

#[derive(Component, Default)]
struct SurfMovementState {
    jump_lock: f32,
}

fn apply_wall_run(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
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
        wall_run.shaft_cooldown = (wall_run.shaft_cooldown - dt).max(0.0);

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

        if let Some((_left, _right)) = find_wall_shaft(output) {
            if state.grounded.is_none()
                && state.mantle.is_none()
                && state.crane_height_left.is_none()
                && water.level <= WaterLevel::Feet
                && keys.pressed(KeyCode::Space)
            {
                if wall_run.shaft_cooldown <= 0.0 {
                    velocity.x *= 0.3;
                    velocity.z *= 0.3;
                    velocity.y = velocity.y.max(WALL_SHAFT_BOOST_SPEED);
                    wall_run.shaft_cooldown = WALL_SHAFT_REPEAT;
                }
                wall_run.active = false;
                continue;
            }
        }

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

fn is_surf_touch(normal: Dir3) -> bool {
    normal.y > 0.18 && normal.y < 0.93 && Vec2::new(normal.x, normal.z).length_squared() > 0.08
}

fn find_wall_shaft(output: &CharacterControllerOutput) -> Option<(Vec3, Vec3)> {
    let walls = output
        .touching_entities
        .iter()
        .filter_map(|touch| {
            if touch.normal.y.abs() > 0.2 {
                return None;
            }
            let normal = Vec3::new(touch.normal.x, 0.0, touch.normal.z).normalize_or_zero();
            (normal != Vec3::ZERO).then_some(normal)
        })
        .collect::<Vec<_>>();

    for (index, normal) in walls.iter().enumerate() {
        for other in walls.iter().skip(index + 1) {
            if normal.dot(*other) < -0.82 {
                return Some((*normal, *other));
            }
        }
    }

    None
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

fn animate_sky_decor(time: Res<Time>, mut decor: Query<(&mut Transform, &SkyDrift)>) {
    let elapsed = time.elapsed_secs();
    for (mut transform, drift) in &mut decor {
        let phase = elapsed * drift.speed + drift.phase;
        transform.translation = drift.anchor
            + drift.primary_axis * (drift.primary_amplitude * phase.sin())
            + drift.secondary_axis * (drift.secondary_amplitude * (phase * 0.63 + 1.1).cos())
            + Vec3::Y * (drift.vertical_amplitude * (phase * 0.47).sin());
        transform.rotation = drift.base_rotation
            * Quat::from_rotation_y(phase * drift.rotation_speed)
            * Quat::from_rotation_z((phase * 0.51).sin() * 0.035);
    }
}

#[derive(Clone)]
struct CourseGraph {
    seed: u64,
    rooms: Vec<RoomPlan>,
    segments: Vec<SegmentPlan>,
    zones: Vec<ZonePlan>,
    zone_edges: Vec<ZoneEdge>,
}

#[derive(Clone)]
struct RunBlueprint {
    seed: u64,
    rooms: Vec<RoomPlan>,
    segments: Vec<SegmentPlan>,
    zones: Vec<ZonePlan>,
    zone_edges: Vec<ZoneEdge>,
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
    biome: BiomeStyle,
    seed: u64,
    section: RoomSectionKind,
    zone_index: usize,
    layer_index: i32,
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
    zone_index: usize,
    zone_role: ZoneRole,
    zone_signature: ZoneSignature,
    biome: BiomeStyle,
    connector: ConnectorKind,
    flow: FlowFieldProfile,
    route_lines: Vec<RouteLine>,
    zone_local_t: f32,
    exit_socket: SocketMask,
}

#[derive(Clone, Default)]
struct GenerationStats {
    attempts: u32,
    repairs: u32,
    downgraded_segments: u32,
    overlap_issues: usize,
    clearance_issues: usize,
    reachability_issues: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ModuleKind {
    StairRun,
    SurfRamp,
    WindowHop,
    PillarAirstrafe,
    HeadcheckRun,
    SpeedcheckRun,
    MovingPlatformRun,
    ShapeGauntlet,
    MantleStack,
    WallRunHall,
    LiftChasm,
    CrumbleBridge,
    WindTunnel,
    IceSpine,
    WaterGarden,
}

#[derive(Clone, Copy)]
enum RoomSectionKind {
    OpenPad,
    SplitPad,
    Terrace,
    CornerPerches,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Theme {
    Stone,
    Overgrown,
    Frost,
    Ember,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BiomeStyle {
    NeonCyber,
    AbstractGeometry,
    GlassNebula,
    OrbitalIndustrial,
    AuroraMonolith,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ZoneRole {
    Accelerator,
    Technical,
    Recovery,
    Spectacle,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ZoneSignature {
    BowlCollector,
    GiantCorkscrew,
    BraidLanes,
    WaveRamps,
    SplitTransfer,
    PillarForest,
    ShapeGarden,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
enum RouteLine {
    Safe,
    Speed,
    Trick,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LandmarkKind {
    BrokenRing,
    ImpossibleBridge,
    CorkscrewTower,
    FloatingMonolithCluster,
    MovingMegastructure,
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
}

#[derive(Clone, Copy, Debug)]
enum RouteCurveArchetype {
    Straight,
    Carve,
    Switchback,
    Slalom,
}

#[derive(Clone, Copy, Debug)]
enum PathLateralStyle {
    Straight,
    Serpentine,
    Switchback,
    Arc,
    OneSidedArc,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ZoneKind {
    Accelerator,
    BranchBraid,
    LayeredLoop,
    SpiralCorkscrew,
    WaveField,
    BasinCollector,
    TransferWeb,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ConnectorKind {
    Funnel,
    Booster,
    Transfer,
    Collector,
    Splitter,
    Crossover,
}

#[derive(Clone, Copy, Debug)]
struct FlowFieldProfile {
    route_curve: RouteCurveArchetype,
    path_style: PathLateralStyle,
    curvature: f32,
    weave_cycles: f32,
    lateral_amplitude: f32,
    vertical_wave: f32,
    noise_amplitude: f32,
    noise_frequency: f32,
    width_scale: f32,
    layered_offset: f32,
    branch_bias: f32,
    dynamic_bias: f32,
    drift_sign: f32,
}

#[derive(Clone, Copy, Debug)]
struct FlowFieldSample {
    lateral: f32,
    vertical: f32,
    width_scale: f32,
}

#[derive(Clone)]
struct ZonePlan {
    index: usize,
    kind: ZoneKind,
    role: ZoneRole,
    signature: ZoneSignature,
    biome: BiomeStyle,
    start_segment: usize,
    end_segment: usize,
    flow: FlowFieldProfile,
    entry_connector: ConnectorKind,
    exit_connector: ConnectorKind,
    landmark: LandmarkKind,
    route_lines: Vec<RouteLine>,
}

#[derive(Clone, Copy)]
struct ZoneEdge {
    from: usize,
    to: usize,
    connector: ConnectorKind,
    branching: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum OwnerTag {
    Room(usize),
    Segment(usize),
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
    StaticSphere,
    StaticCylinder,
    StaticTrapezoid {
        top_scale: f32,
    },
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
    Decoration,
    DynamicProp,
    Water {
        speed: f32,
    },
    Moving {
        end: Vec3,
        speed: f32,
        lethal: bool,
    },
    Crumbling {
        delay: f32,
        sink_speed: f32,
    },
}

#[derive(Clone, Copy)]
enum ExtraKind {
    None,
    SummitGoal,
}

#[derive(Clone)]
enum FeatureSpec {
    CheckpointPad {
        center: Vec3,
        index: usize,
    },
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

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum PaintStyle {
    ThemeFloor(Theme),
    ThemeAccent(Theme),
    ThemeShadow(Theme),
    SectionPlatform(Theme),
    Summit,
    Checkpoint,
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
    active_chunks: HashSet<WorldChunkKey>,
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

fn is_primary_speed_section(kind: ModuleKind) -> bool {
    matches!(
        kind,
        ModuleKind::StairRun
            | ModuleKind::SurfRamp
            | ModuleKind::WindowHop
            | ModuleKind::PillarAirstrafe
            | ModuleKind::HeadcheckRun
            | ModuleKind::SpeedcheckRun
            | ModuleKind::MovingPlatformRun
            | ModuleKind::ShapeGauntlet
    )
}

fn speed_room_size(kind: ModuleKind) -> f32 {
    match kind {
        ModuleKind::SurfRamp => 15.4,
        ModuleKind::ShapeGauntlet => 15.0,
        ModuleKind::PillarAirstrafe | ModuleKind::MovingPlatformRun => 14.8,
        _ if is_primary_speed_section(kind) => 14.6,
        _ => 13.2,
    }
}

fn biome_theme(biome: BiomeStyle, fallback_index: usize, total: usize, offset: usize) -> Theme {
    match biome {
        BiomeStyle::NeonCyber => Theme::Frost,
        BiomeStyle::AbstractGeometry => Theme::Stone,
        BiomeStyle::GlassNebula => Theme::Overgrown,
        BiomeStyle::OrbitalIndustrial => Theme::Stone,
        BiomeStyle::AuroraMonolith => {
            let band = (fallback_index * 4 / total.max(1) + offset) % 4;
            [Theme::Ember, Theme::Frost, Theme::Stone, Theme::Overgrown][band]
        }
    }
}

fn choose_zone_role(_rng: &mut RunRng, zone_index: usize, _previous: Option<ZoneRole>) -> ZoneRole {
    if zone_index == 0 {
        return ZoneRole::Accelerator;
    }

    let cycle = [
        ZoneRole::Accelerator,
        ZoneRole::Technical,
        ZoneRole::Recovery,
        ZoneRole::Spectacle,
    ];
    cycle[zone_index % cycle.len()]
}

fn choose_zone_signature(
    rng: &mut RunRng,
    role: ZoneRole,
    difficulty: f32,
    previous: Option<ZoneSignature>,
) -> ZoneSignature {
    let mut weighted = match role {
        ZoneRole::Accelerator => vec![
            (ZoneSignature::WaveRamps, 5_u32),
            (ZoneSignature::GiantCorkscrew, 4),
            (ZoneSignature::BraidLanes, 3),
            (ZoneSignature::SplitTransfer, 2),
        ],
        ZoneRole::Technical => vec![
            (ZoneSignature::PillarForest, 5_u32),
            (ZoneSignature::ShapeGarden, 5),
            (ZoneSignature::SplitTransfer, 4),
            (ZoneSignature::BraidLanes, 4),
        ],
        ZoneRole::Recovery => vec![
            (ZoneSignature::BowlCollector, 6_u32),
            (ZoneSignature::WaveRamps, 4),
            (ZoneSignature::GiantCorkscrew, 2),
        ],
        ZoneRole::Spectacle => vec![
            (ZoneSignature::GiantCorkscrew, 5_u32),
            (ZoneSignature::BowlCollector, 4),
            (ZoneSignature::ShapeGarden, 4),
            (ZoneSignature::BraidLanes, 3),
            (ZoneSignature::SplitTransfer, 2),
        ],
    };

    if difficulty > 0.35 {
        weighted.push((ZoneSignature::SplitTransfer, 2));
        weighted.push((ZoneSignature::BraidLanes, 2));
    }
    if difficulty > 0.5 {
        weighted.push((ZoneSignature::GiantCorkscrew, 2));
        weighted.push((ZoneSignature::ShapeGarden, 2));
    }
    if difficulty > 0.65 {
        weighted.push((ZoneSignature::PillarForest, 2));
    }

    let mut signature = rng.weighted_choice(&weighted);
    if previous == Some(signature) && rng.chance(0.5) {
        signature = rng.weighted_choice(&weighted);
    }
    signature
}

fn legacy_zone_kind(signature: ZoneSignature, role: ZoneRole) -> ZoneKind {
    match signature {
        ZoneSignature::BowlCollector => ZoneKind::BasinCollector,
        ZoneSignature::GiantCorkscrew => ZoneKind::SpiralCorkscrew,
        ZoneSignature::BraidLanes => ZoneKind::BranchBraid,
        ZoneSignature::WaveRamps => {
            if matches!(role, ZoneRole::Accelerator) {
                ZoneKind::Accelerator
            } else {
                ZoneKind::WaveField
            }
        }
        ZoneSignature::SplitTransfer => ZoneKind::TransferWeb,
        ZoneSignature::PillarForest | ZoneSignature::ShapeGarden => ZoneKind::LayeredLoop,
    }
}

fn choose_landmark_kind(
    rng: &mut RunRng,
    role: ZoneRole,
    signature: ZoneSignature,
) -> LandmarkKind {
    match signature {
        ZoneSignature::BowlCollector => rng.weighted_choice(&[
            (LandmarkKind::BrokenRing, 4_u32),
            (LandmarkKind::MovingMegastructure, 3),
        ]),
        ZoneSignature::GiantCorkscrew => LandmarkKind::CorkscrewTower,
        ZoneSignature::BraidLanes => rng.weighted_choice(&[
            (LandmarkKind::BrokenRing, 3_u32),
            (LandmarkKind::MovingMegastructure, 4),
        ]),
        ZoneSignature::WaveRamps => rng.weighted_choice(&[
            (LandmarkKind::BrokenRing, 3_u32),
            (LandmarkKind::MovingMegastructure, 4),
        ]),
        ZoneSignature::SplitTransfer => LandmarkKind::MovingMegastructure,
        ZoneSignature::PillarForest => LandmarkKind::FloatingMonolithCluster,
        ZoneSignature::ShapeGarden => rng.weighted_choice(&[
            (LandmarkKind::FloatingMonolithCluster, 4_u32),
            (
                LandmarkKind::MovingMegastructure,
                if matches!(role, ZoneRole::Spectacle) {
                    4
                } else {
                    2
                },
            ),
        ]),
    }
}

fn major_zone_route_lines(role: ZoneRole, signature: ZoneSignature) -> Vec<RouteLine> {
    let mut lines = vec![RouteLine::Speed];
    if !matches!(role, ZoneRole::Accelerator)
        || matches!(
            signature,
            ZoneSignature::BraidLanes
                | ZoneSignature::SplitTransfer
                | ZoneSignature::ShapeGarden
                | ZoneSignature::PillarForest
        )
    {
        lines.insert(0, RouteLine::Safe);
    }
    if !matches!(role, ZoneRole::Recovery) || !matches!(signature, ZoneSignature::BowlCollector) {
        lines.push(RouteLine::Trick);
    } else {
        lines.push(RouteLine::Trick);
    }
    lines
}

fn choose_biome_style(
    rng: &mut RunRng,
    difficulty: f32,
    role: ZoneRole,
    signature: ZoneSignature,
    previous: Option<BiomeStyle>,
) -> BiomeStyle {
    let mut weighted = vec![
        (BiomeStyle::NeonCyber, 3_u32),
        (BiomeStyle::AbstractGeometry, 3),
        (BiomeStyle::GlassNebula, 3),
        (BiomeStyle::OrbitalIndustrial, 3),
        (
            BiomeStyle::AuroraMonolith,
            if difficulty > 0.45 { 4 } else { 2 },
        ),
    ];

    match role {
        ZoneRole::Accelerator => {
            weighted.push((BiomeStyle::NeonCyber, 4));
            weighted.push((BiomeStyle::GlassNebula, 3));
        }
        ZoneRole::Technical => {
            weighted.push((BiomeStyle::AbstractGeometry, 4));
            weighted.push((BiomeStyle::OrbitalIndustrial, 3));
        }
        ZoneRole::Recovery => {
            weighted.push((BiomeStyle::GlassNebula, 4));
            weighted.push((BiomeStyle::AuroraMonolith, 3));
        }
        ZoneRole::Spectacle => {
            weighted.push((BiomeStyle::OrbitalIndustrial, 4));
            weighted.push((BiomeStyle::AuroraMonolith, 3));
        }
    }

    match signature {
        ZoneSignature::ShapeGarden | ZoneSignature::PillarForest => {
            weighted.push((BiomeStyle::AbstractGeometry, 3));
        }
        ZoneSignature::GiantCorkscrew | ZoneSignature::SplitTransfer => {
            weighted.push((BiomeStyle::OrbitalIndustrial, 3));
        }
        ZoneSignature::WaveRamps | ZoneSignature::BowlCollector => {
            weighted.push((BiomeStyle::GlassNebula, 3));
        }
        ZoneSignature::BraidLanes => weighted.push((BiomeStyle::NeonCyber, 3)),
    }

    let mut biome = rng.weighted_choice(&weighted);
    if previous == Some(biome) && rng.chance(0.35) {
        biome = rng.weighted_choice(&weighted);
    }
    biome
}

fn zone_segment_span(rng: &mut RunRng, role: ZoneRole, signature: ZoneSignature) -> usize {
    match (role, signature) {
        (ZoneRole::Accelerator, ZoneSignature::WaveRamps) => rng.range_usize(4, 7),
        (ZoneRole::Accelerator, ZoneSignature::GiantCorkscrew) => rng.range_usize(4, 6),
        (ZoneRole::Technical, ZoneSignature::PillarForest | ZoneSignature::ShapeGarden) => {
            rng.range_usize(4, 6)
        }
        (ZoneRole::Recovery, ZoneSignature::BowlCollector) => rng.range_usize(3, 5),
        (ZoneRole::Spectacle, _) => rng.range_usize(5, 7),
        (_, ZoneSignature::SplitTransfer | ZoneSignature::BraidLanes) => rng.range_usize(4, 6),
        _ => rng.range_usize(3, 6),
    }
}

fn choose_zone_entry_connector(
    rng: &mut RunRng,
    role: ZoneRole,
    signature: ZoneSignature,
    previous: Option<ZoneRole>,
) -> ConnectorKind {
    if previous.is_none() {
        return ConnectorKind::Funnel;
    }

    match (role, signature) {
        (ZoneRole::Accelerator, _) => rng.weighted_choice(&[
            (ConnectorKind::Funnel, 4),
            (ConnectorKind::Booster, 5),
            (ConnectorKind::Transfer, 2),
        ]),
        (_, ZoneSignature::BraidLanes) => rng.weighted_choice(&[
            (ConnectorKind::Splitter, 5),
            (ConnectorKind::Crossover, 3),
            (ConnectorKind::Transfer, 2),
        ]),
        (_, ZoneSignature::SplitTransfer) => rng.weighted_choice(&[
            (ConnectorKind::Splitter, 6),
            (ConnectorKind::Transfer, 3),
            (ConnectorKind::Crossover, 3),
        ]),
        (_, ZoneSignature::PillarForest | ZoneSignature::ShapeGarden) => rng.weighted_choice(&[
            (ConnectorKind::Crossover, 5),
            (ConnectorKind::Transfer, 4),
            (ConnectorKind::Collector, 2),
        ]),
        (_, ZoneSignature::GiantCorkscrew) => rng.weighted_choice(&[
            (ConnectorKind::Funnel, 3),
            (ConnectorKind::Transfer, 4),
            (ConnectorKind::Booster, 2),
        ]),
        (_, ZoneSignature::WaveRamps) => rng.weighted_choice(&[
            (ConnectorKind::Funnel, 4),
            (ConnectorKind::Collector, 3),
            (ConnectorKind::Transfer, 2),
        ]),
        (_, ZoneSignature::BowlCollector) => rng.weighted_choice(&[
            (ConnectorKind::Collector, 5),
            (ConnectorKind::Funnel, 2),
            (ConnectorKind::Transfer, 2),
        ]),
    }
}

fn choose_zone_exit_connector(
    rng: &mut RunRng,
    role: ZoneRole,
    signature: ZoneSignature,
) -> ConnectorKind {
    match (role, signature) {
        (ZoneRole::Accelerator, _) => rng.weighted_choice(&[
            (ConnectorKind::Collector, 4),
            (ConnectorKind::Transfer, 4),
            (ConnectorKind::Booster, 2),
        ]),
        (_, ZoneSignature::BraidLanes) => rng.weighted_choice(&[
            (ConnectorKind::Crossover, 5),
            (ConnectorKind::Collector, 3),
            (ConnectorKind::Splitter, 2),
        ]),
        (_, ZoneSignature::SplitTransfer) => rng.weighted_choice(&[
            (ConnectorKind::Collector, 5),
            (ConnectorKind::Crossover, 4),
            (ConnectorKind::Transfer, 2),
        ]),
        (_, ZoneSignature::PillarForest | ZoneSignature::ShapeGarden) => rng.weighted_choice(&[
            (ConnectorKind::Crossover, 4),
            (ConnectorKind::Collector, 4),
            (ConnectorKind::Transfer, 2),
        ]),
        (_, ZoneSignature::GiantCorkscrew) => rng.weighted_choice(&[
            (ConnectorKind::Transfer, 4),
            (ConnectorKind::Collector, 3),
            (ConnectorKind::Booster, 2),
        ]),
        (_, ZoneSignature::WaveRamps) => rng.weighted_choice(&[
            (ConnectorKind::Collector, 4),
            (ConnectorKind::Transfer, 3),
            (ConnectorKind::Funnel, 2),
        ]),
        (_, ZoneSignature::BowlCollector) => rng.weighted_choice(&[
            (ConnectorKind::Collector, 6),
            (ConnectorKind::Transfer, 2),
            (ConnectorKind::Booster, 1),
        ]),
    }
}

fn default_flow_profile() -> FlowFieldProfile {
    FlowFieldProfile {
        route_curve: RouteCurveArchetype::Straight,
        path_style: PathLateralStyle::Straight,
        curvature: 0.8,
        weave_cycles: 1.0,
        lateral_amplitude: 0.5,
        vertical_wave: 0.1,
        noise_amplitude: 0.12,
        noise_frequency: 0.9,
        width_scale: 1.0,
        layered_offset: 0.0,
        branch_bias: 0.0,
        dynamic_bias: 0.0,
        drift_sign: 1.0,
    }
}

fn zone_flow_profile(
    rng: &mut RunRng,
    role: ZoneRole,
    signature: ZoneSignature,
    difficulty: f32,
) -> FlowFieldProfile {
    let mut profile = default_flow_profile();
    match signature {
        ZoneSignature::BowlCollector => {
            profile.route_curve = rng.weighted_choice(&[
                (RouteCurveArchetype::Carve, 4),
                (RouteCurveArchetype::Straight, 2),
                (RouteCurveArchetype::Slalom, 2),
            ]);
            profile.path_style = rng.weighted_choice(&[
                (PathLateralStyle::Arc, 5),
                (PathLateralStyle::OneSidedArc, 3),
                (PathLateralStyle::Straight, 2),
            ]);
            profile.curvature = rng.range_f32(0.86, 1.08);
            profile.weave_cycles = rng.range_f32(0.9, 1.6);
            profile.lateral_amplitude = rng.range_f32(0.55, 1.0);
            profile.vertical_wave = rng.range_f32(0.08, 0.22);
            profile.width_scale = rng.range_f32(1.1, 1.32);
        }
        ZoneSignature::GiantCorkscrew => {
            profile.route_curve = RouteCurveArchetype::Carve;
            profile.path_style = PathLateralStyle::OneSidedArc;
            profile.curvature = rng.range_f32(1.06, 1.34);
            profile.weave_cycles = rng.range_f32(0.72, 1.2);
            profile.lateral_amplitude = rng.range_f32(1.15, 1.92);
            profile.vertical_wave = rng.range_f32(0.1, 0.22);
            profile.layered_offset = rng.range_f32(0.18, 0.56);
            profile.width_scale = rng.range_f32(0.94, 1.12);
        }
        ZoneSignature::BraidLanes => {
            profile.route_curve = rng.weighted_choice(&[
                (RouteCurveArchetype::Switchback, 4),
                (RouteCurveArchetype::Slalom, 4),
                (RouteCurveArchetype::Carve, 2),
            ]);
            profile.path_style = rng.weighted_choice(&[
                (PathLateralStyle::Switchback, 5),
                (PathLateralStyle::Serpentine, 4),
                (PathLateralStyle::Arc, 2),
            ]);
            profile.curvature = rng.range_f32(1.02, 1.3);
            profile.weave_cycles = rng.range_f32(2.2, 3.5);
            profile.lateral_amplitude = rng.range_f32(1.15, 2.0);
            profile.vertical_wave = rng.range_f32(0.08, 0.2);
            profile.branch_bias = rng.range_f32(0.4, 0.8);
        }
        ZoneSignature::WaveRamps => {
            profile.route_curve = rng.weighted_choice(&[
                (RouteCurveArchetype::Slalom, 5),
                (RouteCurveArchetype::Carve, 3),
                (RouteCurveArchetype::Straight, 2),
            ]);
            profile.path_style = rng.weighted_choice(&[
                (PathLateralStyle::Serpentine, 4),
                (PathLateralStyle::Arc, 3),
                (PathLateralStyle::OneSidedArc, 3),
            ]);
            profile.curvature = rng.range_f32(0.94, 1.18);
            profile.weave_cycles = rng.range_f32(1.8, 3.8);
            profile.lateral_amplitude = rng.range_f32(0.92, 1.72);
            profile.vertical_wave = rng.range_f32(0.14, 0.32);
            profile.noise_amplitude = rng.range_f32(0.16, 0.28);
            profile.noise_frequency = rng.range_f32(1.1, 1.65);
        }
        ZoneSignature::SplitTransfer => {
            profile.route_curve = rng.weighted_choice(&[
                (RouteCurveArchetype::Switchback, 4),
                (RouteCurveArchetype::Carve, 3),
                (RouteCurveArchetype::Slalom, 3),
            ]);
            profile.path_style = rng.weighted_choice(&[
                (PathLateralStyle::Switchback, 4),
                (PathLateralStyle::Arc, 3),
                (PathLateralStyle::Serpentine, 3),
            ]);
            profile.curvature = rng.range_f32(1.02, 1.28);
            profile.weave_cycles = rng.range_f32(1.5, 2.8);
            profile.lateral_amplitude = rng.range_f32(1.02, 1.82);
            profile.vertical_wave = rng.range_f32(0.08, 0.2);
            profile.branch_bias = rng.range_f32(0.3, 0.64);
        }
        ZoneSignature::PillarForest => {
            profile.route_curve = rng.weighted_choice(&[
                (RouteCurveArchetype::Switchback, 4),
                (RouteCurveArchetype::Slalom, 3),
                (RouteCurveArchetype::Carve, 2),
            ]);
            profile.path_style = rng.weighted_choice(&[
                (PathLateralStyle::Switchback, 4),
                (PathLateralStyle::Serpentine, 3),
                (PathLateralStyle::Arc, 3),
            ]);
            profile.curvature = rng.range_f32(0.96, 1.24);
            profile.weave_cycles = rng.range_f32(1.8, 3.2);
            profile.lateral_amplitude = rng.range_f32(1.0, 1.88);
            profile.vertical_wave = rng.range_f32(0.12, 0.26);
            profile.layered_offset = rng.range_f32(0.18, 0.62);
        }
        ZoneSignature::ShapeGarden => {
            profile.route_curve = rng.weighted_choice(&[
                (RouteCurveArchetype::Switchback, 4),
                (RouteCurveArchetype::Slalom, 4),
                (RouteCurveArchetype::Carve, 2),
            ]);
            profile.path_style = rng.weighted_choice(&[
                (PathLateralStyle::Switchback, 4),
                (PathLateralStyle::Serpentine, 4),
                (PathLateralStyle::Arc, 2),
            ]);
            profile.curvature = rng.range_f32(1.0, 1.28);
            profile.weave_cycles = rng.range_f32(1.7, 3.0);
            profile.lateral_amplitude = rng.range_f32(1.08, 1.92);
            profile.vertical_wave = rng.range_f32(0.1, 0.22);
            profile.layered_offset = rng.range_f32(0.16, 0.48);
        }
    }

    match role {
        ZoneRole::Accelerator => {
            profile.width_scale += 0.08;
            profile.dynamic_bias += rng.range_f32(0.18, 0.32);
        }
        ZoneRole::Technical => {
            profile.curvature += 0.06;
            profile.noise_amplitude += 0.04;
            profile.branch_bias += 0.08;
        }
        ZoneRole::Recovery => {
            profile.width_scale += 0.12;
            profile.curvature *= 0.92;
            profile.lateral_amplitude *= 0.88;
        }
        ZoneRole::Spectacle => {
            profile.width_scale += 0.16;
            profile.layered_offset += 0.08;
            profile.vertical_wave += 0.04;
        }
    }

    profile.curvature += difficulty * 0.16;
    profile.noise_amplitude += difficulty * 0.08;
    profile.drift_sign = if rng.chance(0.5) { 1.0 } else { -1.0 };
    profile
}

fn should_branch_zone(
    role: ZoneRole,
    signature: ZoneSignature,
    flow: FlowFieldProfile,
    difficulty: f32,
    rng: &mut RunRng,
) -> bool {
    let base = match signature {
        ZoneSignature::BraidLanes | ZoneSignature::SplitTransfer => 0.85,
        ZoneSignature::PillarForest | ZoneSignature::ShapeGarden => 0.52,
        ZoneSignature::WaveRamps | ZoneSignature::BowlCollector => 0.28,
        ZoneSignature::GiantCorkscrew => 0.22,
    } + match role {
        ZoneRole::Technical => 0.08,
        ZoneRole::Spectacle => 0.04,
        ZoneRole::Accelerator | ZoneRole::Recovery => 0.0,
    };
    rng.chance((base + flow.branch_bias * 0.2 + difficulty * 0.08).clamp(0.0, 0.95))
}

fn zone_layer_index(
    signature: ZoneSignature,
    flow: FlowFieldProfile,
    seed: u64,
    local_t: f32,
) -> i32 {
    let sample = sample_flow_field(flow, seed, local_t, 1.0);
    let layers = match signature {
        ZoneSignature::PillarForest | ZoneSignature::ShapeGarden => {
            sample.vertical * 3.2 + sample.lateral * 0.52
        }
        ZoneSignature::GiantCorkscrew => {
            (local_t * TAU * (1.0 + flow.curvature * 0.12) + flow.drift_sign).sin() * 2.4
                + sample.vertical * 1.2
        }
        ZoneSignature::WaveRamps => sample.vertical * 2.6,
        ZoneSignature::BraidLanes | ZoneSignature::SplitTransfer => sample.lateral * 1.8,
        ZoneSignature::BowlCollector => sample.vertical * 1.4 - 0.4,
    };
    layers.round() as i32
}

fn flow_noise_1d(seed: u64, x: f32) -> f32 {
    let x0 = x.floor() as i32;
    let x1 = x0 + 1;
    let t = smoothstep01(x.fract());
    let v0 = hash_noise(seed, x0);
    let v1 = hash_noise(seed, x1);
    lerp(v0, v1, t) * 2.0 - 1.0
}

fn hash_noise(seed: u64, x: i32) -> f32 {
    let mut z = seed ^ (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    let bits = (z ^ (z >> 31)) >> 40;
    bits as f32 / ((1_u64 << 24) - 1) as f32
}

fn sample_flow_field(
    profile: FlowFieldProfile,
    seed: u64,
    t: f32,
    envelope: f32,
) -> FlowFieldSample {
    let base_lateral = path_lateral_offset(
        profile.path_style,
        t,
        envelope,
        seed as f32 * 0.000_000_13,
        profile.weave_cycles,
        profile.lateral_amplitude,
    );
    let noise_phase = t * profile.noise_frequency * 6.0;
    let noise =
        flow_noise_1d(seed ^ 0xA11C_E_u64, noise_phase) * profile.noise_amplitude * envelope;
    let layered_wave = (t * TAU * (1.0 + profile.layered_offset) + profile.drift_sign).sin()
        * profile.vertical_wave
        * envelope;

    FlowFieldSample {
        lateral: (base_lateral + noise) * profile.curvature,
        vertical: layered_wave
            + flow_noise_1d(seed ^ 0x51DE_C0DE_u64, noise_phase + 3.7)
                * profile.vertical_wave
                * 0.35
                * envelope,
        width_scale: (profile.width_scale
            + flow_noise_1d(seed ^ 0xBEEF_1000_u64, noise_phase + 1.7) * 0.08)
            .clamp(0.74, 1.42),
    }
}

fn segment_local_progress(zone: &ZonePlan, segment_index: usize) -> f32 {
    let span = zone.end_segment.saturating_sub(zone.start_segment).max(1);
    (segment_index.saturating_sub(zone.start_segment)) as f32 / span as f32
}

fn push_zone_plan(
    rng: &mut RunRng,
    zones: &mut Vec<ZonePlan>,
    zone_edges: &mut Vec<ZoneEdge>,
    start_segment: usize,
    difficulty: f32,
    force_role: Option<ZoneRole>,
) -> usize {
    let zone_index = zones.len();
    let previous_role = zones.last().map(|zone| zone.role);
    let previous_signature = zones.last().map(|zone| zone.signature);
    let previous_biome = zones.last().map(|zone| zone.biome);
    let role = force_role.unwrap_or_else(|| choose_zone_role(rng, zone_index, previous_role));
    let signature = choose_zone_signature(rng, role, difficulty, previous_signature);
    let kind = legacy_zone_kind(signature, role);
    let biome = choose_biome_style(rng, difficulty, role, signature, previous_biome);
    let flow = zone_flow_profile(rng, role, signature, difficulty);
    let index = zone_index;
    let entry_connector = choose_zone_entry_connector(rng, role, signature, previous_role);
    let exit_connector = choose_zone_exit_connector(rng, role, signature);
    let end_segment = start_segment + zone_segment_span(rng, role, signature).saturating_sub(1);
    if let Some(previous) = zones.last() {
        zone_edges.push(ZoneEdge {
            from: previous.index,
            to: index,
            connector: entry_connector,
            branching: should_branch_zone(role, signature, flow, difficulty, rng),
        });
    }
    zones.push(ZonePlan {
        index,
        kind,
        role,
        signature,
        biome,
        start_segment,
        end_segment,
        flow,
        entry_connector,
        exit_connector,
        landmark: choose_landmark_kind(rng, role, signature),
        route_lines: major_zone_route_lines(role, signature),
    });
    index
}

fn finalize_zone_ranges(zones: &mut [ZonePlan], segment_len: usize) {
    let Some(last_segment) = segment_len.checked_sub(1) else {
        return;
    };
    for zone in zones {
        zone.end_segment = zone.end_segment.min(last_segment);
    }
}

fn zone_module_weight_bonus(
    kind: ModuleKind,
    zone_role: ZoneRole,
    zone_signature: ZoneSignature,
    connector: ConnectorKind,
    flow: FlowFieldProfile,
) -> u32 {
    let mut bonus = match zone_signature {
        ZoneSignature::WaveRamps => match kind {
            ModuleKind::SurfRamp => 9,
            ModuleKind::SpeedcheckRun => 7,
            ModuleKind::WindTunnel | ModuleKind::IceSpine => 5,
            ModuleKind::MovingPlatformRun => 3,
            ModuleKind::StairRun => 2,
            _ => 0,
        },
        ZoneSignature::BraidLanes => match kind {
            ModuleKind::StairRun => 7,
            ModuleKind::WindowHop => 6,
            ModuleKind::PillarAirstrafe => 5,
            ModuleKind::ShapeGauntlet => 4,
            ModuleKind::MovingPlatformRun => 2,
            _ => 0,
        },
        ZoneSignature::PillarForest => match kind {
            ModuleKind::PillarAirstrafe => 9,
            ModuleKind::MovingPlatformRun => 5,
            ModuleKind::StairRun => 4,
            ModuleKind::ShapeGauntlet => 4,
            _ => 0,
        },
        ZoneSignature::ShapeGarden => match kind {
            ModuleKind::ShapeGauntlet => 9,
            ModuleKind::StairRun => 5,
            ModuleKind::SpeedcheckRun => 4,
            ModuleKind::WindowHop => 3,
            _ => 0,
        },
        ZoneSignature::GiantCorkscrew => match kind {
            ModuleKind::SurfRamp => 8,
            ModuleKind::MovingPlatformRun => 4,
            ModuleKind::ShapeGauntlet => 4,
            ModuleKind::WallRunHall => 3,
            ModuleKind::LiftChasm => 3,
            _ => 0,
        },
        ZoneSignature::BowlCollector => match kind {
            ModuleKind::SurfRamp => 7,
            ModuleKind::StairRun => 5,
            ModuleKind::WaterGarden => 5,
            ModuleKind::MovingPlatformRun => 4,
            ModuleKind::CrumbleBridge => 3,
            _ => 0,
        },
        ZoneSignature::SplitTransfer => match kind {
            ModuleKind::WindowHop => 4,
            ModuleKind::PillarAirstrafe => 5,
            ModuleKind::SpeedcheckRun => 5,
            ModuleKind::MovingPlatformRun => 4,
            ModuleKind::ShapeGauntlet => 3,
            ModuleKind::SurfRamp => 3,
            _ => 0,
        },
    };

    bonus += match zone_role {
        ZoneRole::Accelerator => match kind {
            ModuleKind::SurfRamp | ModuleKind::SpeedcheckRun => 3,
            _ => 0,
        },
        ZoneRole::Technical => match kind {
            ModuleKind::WindowHop
            | ModuleKind::PillarAirstrafe
            | ModuleKind::HeadcheckRun
            | ModuleKind::ShapeGauntlet => 3,
            _ => 0,
        },
        ZoneRole::Recovery => match kind {
            ModuleKind::SurfRamp | ModuleKind::WaterGarden | ModuleKind::MovingPlatformRun => 2,
            _ => 0,
        },
        ZoneRole::Spectacle => match kind {
            ModuleKind::SurfRamp | ModuleKind::ShapeGauntlet | ModuleKind::MovingPlatformRun => 2,
            _ => 0,
        },
    };

    bonus += match connector {
        ConnectorKind::Funnel => match kind {
            ModuleKind::SurfRamp | ModuleKind::StairRun | ModuleKind::WaterGarden => 2,
            _ => 0,
        },
        ConnectorKind::Booster => match kind {
            ModuleKind::SurfRamp | ModuleKind::SpeedcheckRun | ModuleKind::WindTunnel => 3,
            _ => 0,
        },
        ConnectorKind::Transfer => match kind {
            ModuleKind::MovingPlatformRun | ModuleKind::SurfRamp | ModuleKind::SpeedcheckRun => 2,
            _ => 0,
        },
        ConnectorKind::Collector => match kind {
            ModuleKind::SurfRamp | ModuleKind::StairRun | ModuleKind::WaterGarden => 2,
            _ => 0,
        },
        ConnectorKind::Splitter | ConnectorKind::Crossover => match kind {
            ModuleKind::WindowHop
            | ModuleKind::PillarAirstrafe
            | ModuleKind::ShapeGauntlet
            | ModuleKind::MovingPlatformRun => 2,
            _ => 0,
        },
    };

    if flow.dynamic_bias > 0.22
        && matches!(
            kind,
            ModuleKind::MovingPlatformRun | ModuleKind::LiftChasm | ModuleKind::WindTunnel
        )
    {
        bonus += 2;
    }
    if flow.width_scale > 1.08 && matches!(kind, ModuleKind::SurfRamp | ModuleKind::ShapeGauntlet) {
        bonus += 1;
    }
    bonus
}

fn choose_zone_module_template(
    rng: &mut RunRng,
    current_socket: SocketMask,
    difficulty: f32,
    projected_gap: f32,
    zone: &ZonePlan,
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
                ModuleKind::SurfRamp
                    | ModuleKind::StairRun
                    | ModuleKind::WindowHop
                    | ModuleKind::PillarAirstrafe
                    | ModuleKind::HeadcheckRun
                    | ModuleKind::SpeedcheckRun
                    | ModuleKind::MovingPlatformRun
                    | ModuleKind::ShapeGauntlet
                    | ModuleKind::CrumbleBridge
                    | ModuleKind::WindTunnel
                    | ModuleKind::IceSpine
                    | ModuleKind::WallRunHall
            )
        {
            weight += 5;
        }
        if difficulty > 0.2 && matches!(template.kind, ModuleKind::SurfRamp) {
            weight += 3;
        }
        if difficulty > 0.35
            && matches!(template.kind, ModuleKind::StairRun | ModuleKind::WindowHop)
        {
            weight += 4;
        }
        if difficulty > 0.32
            && matches!(
                template.kind,
                ModuleKind::PillarAirstrafe
                    | ModuleKind::HeadcheckRun
                    | ModuleKind::SpeedcheckRun
                    | ModuleKind::MovingPlatformRun
                    | ModuleKind::ShapeGauntlet
            )
        {
            weight += 3;
        }
        if difficulty < 0.35
            && matches!(
                template.kind,
                ModuleKind::SurfRamp
                    | ModuleKind::StairRun
                    | ModuleKind::WindowHop
                    | ModuleKind::MantleStack
                    | ModuleKind::WaterGarden
            )
        {
            weight += 3;
        }
        if difficulty > 0.45
            && matches!(
                template.kind,
                ModuleKind::MantleStack | ModuleKind::LiftChasm | ModuleKind::WaterGarden
            )
        {
            weight = weight.saturating_sub(3).max(1);
        }
        weight += zone_module_weight_bonus(
            template.kind,
            zone.role,
            zone.signature,
            zone.entry_connector,
            zone.flow,
        );
        weighted.push((template, weight.max(1)));
    }

    if weighted.is_empty() {
        return module_template(safe_fallback_kind(difficulty));
    }

    rng.weighted_choice(&weighted)
}

fn route_turn_limit(kind: ModuleKind, difficulty: f32) -> f32 {
    let base = match kind {
        ModuleKind::SurfRamp => 15.5_f32.to_radians(),
        ModuleKind::StairRun | ModuleKind::ShapeGauntlet => 18.5_f32.to_radians(),
        ModuleKind::WindowHop | ModuleKind::PillarAirstrafe => 17.0_f32.to_radians(),
        ModuleKind::HeadcheckRun | ModuleKind::SpeedcheckRun => 15.0_f32.to_radians(),
        ModuleKind::MovingPlatformRun => 16.2_f32.to_radians(),
        ModuleKind::MantleStack | ModuleKind::WallRunHall => 11.2_f32.to_radians(),
        _ => 12.6_f32.to_radians(),
    };
    (base + difficulty * 3.6_f32.to_radians()).clamp(3.4_f32.to_radians(), MAX_SECTION_TURN_RADIANS)
}

fn choose_route_curve_archetype(
    rng: &mut RunRng,
    difficulty: f32,
    kind: ModuleKind,
) -> RouteCurveArchetype {
    let straight_weight = if matches!(kind, ModuleKind::SpeedcheckRun) {
        3
    } else {
        1
    };
    let carve_weight = if matches!(kind, ModuleKind::SurfRamp | ModuleKind::MovingPlatformRun) {
        7
    } else {
        4
    };
    let switch_weight = if matches!(
        kind,
        ModuleKind::StairRun
            | ModuleKind::WindowHop
            | ModuleKind::ShapeGauntlet
            | ModuleKind::PillarAirstrafe
    ) {
        7
    } else {
        3
    };
    let slalom_weight = if difficulty > 0.32 { 6 } else { 3 };
    rng.weighted_choice(&[
        (RouteCurveArchetype::Straight, straight_weight),
        (RouteCurveArchetype::Carve, carve_weight),
        (RouteCurveArchetype::Switchback, switch_weight),
        (RouteCurveArchetype::Slalom, slalom_weight),
    ])
}

fn route_turn_delta(
    rng: &mut RunRng,
    archetype: RouteCurveArchetype,
    index: usize,
    direction: f32,
    strength: f32,
    difficulty: f32,
    kind: ModuleKind,
) -> f32 {
    let style_bias = match kind {
        ModuleKind::SurfRamp => 1.08,
        ModuleKind::StairRun | ModuleKind::ShapeGauntlet => 1.28,
        ModuleKind::WindowHop | ModuleKind::PillarAirstrafe => 1.18,
        ModuleKind::MovingPlatformRun => 1.12,
        _ => 0.96,
    };
    let phase = index as f32 * (0.82 + difficulty * 0.48) + rng.range_f32(-0.15, 0.15);
    let amplitude = route_turn_limit(kind, difficulty) * strength * style_bias;
    let jitter = rng.range_f32(-1.3_f32.to_radians(), 1.3_f32.to_radians());

    match archetype {
        RouteCurveArchetype::Straight => {
            let drift = direction * amplitude * 0.12;
            let pulse = (phase * 0.45).sin() * amplitude * 0.16;
            drift + pulse + jitter * 0.4
        }
        RouteCurveArchetype::Carve => {
            let carve = direction * amplitude * rng.range_f32(0.84, 1.16);
            let pulse = (phase * 0.6).sin() * amplitude * 0.24;
            carve + pulse + jitter
        }
        RouteCurveArchetype::Switchback => {
            let sign = if index % 2 == 0 {
                direction
            } else {
                -direction
            };
            let shoulder = (phase * 0.55).sin() * amplitude * 0.22;
            sign * amplitude * rng.range_f32(0.88, 1.18) + shoulder + jitter
        }
        RouteCurveArchetype::Slalom => {
            let wave = (phase * 1.18).sin();
            let shaped = wave.signum() * wave.abs().powf(0.45);
            shaped * amplitude * 1.08 + jitter * 0.72
        }
    }
}

fn section_spacing_padding(kind: ModuleKind, difficulty: f32) -> f32 {
    let base = match kind {
        ModuleKind::SurfRamp => 56.0,
        ModuleKind::StairRun => 36.0,
        ModuleKind::ShapeGauntlet => 48.0,
        ModuleKind::PillarAirstrafe => 34.0,
        ModuleKind::WindowHop | ModuleKind::HeadcheckRun | ModuleKind::SpeedcheckRun => 28.0,
        ModuleKind::MovingPlatformRun => 32.0,
        ModuleKind::WallRunHall => 24.0,
        _ => 18.0,
    };
    base + difficulty * 16.0
}

fn room_generation_radius(size: Vec2, kind: ModuleKind, difficulty: f32) -> f32 {
    size.max_element() * 0.62 + section_spacing_padding(kind, difficulty)
}

fn candidate_room_clearance(
    candidate: Vec3,
    size: Vec2,
    kind: ModuleKind,
    difficulty: f32,
    rooms: &[RoomPlan],
) -> f32 {
    let candidate_radius = room_generation_radius(size, kind, difficulty);
    rooms
        .iter()
        .map(|room| {
            let existing_radius = room.size.max_element() * 0.58 + 14.0;
            Vec2::new(candidate.x - room.top.x, candidate.z - room.top.z).length()
                - (candidate_radius + existing_radius)
        })
        .fold(f32::INFINITY, f32::min)
}

fn choose_room_candidate(
    rng: &mut RunRng,
    current_top: Vec3,
    current_height: f32,
    desired_heading_angle: f32,
    step_distance: f32,
    room_size: Vec2,
    kind: ModuleKind,
    difficulty: f32,
    rooms: &[RoomPlan],
    occupied_rooms: &HashSet<IVec2>,
) -> (Vec3, IVec2, f32) {
    let turn_limit = route_turn_limit(kind, difficulty);
    let bend_strength = match kind {
        ModuleKind::SurfRamp => 0.12 + difficulty * 0.08,
        ModuleKind::StairRun | ModuleKind::ShapeGauntlet => 0.18 + difficulty * 0.1,
        ModuleKind::WindowHop | ModuleKind::PillarAirstrafe => 0.14 + difficulty * 0.09,
        ModuleKind::HeadcheckRun | ModuleKind::SpeedcheckRun | ModuleKind::MovingPlatformRun => {
            0.16 + difficulty * 0.1
        }
        _ => 0.1 + difficulty * 0.06,
    };
    let turn_offsets = [0.0, 0.18, -0.18, 0.35, -0.35, 0.55, -0.55, 0.78, -0.78];
    let distance_pushes = [0.0, CELL_SIZE * 0.55, CELL_SIZE * 1.05, CELL_SIZE * 1.6];
    let bend_offsets = [0.0, 0.45, -0.45, 0.95, -0.95];
    let mut best = (
        current_top
            + Vec3::new(
                desired_heading_angle.cos(),
                0.0,
                desired_heading_angle.sin(),
            ) * step_distance,
        room_grid_cell(current_top),
        desired_heading_angle,
    );
    let mut best_score = f32::NEG_INFINITY;

    for turn_offset in turn_offsets {
        let heading_angle =
            desired_heading_angle + turn_offset * turn_limit * rng.range_f32(0.9, 1.08);
        let heading = Vec3::new(heading_angle.cos(), 0.0, heading_angle.sin()).normalize_or_zero();
        let right = Vec3::new(-heading.z, 0.0, heading.x);
        for distance_push in distance_pushes {
            for bend_offset in bend_offsets {
                let bend =
                    right * bend_offset * bend_strength * step_distance * rng.range_f32(0.85, 1.12);
                let mut top = current_top + heading * (step_distance + distance_push) + bend;
                top.y = current_height;
                let cell = room_grid_cell(top);
                let clearance = candidate_room_clearance(top, room_size, kind, difficulty, rooms);
                let occupied_penalty = if occupied_rooms.contains(&cell) {
                    180.0
                } else {
                    0.0
                };
                let score = clearance - occupied_penalty - distance_push * 0.06;
                if score > best_score {
                    best = (top, cell, heading_angle);
                    best_score = score;
                }
                if clearance >= 10.0 && !occupied_rooms.contains(&cell) {
                    return (top, cell, heading_angle);
                }
            }
        }
    }

    if best_score < 0.0 {
        let heading = Vec3::new(best.2.cos(), 0.0, best.2.sin()).normalize_or_zero();
        let push = -best_score + 12.0;
        best.0 += heading * push;
        best.1 = room_grid_cell(best.0);
    }

    best
}

fn segment_route_lines_for_zone(zone: &ZonePlan, zone_t: f32) -> Vec<RouteLine> {
    let interior = zone_t >= 0.12 && zone_t <= 0.88;
    if interior {
        return zone.route_lines.clone();
    }

    match zone.role {
        ZoneRole::Recovery => vec![RouteLine::Safe, RouteLine::Speed],
        ZoneRole::Spectacle if zone_t < 0.18 => vec![RouteLine::Speed, RouteLine::Trick],
        _ => vec![RouteLine::Speed],
    }
}

fn build_run_blueprint(seed: u64) -> RunBlueprint {
    let mut best_blueprint = None;
    let mut best_score = usize::MAX;

    for attempt in 0..18 {
        let mut rng = RunRng::new(seed ^ (attempt as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let graph = draft_course_graph(seed, &mut rng);
        let mut blueprint = compile_course_graph(graph);
        let repair_stats = repair_run_blueprint(&mut blueprint, &mut rng);
        force_first_segment_to_surf(&mut blueprint);
        let validation = validate_run_blueprint(&blueprint);

        blueprint.stats.attempts = attempt + 1;
        blueprint.stats.repairs = repair_stats.repairs;
        blueprint.stats.downgraded_segments = repair_stats.downgraded_segments;
        blueprint.stats.overlap_issues = validation.overlap_issues;
        blueprint.stats.clearance_issues = validation.clearance_issues;
        blueprint.stats.reachability_issues = validation.reachability_issues;

        let score = validation.overlap_issues * 10
            + validation.clearance_issues * 8
            + validation.reachability_issues * 6;

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
}

fn draft_course_graph(seed: u64, rng: &mut RunRng) -> CourseGraph {
    let floors = rng.range_usize(34, 49);
    let mut rooms = Vec::with_capacity(floors);
    let mut segments = Vec::with_capacity(floors.saturating_sub(1));
    let mut zones = Vec::new();
    let mut zone_edges = Vec::new();
    let mut occupied_rooms: HashSet<IVec2> = HashSet::default();
    let theme_offset = (rng.next_u64() % 4) as usize;

    let mut current_socket = SOCKET_SAFE_REST;
    let mut current_height = 420.0;
    let mut current_top = Vec3::new(0.0, current_height, 0.0);
    let mut heading_angle = rng.range_f32(0.0, TAU);
    let active_zone_index = push_zone_plan(
        rng,
        &mut zones,
        &mut zone_edges,
        0,
        0.0,
        Some(ZoneRole::Accelerator),
    );
    let active_zone = zones[active_zone_index].clone();

    let cell = room_grid_cell(current_top);
    occupied_rooms.insert(cell);
    let first_room_seed = rng.next_u64();
    rooms.push(RoomPlan {
        index: 0,
        cell,
        top: current_top,
        size: Vec2::splat(13.5),
        theme: biome_theme(active_zone.biome, 0, floors, theme_offset),
        seed: first_room_seed,
        section: RoomSectionKind::OpenPad,
        checkpoint_slot: Some(0),
        biome: active_zone.biome,
        zone_index: active_zone_index,
        layer_index: zone_layer_index(
            active_zone.signature,
            active_zone.flow,
            first_room_seed,
            0.0,
        ),
    });

    let mut active_zone_index = active_zone_index;
    for index in 1..floors {
        let difficulty = index as f32 / (floors.saturating_sub(1).max(1)) as f32;
        let segment_index = index - 1;
        if segment_index > zones[active_zone_index].end_segment {
            active_zone_index = push_zone_plan(
                rng,
                &mut zones,
                &mut zone_edges,
                segment_index,
                difficulty,
                None,
            );
        }
        let active_zone = zones[active_zone_index].clone();
        let zone_t = segment_local_progress(&active_zone, segment_index);
        let flow_seed = seed ^ active_zone.index as u64 ^ 0xF10F_1E1D_u64;
        let flow_sample = sample_flow_field(active_zone.flow, flow_seed, zone_t, 1.0);
        let room_size = (Vec2::splat(lerp(13.2, 9.8, difficulty))
            * flow_sample.width_scale.clamp(0.84, 1.28))
        .max(Vec2::splat(9.2));
        let mut step_distance = rng.range_f32(CELL_SIZE * 5.8, CELL_SIZE * 8.4);
        let projected_gap = projected_gap(step_distance, rooms.last().unwrap().size, room_size);
        let template = if index == 1 {
            module_template(ModuleKind::SurfRamp)
        } else {
            choose_zone_module_template(
                rng,
                current_socket,
                difficulty,
                projected_gap,
                &active_zone,
            )
        };
        step_distance = match template.kind {
            ModuleKind::SurfRamp => rng.range_f32(CELL_SIZE * 14.0, CELL_SIZE * 22.0),
            ModuleKind::StairRun => rng.range_f32(CELL_SIZE * 9.0, CELL_SIZE * 15.0),
            ModuleKind::WindowHop => rng.range_f32(CELL_SIZE * 7.0, CELL_SIZE * 10.5),
            ModuleKind::PillarAirstrafe => rng.range_f32(CELL_SIZE * 8.0, CELL_SIZE * 12.5),
            ModuleKind::HeadcheckRun => rng.range_f32(CELL_SIZE * 7.0, CELL_SIZE * 10.0),
            ModuleKind::SpeedcheckRun => rng.range_f32(CELL_SIZE * 8.0, CELL_SIZE * 12.0),
            ModuleKind::MovingPlatformRun => rng.range_f32(CELL_SIZE * 7.5, CELL_SIZE * 11.5),
            ModuleKind::ShapeGauntlet => rng.range_f32(CELL_SIZE * 9.0, CELL_SIZE * 13.5),
            ModuleKind::IceSpine | ModuleKind::CrumbleBridge | ModuleKind::WindTunnel => {
                rng.range_f32(CELL_SIZE * 7.0, CELL_SIZE * 11.5)
            }
            ModuleKind::WallRunHall => rng.range_f32(CELL_SIZE * 6.0, CELL_SIZE * 9.0),
            ModuleKind::MantleStack | ModuleKind::LiftChasm | ModuleKind::WaterGarden => {
                rng.range_f32(CELL_SIZE * 5.0, CELL_SIZE * 8.0)
            }
        };
        let distance_multiplier = match active_zone.flow.route_curve {
            RouteCurveArchetype::Straight => 1.18,
            RouteCurveArchetype::Carve => 1.04,
            RouteCurveArchetype::Switchback => 0.96,
            RouteCurveArchetype::Slalom => 1.0,
        };
        let flow_distance_scale = (active_zone.flow.width_scale
            * (0.9 + active_zone.flow.curvature * 0.08))
            .clamp(0.88, 1.34);
        step_distance *= distance_multiplier * flow_distance_scale;
        step_distance += flow_sample.lateral.abs() * CELL_SIZE * 0.22;
        let descent = rng.range_f32(template.min_rise, template.max_rise)
            + lerp(8.5, 15.0, difficulty)
            + difficulty * 4.8
            + active_zone.flow.vertical_wave * 9.0
            + flow_sample.vertical.abs() * 6.0;
        current_height -= descent;
        let zone_turn_limit = route_turn_limit(template.kind, difficulty)
            * active_zone.flow.curvature.clamp(0.85, 1.28);
        heading_angle += route_turn_delta(
            rng,
            active_zone.flow.route_curve,
            index,
            active_zone.flow.drift_sign,
            active_zone.flow.curvature,
            difficulty,
            template.kind,
        )
        .clamp(-zone_turn_limit, zone_turn_limit)
            + flow_sample.lateral.signum()
                * flow_sample.lateral.abs().min(1.0)
                * 1.4_f32.to_radians();
        let (top, cell, settled_heading_angle) = choose_room_candidate(
            rng,
            current_top,
            current_height,
            heading_angle,
            step_distance,
            room_size,
            template.kind,
            difficulty,
            &rooms,
            &occupied_rooms,
        );
        heading_angle = settled_heading_angle;
        occupied_rooms.insert(cell);
        current_top = top;

        if is_primary_speed_section(template.kind) {
            if let Some(previous_room) = rooms.last_mut() {
                previous_room.section = RoomSectionKind::OpenPad;
                previous_room.size = previous_room
                    .size
                    .max(Vec2::splat(speed_room_size(template.kind)));
            }
        }

        rooms.push(RoomPlan {
            index,
            cell,
            top,
            size: if index == floors - 1 {
                Vec2::splat(15.2)
            } else if is_primary_speed_section(template.kind) {
                room_size.max(Vec2::splat(
                    speed_room_size(template.kind) * flow_sample.width_scale.clamp(0.96, 1.24),
                ))
            } else {
                room_size
            },
            theme: biome_theme(active_zone.biome, index, floors, theme_offset),
            seed: rng.next_u64(),
            section: if is_primary_speed_section(template.kind) {
                RoomSectionKind::OpenPad
            } else {
                choose_room_section(rng, difficulty, index, floors)
            },
            checkpoint_slot: Some(index),
            biome: active_zone.biome,
            zone_index: active_zone_index,
            layer_index: zone_layer_index(
                active_zone.signature,
                active_zone.flow,
                flow_seed,
                zone_t,
            ),
        });

        segments.push(SegmentPlan {
            index: segment_index,
            from: segment_index,
            to: index,
            kind: template.kind,
            difficulty,
            seed: rng.next_u64(),
            zone_role: active_zone.role,
            zone_signature: active_zone.signature,
            exit_socket: template.exit,
            zone_index: active_zone_index,
            biome: active_zone.biome,
            connector: if segment_index == active_zone.start_segment {
                active_zone.entry_connector
            } else if segment_index == active_zone.end_segment {
                active_zone.exit_connector
            } else {
                ConnectorKind::Transfer
            },
            flow: active_zone.flow,
            route_lines: segment_route_lines_for_zone(&active_zone, zone_t),
            zone_local_t: zone_t,
        });
        current_socket = template.exit;
    }

    finalize_zone_ranges(&mut zones, segments.len());
    CourseGraph {
        seed,
        rooms,
        segments,
        zones,
        zone_edges,
    }
}

fn compile_course_graph(graph: CourseGraph) -> RunBlueprint {
    let mut blueprint = RunBlueprint {
        seed: graph.seed,
        spawn: graph.rooms[0].top + Vec3::new(0.0, PLAYER_SPAWN_CLEARANCE, graph.rooms[0].size.y * 0.18),
        summit: graph.rooms.last().unwrap().top + Vec3::new(0.0, 1.4, 0.0),
        death_plane: graph.rooms.last().unwrap().top.y - 90.0,
        rooms: graph.rooms,
        segments: graph.segments,
        zones: graph.zones,
        zone_edges: graph.zone_edges,
        stats: GenerationStats::default(),
    };
    force_first_segment_to_surf(&mut blueprint);
    blueprint
}

fn build_course_graph(seed: u64) -> CourseGraph {
    let mut rng = RunRng::new(seed ^ 0x9E37_79B9_7F4A_7C15);
    draft_course_graph(seed, &mut rng)
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
    force_first_segment_to_surf(blueprint);

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
                force_first_segment_to_surf(blueprint);
                continue;
            }
        }

        if let Some((a, b)) = validation.first_overlap.or(validation.first_clearance) {
            if spread_room_owner(blueprint, a, 10.0) || spread_room_owner(blueprint, b, 10.0) {
                stats.repairs += 1;
                continue;
            }

            if trim_segment_route_lines(blueprint, a) || trim_segment_route_lines(blueprint, b) {
                stats.repairs += 1;
                continue;
            }

            if downgrade_segment_owner(blueprint, a, rng)
                || downgrade_segment_owner(blueprint, b, rng)
            {
                stats.repairs += 1;
                stats.downgraded_segments += 1;
                force_first_segment_to_surf(blueprint);
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
    let graph = draft_course_graph(seed, &mut rng);
    let mut blueprint = compile_course_graph(graph);
    for segment in &mut blueprint.segments {
        segment.kind = if segment.difficulty > 0.55 {
            ModuleKind::SurfRamp
        } else {
            ModuleKind::StairRun
        };
        segment.exit_socket = module_template(segment.kind).exit;
    }
    force_first_segment_to_surf(&mut blueprint);
    let validation = validate_run_blueprint(&blueprint);
    blueprint.stats.attempts = 1;
    blueprint.stats.overlap_issues = validation.overlap_issues;
    blueprint.stats.clearance_issues = validation.clearance_issues;
    blueprint.stats.reachability_issues = validation.reachability_issues;
    blueprint
}

fn force_first_segment_to_surf(blueprint: &mut RunBlueprint) {
    let Some(first_segment) = blueprint.segments.first_mut() else {
        return;
    };
    first_segment.kind = ModuleKind::SurfRamp;
    first_segment.exit_socket = module_template(ModuleKind::SurfRamp).exit;

    if let Some(first_room) = blueprint.rooms.get_mut(0) {
        first_room.section = RoomSectionKind::OpenPad;
        first_room.size = first_room
            .size
            .max(Vec2::splat(speed_room_size(ModuleKind::SurfRamp)));
    }
    if let Some(second_room) = blueprint.rooms.get_mut(1) {
        second_room.section = RoomSectionKind::OpenPad;
        second_room.size = second_room
            .size
            .max(Vec2::splat(speed_room_size(ModuleKind::SurfRamp)));
    }
}

fn theme_offset_from_theme(theme: Theme) -> usize {
    match theme {
        Theme::Stone => 0,
        Theme::Overgrown => 1,
        Theme::Frost => 2,
        Theme::Ember => 3,
    }
}

fn tail_heading_angle(rooms: &[RoomPlan]) -> f32 {
    if rooms.len() < 2 {
        return 0.0;
    }

    let heading = direction_from_delta(rooms[rooms.len() - 1].top - rooms[rooms.len() - 2].top);
    if heading == Vec3::ZERO {
        0.0
    } else {
        heading.z.atan2(heading.x)
    }
}

fn endless_difficulty(index: usize) -> f32 {
    (index as f32 / 28.0).clamp(0.0, 1.0)
}

fn append_run_blueprint(blueprint: &mut RunBlueprint, additional_rooms: usize) {
    if additional_rooms == 0 || blueprint.rooms.is_empty() {
        return;
    }

    let start_index = blueprint.rooms.len();
    let total_after_append = start_index + additional_rooms;
    let theme_offset = blueprint
        .rooms
        .first()
        .map(|room| theme_offset_from_theme(room.theme))
        .unwrap_or(0);
    let mut occupied_rooms = blueprint
        .rooms
        .iter()
        .map(|room| room.cell)
        .collect::<HashSet<_>>();

    let mut rng =
        RunRng::new(blueprint.seed ^ (start_index as u64 + 1).wrapping_mul(0xD1B5_4A32_D192_ED03));
    let mut current_socket = blueprint
        .segments
        .last()
        .map(|segment| segment.exit_socket)
        .unwrap_or(SOCKET_SAFE_REST);
    let mut current_top = blueprint.rooms.last().unwrap().top;
    let mut current_height = current_top.y;
    let mut heading_angle = tail_heading_angle(&blueprint.rooms);
    let tail_difficulty = blueprint
        .segments
        .last()
        .map(|segment| segment.difficulty)
        .unwrap_or(0.0);
    let mut active_zone_index = push_zone_plan(
        &mut rng,
        &mut blueprint.zones,
        &mut blueprint.zone_edges,
        blueprint.segments.len(),
        tail_difficulty,
        None,
    );

    for index in start_index..total_after_append {
        let difficulty = endless_difficulty(index);
        let segment_index = index - 1;
        if segment_index > blueprint.zones[active_zone_index].end_segment {
            active_zone_index = push_zone_plan(
                &mut rng,
                &mut blueprint.zones,
                &mut blueprint.zone_edges,
                segment_index,
                difficulty,
                None,
            );
        }
        let active_zone = blueprint.zones[active_zone_index].clone();
        let zone_t = segment_local_progress(&active_zone, segment_index);
        let flow_seed = blueprint.seed ^ active_zone.index as u64 ^ 0xABCD_9876_u64;
        let flow_sample = sample_flow_field(active_zone.flow, flow_seed, zone_t, 1.0);
        let room_size = (Vec2::splat(lerp(13.2, 9.8, difficulty))
            * flow_sample.width_scale.clamp(0.84, 1.28))
        .max(Vec2::splat(9.2));
        let mut step_distance = rng.range_f32(CELL_SIZE * 5.8, CELL_SIZE * 8.4);
        let projected_gap = projected_gap(
            step_distance,
            blueprint.rooms.last().unwrap().size,
            room_size,
        );
        let template = choose_zone_module_template(
            &mut rng,
            current_socket,
            difficulty,
            projected_gap,
            &active_zone,
        );
        step_distance = match template.kind {
            ModuleKind::SurfRamp => rng.range_f32(CELL_SIZE * 14.0, CELL_SIZE * 22.0),
            ModuleKind::StairRun => rng.range_f32(CELL_SIZE * 9.0, CELL_SIZE * 15.0),
            ModuleKind::WindowHop => rng.range_f32(CELL_SIZE * 7.0, CELL_SIZE * 10.5),
            ModuleKind::PillarAirstrafe => rng.range_f32(CELL_SIZE * 8.0, CELL_SIZE * 12.5),
            ModuleKind::HeadcheckRun => rng.range_f32(CELL_SIZE * 7.0, CELL_SIZE * 10.0),
            ModuleKind::SpeedcheckRun => rng.range_f32(CELL_SIZE * 8.0, CELL_SIZE * 12.0),
            ModuleKind::MovingPlatformRun => rng.range_f32(CELL_SIZE * 7.5, CELL_SIZE * 11.5),
            ModuleKind::ShapeGauntlet => rng.range_f32(CELL_SIZE * 9.0, CELL_SIZE * 13.5),
            ModuleKind::IceSpine | ModuleKind::CrumbleBridge | ModuleKind::WindTunnel => {
                rng.range_f32(CELL_SIZE * 7.0, CELL_SIZE * 11.5)
            }
            ModuleKind::WallRunHall => rng.range_f32(CELL_SIZE * 6.0, CELL_SIZE * 9.0),
            ModuleKind::MantleStack | ModuleKind::LiftChasm | ModuleKind::WaterGarden => {
                rng.range_f32(CELL_SIZE * 5.0, CELL_SIZE * 8.0)
            }
        };
        let distance_multiplier = match active_zone.flow.route_curve {
            RouteCurveArchetype::Straight => 1.18,
            RouteCurveArchetype::Carve => 1.04,
            RouteCurveArchetype::Switchback => 0.96,
            RouteCurveArchetype::Slalom => 1.0,
        };
        let flow_distance_scale = (active_zone.flow.width_scale
            * (0.9 + active_zone.flow.curvature * 0.08))
            .clamp(0.88, 1.34);
        step_distance *= distance_multiplier * flow_distance_scale;
        step_distance += flow_sample.lateral.abs() * CELL_SIZE * 0.22;
        let descent = rng.range_f32(template.min_rise, template.max_rise)
            + lerp(8.5, 15.0, difficulty)
            + difficulty * 4.8
            + active_zone.flow.vertical_wave * 9.0
            + flow_sample.vertical.abs() * 6.0;
        current_height -= descent;
        let zone_turn_limit = route_turn_limit(template.kind, difficulty)
            * active_zone.flow.curvature.clamp(0.85, 1.28);
        heading_angle += route_turn_delta(
            &mut rng,
            active_zone.flow.route_curve,
            index,
            active_zone.flow.drift_sign,
            active_zone.flow.curvature,
            difficulty,
            template.kind,
        )
        .clamp(-zone_turn_limit, zone_turn_limit)
            + flow_sample.lateral.signum()
                * flow_sample.lateral.abs().min(1.0)
                * 1.4_f32.to_radians();
        let (top, cell, settled_heading_angle) = choose_room_candidate(
            &mut rng,
            current_top,
            current_height,
            heading_angle,
            step_distance,
            room_size,
            template.kind,
            difficulty,
            &blueprint.rooms,
            &occupied_rooms,
        );
        heading_angle = settled_heading_angle;
        occupied_rooms.insert(cell);
        current_top = top;

        if is_primary_speed_section(template.kind) {
            if let Some(previous_room) = blueprint.rooms.last_mut() {
                previous_room.section = RoomSectionKind::OpenPad;
                previous_room.size = previous_room
                    .size
                    .max(Vec2::splat(speed_room_size(template.kind)));
            }
        }

        blueprint.rooms.push(RoomPlan {
            index,
            cell,
            top,
            size: if is_primary_speed_section(template.kind) {
                room_size.max(Vec2::splat(
                    speed_room_size(template.kind) * flow_sample.width_scale.clamp(0.96, 1.24),
                ))
            } else {
                room_size
            },
            theme: biome_theme(active_zone.biome, index, total_after_append, theme_offset),
            seed: rng.next_u64(),
            section: if is_primary_speed_section(template.kind) {
                RoomSectionKind::OpenPad
            } else {
                choose_room_section(&mut rng, difficulty, index, total_after_append)
            },
            checkpoint_slot: Some(index),
            biome: active_zone.biome,
            zone_index: active_zone_index,
            layer_index: zone_layer_index(
                active_zone.signature,
                active_zone.flow,
                flow_seed,
                zone_t,
            ),
        });

        let mut segment = SegmentPlan {
            index: segment_index,
            from: segment_index,
            to: index,
            kind: template.kind,
            difficulty,
            seed: rng.next_u64(),
            zone_role: active_zone.role,
            zone_signature: active_zone.signature,
            exit_socket: template.exit,
            zone_index: active_zone_index,
            biome: active_zone.biome,
            connector: if segment_index == active_zone.start_segment {
                active_zone.entry_connector
            } else if segment_index == active_zone.end_segment {
                active_zone.exit_connector
            } else {
                ConnectorKind::Transfer
            },
            flow: active_zone.flow,
            route_lines: segment_route_lines_for_zone(&active_zone, zone_t),
            zone_local_t: zone_t,
        };
        if !segment_reachable(&segment, &blueprint.rooms) {
            segment.kind = safe_fallback_kind(segment.difficulty);
            segment.exit_socket = module_template(segment.kind).exit;
        }
        current_socket = segment.exit_socket;
        blueprint.segments.push(segment);
    }

    finalize_zone_ranges(&mut blueprint.zones, blueprint.segments.len());
    if let Some(last_room) = blueprint.rooms.last() {
        blueprint.summit = last_room.top + Vec3::Y * 1.4;
        blueprint.death_plane = last_room.top.y - 90.0;
    }
}

fn choose_room_section(
    rng: &mut RunRng,
    difficulty: f32,
    index: usize,
    floors: usize,
) -> RoomSectionKind {
    if index == 0 || index + 1 == floors {
        return RoomSectionKind::OpenPad;
    }

    let roll = rng.range_f32(0.0, 1.0);
    if difficulty < 0.22 || roll < 0.28 {
        RoomSectionKind::OpenPad
    } else if roll < 0.53 {
        RoomSectionKind::Terrace
    } else if roll < 0.78 {
        RoomSectionKind::SplitPad
    } else {
        RoomSectionKind::CornerPerches
    }
}

fn validate_run_blueprint(blueprint: &RunBlueprint) -> ValidationOutcome {
    let mut volumes = Vec::new();
    let mut clearances = Vec::new();
    let mut outcome = ValidationOutcome::default();

    for room in &blueprint.rooms {
        let layout = build_room_layout(room);
        collect_internal_route_line_overlaps(&layout, &mut outcome);
        collect_layout_validation(&layout, &mut volumes, &mut clearances);
    }
    for segment in &blueprint.segments {
        let layout = build_segment_layout(segment, &blueprint.rooms);
        collect_internal_route_line_overlaps(&layout, &mut outcome);
        collect_layout_validation(&layout, &mut volumes, &mut clearances);
    }
    let layout = build_summit_layout(blueprint.rooms.last().unwrap(), blueprint.summit);
    collect_internal_route_line_overlaps(&layout, &mut outcome);
    collect_layout_validation(&layout, &mut volumes, &mut clearances);

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

fn layout_has_internal_route_line_overlap(layout: &ModuleLayout) -> bool {
    let volumes = layout
        .solids
        .iter()
        .enumerate()
        .filter_map(|(index, solid)| solid.preview_volume().map(|volume| (index, volume)))
        .collect::<Vec<_>>();

    for i in 0..volumes.len() {
        for j in i + 1..volumes.len() {
            let (solid_i, volume_i) = volumes[i];
            let (solid_j, volume_j) = volumes[j];
            let line_i = route_line_from_label(&layout.solids[solid_i].label);
            let line_j = route_line_from_label(&layout.solids[solid_j].label);
            if line_i.is_none() || line_j.is_none() || line_i == line_j {
                continue;
            }
            if intersects(volume_i, volume_j, 0.08) {
                return true;
            }
        }
    }
    false
}

fn collect_internal_route_line_overlaps(layout: &ModuleLayout, outcome: &mut ValidationOutcome) {
    if layout_has_internal_route_line_overlap(layout) {
        outcome.overlap_issues += 1;
        if let Some(owner) = layout.solids.first().map(|solid| solid.owner) {
            outcome.first_overlap.get_or_insert((owner, owner));
        }
    }
}

impl SolidSpec {
    fn preview_volume(&self) -> Option<AabbVolume> {
        let size = match &self.body {
            SolidBody::Static
            | SolidBody::StaticSphere
            | SolidBody::StaticCylinder
            | SolidBody::StaticTrapezoid { .. }
            | SolidBody::Crumbling { .. } => self.size,
            SolidBody::StaticSurfWedge { render_points, .. } => {
                if render_points.is_empty() {
                    return None;
                }
                let (min, max) =
                    transformed_point_bounds(self.center, Quat::IDENTITY, render_points);
                return Some(AabbVolume {
                    owner: self.owner,
                    center: (min + max) * 0.5,
                    size: max - min,
                });
            }
            SolidBody::StaticSurfStrip {
                collider_strip_points,
                ..
            } => {
                if collider_strip_points.is_empty() {
                    return None;
                }
                let (min, max) =
                    transformed_point_bounds(self.center, Quat::IDENTITY, collider_strip_points);
                return Some(AabbVolume {
                    owner: self.owner,
                    center: (min + max) * 0.5,
                    size: max - min,
                });
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
    true
}

fn trim_segment_route_lines(blueprint: &mut RunBlueprint, owner: OwnerTag) -> bool {
    let OwnerTag::Segment(index) = owner else {
        return false;
    };
    let Some(segment) = blueprint.segments.get_mut(index) else {
        return false;
    };
    if segment.route_lines.len() <= 1 {
        return false;
    }

    if let Some(position) = segment
        .route_lines
        .iter()
        .position(|line| matches!(line, RouteLine::Trick))
    {
        segment.route_lines.remove(position);
        return true;
    }
    if let Some(position) = segment
        .route_lines
        .iter()
        .position(|line| matches!(line, RouteLine::Safe))
    {
        segment.route_lines.remove(position);
        return true;
    }

    segment.route_lines.truncate(1);
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

fn spread_room_owner(blueprint: &mut RunBlueprint, owner: OwnerTag, amount: f32) -> bool {
    let index = match owner {
        OwnerTag::Room(index) => index,
        OwnerTag::Segment(index) => index.saturating_add(1),
        _ => return false,
    };
    if index == 0 || index >= blueprint.rooms.len() {
        return false;
    }

    let forward = if index + 1 < blueprint.rooms.len() {
        direction_from_delta(blueprint.rooms[index + 1].top - blueprint.rooms[index - 1].top)
    } else {
        direction_from_delta(blueprint.rooms[index].top - blueprint.rooms[index - 1].top)
    };
    if forward == Vec3::ZERO {
        return false;
    }
    let right = Vec3::new(-forward.z, 0.0, forward.x);
    let sign = if index % 2 == 0 { 1.0 } else { -1.0 };
    let shift = right * amount * sign + forward * (amount * 0.4);

    for room in blueprint.rooms.iter_mut().skip(index) {
        room.top += shift;
        room.cell = room_grid_cell(room.top);
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
    let rise = (to.top.y - from.top.y).abs();
    let gap = edge_gap(from, to);
    rise >= template.min_rise - 0.2
        && rise <= template.max_rise + 0.9
        && gap >= template.min_gap - 1.4
        && gap <= template.max_gap + 1.8
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
                ModuleKind::SurfRamp
                    | ModuleKind::StairRun
                    | ModuleKind::WindowHop
                    | ModuleKind::PillarAirstrafe
                    | ModuleKind::HeadcheckRun
                    | ModuleKind::SpeedcheckRun
                    | ModuleKind::MovingPlatformRun
                    | ModuleKind::ShapeGauntlet
                    | ModuleKind::CrumbleBridge
                    | ModuleKind::WindTunnel
                    | ModuleKind::IceSpine
                    | ModuleKind::WallRunHall
            )
        {
            weight += 5;
        }
        if difficulty > 0.2 && matches!(template.kind, ModuleKind::SurfRamp) {
            weight += 3;
        }
        if difficulty > 0.35
            && matches!(template.kind, ModuleKind::StairRun | ModuleKind::WindowHop)
        {
            weight += 4;
        }
        if difficulty > 0.32
            && matches!(
                template.kind,
                ModuleKind::PillarAirstrafe
                    | ModuleKind::HeadcheckRun
                    | ModuleKind::SpeedcheckRun
                    | ModuleKind::MovingPlatformRun
                    | ModuleKind::ShapeGauntlet
            )
        {
            weight += 3;
        }
        if difficulty < 0.35
            && matches!(
                template.kind,
                ModuleKind::SurfRamp
                    | ModuleKind::StairRun
                    | ModuleKind::WindowHop
                    | ModuleKind::MantleStack
                    | ModuleKind::WaterGarden
            )
        {
            weight += 3;
        }
        if difficulty > 0.45
            && matches!(
                template.kind,
                ModuleKind::MantleStack | ModuleKind::LiftChasm | ModuleKind::WaterGarden
            )
        {
            weight = weight.saturating_sub(3).max(1);
        }
        weighted.push((template, weight));
    }

    if weighted.is_empty() {
        return module_template(safe_fallback_kind(difficulty));
    }

    rng.weighted_choice(&weighted)
}

fn all_templates() -> Vec<ModuleTemplate> {
    vec![
        module_template(ModuleKind::StairRun),
        module_template(ModuleKind::SurfRamp),
        module_template(ModuleKind::WindowHop),
        module_template(ModuleKind::PillarAirstrafe),
        module_template(ModuleKind::HeadcheckRun),
        module_template(ModuleKind::SpeedcheckRun),
        module_template(ModuleKind::MovingPlatformRun),
        module_template(ModuleKind::ShapeGauntlet),
        module_template(ModuleKind::MantleStack),
        module_template(ModuleKind::WallRunHall),
        module_template(ModuleKind::LiftChasm),
        module_template(ModuleKind::CrumbleBridge),
        module_template(ModuleKind::WindTunnel),
        module_template(ModuleKind::IceSpine),
        module_template(ModuleKind::WaterGarden),
        module_template(ModuleKind::StairRun),
        module_template(ModuleKind::SurfRamp),
        module_template(ModuleKind::ShapeGauntlet),
    ]
}

fn module_template(kind: ModuleKind) -> ModuleTemplate {
    match kind {
        ModuleKind::StairRun => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_SHORTCUT_ANCHOR,
            exit: SOCKET_SAFE_REST | SOCKET_MANTLE_ENTRY,
            min_difficulty: 0.0,
            max_difficulty: 1.0,
            weight: 5,
            min_rise: 20.0,
            max_rise: 58.0,
            min_gap: 26.0,
            max_gap: 320.0,
        },
        ModuleKind::SurfRamp => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_SHORTCUT_ANCHOR,
            exit: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            min_difficulty: 0.12,
            max_difficulty: 1.0,
            weight: 7,
            min_rise: 38.0,
            max_rise: 150.0,
            min_gap: 40.0,
            max_gap: 520.0,
        },
        ModuleKind::WindowHop => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_SHORTCUT_ANCHOR,
            exit: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            min_difficulty: 0.1,
            max_difficulty: 0.86,
            weight: 4,
            min_rise: 14.0,
            max_rise: 34.0,
            min_gap: 20.0,
            max_gap: 88.0,
        },
        ModuleKind::PillarAirstrafe => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            exit: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            min_difficulty: 0.18,
            max_difficulty: 1.0,
            weight: 4,
            min_rise: 18.0,
            max_rise: 48.0,
            min_gap: 24.0,
            max_gap: 128.0,
        },
        ModuleKind::HeadcheckRun => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            exit: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            min_difficulty: 0.16,
            max_difficulty: 0.92,
            weight: 3,
            min_rise: 16.0,
            max_rise: 40.0,
            min_gap: 20.0,
            max_gap: 92.0,
        },
        ModuleKind::SpeedcheckRun => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            exit: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH | SOCKET_SHORTCUT_ANCHOR,
            min_difficulty: 0.28,
            max_difficulty: 1.0,
            weight: 4,
            min_rise: 18.0,
            max_rise: 54.0,
            min_gap: 24.0,
            max_gap: 128.0,
        },
        ModuleKind::MovingPlatformRun => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH | SOCKET_SHORTCUT_ANCHOR,
            exit: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            min_difficulty: 0.2,
            max_difficulty: 0.9,
            weight: 3,
            min_rise: 18.0,
            max_rise: 42.0,
            min_gap: 22.0,
            max_gap: 110.0,
        },
        ModuleKind::ShapeGauntlet => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            exit: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            min_difficulty: 0.24,
            max_difficulty: 1.0,
            weight: 4,
            min_rise: 20.0,
            max_rise: 58.0,
            min_gap: 26.0,
            max_gap: 150.0,
        },
        ModuleKind::MantleStack => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_MANTLE_ENTRY,
            exit: SOCKET_SAFE_REST | SOCKET_WALLRUN_READY,
            min_difficulty: 0.15,
            max_difficulty: 0.42,
            weight: 2,
            min_rise: 14.0,
            max_rise: 30.0,
            min_gap: 18.0,
            max_gap: 64.0,
        },
        ModuleKind::WallRunHall => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_WALLRUN_READY,
            exit: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            min_difficulty: 0.28,
            max_difficulty: 0.9,
            weight: 4,
            min_rise: 16.0,
            max_rise: 40.0,
            min_gap: 20.0,
            max_gap: 90.0,
        },
        ModuleKind::LiftChasm => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH | SOCKET_SHORTCUT_ANCHOR,
            exit: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            min_difficulty: 0.22,
            max_difficulty: 0.45,
            weight: 1,
            min_rise: 14.0,
            max_rise: 32.0,
            min_gap: 20.0,
            max_gap: 70.0,
        },
        ModuleKind::CrumbleBridge => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            exit: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH | SOCKET_SHORTCUT_ANCHOR,
            min_difficulty: 0.34,
            max_difficulty: 0.95,
            weight: 4,
            min_rise: 16.0,
            max_rise: 40.0,
            min_gap: 18.0,
            max_gap: 96.0,
        },
        ModuleKind::WindTunnel => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_WALLRUN_READY | SOCKET_HAZARD_BRANCH,
            exit: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH | SOCKET_SHORTCUT_ANCHOR,
            min_difficulty: 0.35,
            max_difficulty: 1.0,
            weight: 5,
            min_rise: 18.0,
            max_rise: 46.0,
            min_gap: 22.0,
            max_gap: 110.0,
        },
        ModuleKind::IceSpine => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            exit: SOCKET_SAFE_REST | SOCKET_HAZARD_BRANCH,
            min_difficulty: 0.28,
            max_difficulty: 1.0,
            weight: 5,
            min_rise: 16.0,
            max_rise: 42.0,
            min_gap: 20.0,
            max_gap: 120.0,
        },
        ModuleKind::WaterGarden => ModuleTemplate {
            kind,
            entry: SOCKET_SAFE_REST | SOCKET_MANTLE_ENTRY,
            exit: SOCKET_SAFE_REST,
            min_difficulty: 0.18,
            max_difficulty: 0.32,
            weight: 1,
            min_rise: 12.0,
            max_rise: 26.0,
            min_gap: 18.0,
            max_gap: 52.0,
        },
    }
}

fn safe_fallback_kind(difficulty: f32) -> ModuleKind {
    if difficulty > 0.62 {
        ModuleKind::ShapeGauntlet
    } else if difficulty > 0.4 {
        ModuleKind::StairRun
    } else {
        ModuleKind::SurfRamp
    }
}

fn spawn_run_world(
    blueprint: &RunBlueprint,
    checkpoint_index: usize,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_cache: &mut WorldAssetCache,
) -> RunSnapshot {
    spawn_world_light_rig(blueprint, commands);
    spawn_sky_backdrop(blueprint, commands, meshes, materials);
    spawn_floating_spheres(blueprint, commands, meshes, materials);
    spawn_macro_spectacle(blueprint, commands, meshes, materials);

    let chunk_order = desired_chunk_window(blueprint, checkpoint_index);
    for chunk in &chunk_order {
        spawn_chunk(*chunk, blueprint, commands, meshes, materials, asset_cache);
    }

    build_run_snapshot(chunk_order.into_iter().collect())
}

fn spawn_world_light_rig(blueprint: &RunBlueprint, commands: &mut Commands) {
    let (center, course_radius) = course_visual_center_and_radius(blueprint);
    let course_min_y = blueprint
        .rooms
        .iter()
        .map(|room| room.top.y)
        .fold(f32::INFINITY, f32::min);
    let course_max_y = blueprint
        .rooms
        .iter()
        .map(|room| room.top.y)
        .fold(f32::NEG_INFINITY, f32::max);
    let course_mid_y = lerp(course_min_y, course_max_y, 0.5);
    let entry_heading = if blueprint.rooms.len() > 1 {
        let mut heading = blueprint.rooms[1].top - blueprint.rooms[0].top;
        heading.y = 0.0;
        heading.normalize_or_zero()
    } else {
        Vec3::new(0.0, 0.0, -1.0)
    };
    let entry_right = Vec3::new(-entry_heading.z, 0.0, entry_heading.x).normalize_or_zero();
    let target = center + Vec3::Y * 26.0;

    commands.spawn((
        GeneratedWorld,
        Name::new("Moon Key"),
        Transform::from_translation(
            center
                + entry_heading * (course_radius * 0.58 + 220.0)
                + entry_right * (course_radius * 0.2 + 140.0)
                + Vec3::Y * (course_mid_y + 260.0),
        )
        .looking_at(target, Vec3::Y),
        DirectionalLight {
            shadows_enabled: true,
            illuminance: 24_000.0,
            color: Color::srgb(0.98, 0.94, 0.88),
            ..default()
        },
        CascadeShadowConfigBuilder {
            first_cascade_far_bound: 36.0,
            maximum_distance: 260.0,
            overlap_proportion: 0.24,
            ..default()
        }
        .build(),
    ));
    commands.spawn((
        GeneratedWorld,
        Name::new("Sky Fill"),
        Transform::from_translation(
            center
                - entry_heading * (course_radius * 0.46 + 180.0)
                - entry_right * (course_radius * 0.24 + 180.0)
                + Vec3::Y * (course_mid_y + 250.0),
        )
        .looking_at(target + entry_heading * 26.0, Vec3::Y),
        DirectionalLight {
            shadows_enabled: false,
            illuminance: 14_000.0,
            color: Color::srgb(0.46, 0.58, 0.88),
            ..default()
        },
    ));
    commands.spawn((
        GeneratedWorld,
        Name::new("Lift Bounce"),
        Transform::from_translation(
            center
                + entry_right * (course_radius * 0.26 + 120.0)
                - entry_heading * (course_radius * 0.18 + 80.0)
                + Vec3::Y * (course_min_y + 180.0),
        )
        .looking_at(target - entry_right * 20.0, Vec3::Y),
        DirectionalLight {
            shadows_enabled: false,
            illuminance: 9_500.0,
            color: Color::srgb(0.94, 0.58, 0.48),
            ..default()
        },
    ));
    commands.spawn((
        GeneratedWorld,
        Name::new("Ridge Rim"),
        Transform::from_translation(
            center
                - entry_right * (course_radius * 0.16 + 90.0)
                + entry_heading * (course_radius * 0.14 + 60.0)
                + Vec3::Y * (course_max_y + 140.0),
        )
        .looking_at(target + Vec3::Y * 22.0, Vec3::Y),
        DirectionalLight {
            shadows_enabled: false,
            illuminance: 7_500.0,
            color: Color::srgb(0.44, 0.84, 0.76),
            ..default()
        },
    ));
    commands.spawn((
        GeneratedWorld,
        Name::new("Prism Fill"),
        Transform::from_translation(
            center
                + entry_heading * (course_radius * 0.22 + 110.0)
                - entry_right * (course_radius * 0.34 + 180.0)
                + Vec3::Y * (course_mid_y + 110.0),
        )
        .looking_at(target - entry_heading * 34.0, Vec3::Y),
        DirectionalLight {
            shadows_enabled: false,
            illuminance: 5_400.0,
            color: Color::srgb(0.78, 0.58, 1.0),
            ..default()
        },
    ));
    commands.spawn((
        GeneratedWorld,
        Name::new("Course Lantern"),
        PointLight {
            intensity: 6_200_000.0,
            range: course_radius * 2.6 + 640.0,
            radius: 28.0,
            color: Color::srgb(0.74, 0.82, 1.0),
            shadows_enabled: false,
            ..default()
        },
        Transform::from_translation(center + Vec3::Y * (course_mid_y + 180.0)),
    ));
}

fn respawn_active_chunks(
    blueprint: &RunBlueprint,
    checkpoint_index: usize,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_cache: &mut WorldAssetCache,
) -> RunSnapshot {
    let chunk_order = desired_chunk_window(blueprint, checkpoint_index);
    for chunk in &chunk_order {
        spawn_chunk(*chunk, blueprint, commands, meshes, materials, asset_cache);
    }

    build_run_snapshot(chunk_order.into_iter().collect())
}

fn build_run_snapshot(active_chunks: HashSet<WorldChunkKey>) -> RunSnapshot {
    RunSnapshot { active_chunks }
}

fn closest_point_on_line_segment(point: Vec3, start: Vec3, end: Vec3) -> (Vec3, f32) {
    let delta = end - start;
    let length_squared = delta.length_squared();
    if length_squared <= f32::EPSILON {
        return (start, 0.0);
    }

    let t = ((point - start).dot(delta) / length_squared).clamp(0.0, 1.0);
    (start + delta * t, t)
}

#[derive(Clone, Copy)]
struct StreamWindow {
    start_room: usize,
    frontier_room: usize,
}

fn stream_focus_room(
    blueprint: &RunBlueprint,
    checkpoint_index: usize,
    previous_focus_room: usize,
    player_position: Vec3,
) -> usize {
    let last_room = blueprint.rooms.len().saturating_sub(1);
    let checkpoint_index = checkpoint_index.min(last_room);
    let previous_focus_room = previous_focus_room
        .max(checkpoint_index)
        .min(last_room);
    let search_start = checkpoint_index
        .min(previous_focus_room)
        .saturating_sub(STREAM_BEHIND_ROOMS + 2);
    let search_end = previous_focus_room
        .max(checkpoint_index)
        .saturating_add(STREAM_AHEAD_ROOMS + INFINITE_APPEND_TRIGGER_ROOMS + 10)
        .min(last_room);
    let mut best_room = previous_focus_room.max(checkpoint_index);
    let mut best_score = f32::INFINITY;

    for room in &blueprint.rooms[search_start..=search_end] {
        let offset = room.top - player_position;
        let horizontal = offset.xz().length();
        let vertical = offset.y.abs() * 0.24;
        let progression_bias = (room.index.saturating_sub(checkpoint_index)) as f32 * 1.25;
        let focus_bias = (previous_focus_room.abs_diff(room.index) as f32) * 0.85;
        let score = horizontal * 0.86 + vertical + progression_bias + focus_bias;
        if score < best_score {
            best_score = score;
            best_room = room.index;
        }
    }

    let segment_start = search_start.min(blueprint.segments.len());
    let segment_end = search_end.min(blueprint.segments.len().saturating_sub(1));
    if !blueprint.segments.is_empty() && segment_start <= segment_end {
        for segment in &blueprint.segments[segment_start..=segment_end] {
            let from = blueprint.rooms[segment.from].top;
            let to = blueprint.rooms[segment.to].top;
            let (closest, t) = closest_point_on_line_segment(player_position, from, to);
            let offset = player_position - closest;
            let horizontal = offset.xz().length();
            let vertical = offset.y.abs() * 0.2;
            let progression_bias = (segment.to.saturating_sub(checkpoint_index)) as f32 * 1.05;
            let focus_bias = (previous_focus_room.abs_diff(segment.to) as f32) * 0.75;
            let along_bonus = from.distance(to) * t * 0.12;
            let score = horizontal * 0.62 + vertical + progression_bias + focus_bias - along_bonus;
            let candidate_room = if t >= 0.38 { segment.to } else { segment.from };
            if score < best_score {
                best_score = score;
                best_room = candidate_room;
            }
        }
    }

    best_room.max(checkpoint_index).max(previous_focus_room.saturating_sub(1))
}

fn stream_window(blueprint: &RunBlueprint, focus_room: usize) -> StreamWindow {
    let last_room = blueprint.rooms.len().saturating_sub(1);
    let focus_room = focus_room.min(last_room);
    let start_room = focus_room.saturating_sub(STREAM_BEHIND_ROOMS);
    let end_room = (focus_room + STREAM_AHEAD_ROOMS).min(last_room);
    let frontier_room = (end_room + 2).min(last_room);

    StreamWindow {
        start_room,
        frontier_room,
    }
}

fn desired_chunk_window(blueprint: &RunBlueprint, focus_room: usize) -> Vec<WorldChunkKey> {
    let window = stream_window(blueprint, focus_room);
    let mut chunks = Vec::new();

    for room_index in window.start_room..=window.frontier_room {
        chunks.push(WorldChunkKey::Room(room_index));
    }
    for segment in &blueprint.segments {
        if segment.from >= window.start_room.saturating_sub(1)
            && segment.from < window.frontier_room
        {
            chunks.push(WorldChunkKey::Segment(segment.index));
        }
    }

    chunks
}

fn build_chunk_layout(chunk: WorldChunkKey, blueprint: &RunBlueprint) -> Option<ModuleLayout> {
    match chunk {
        WorldChunkKey::Room(index) => blueprint.rooms.get(index).map(build_room_layout),
        WorldChunkKey::Segment(index) => blueprint
            .segments
            .get(index)
            .map(|segment| build_segment_layout(segment, &blueprint.rooms)),
        WorldChunkKey::Summit => {
            blueprint
                .rooms
                .last()
                .map(|room| build_summit_layout(room, blueprint.summit))
        }
    }
}

fn spawn_chunk(
    chunk: WorldChunkKey,
    blueprint: &RunBlueprint,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_cache: &mut WorldAssetCache,
) {
    let Some(layout) = build_chunk_layout(chunk, blueprint) else {
        return;
    };
    spawn_layout(
        &layout,
        Some(chunk),
        commands,
        meshes,
        materials,
        asset_cache,
    );
}

fn spawn_sky_backdrop(
    blueprint: &RunBlueprint,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let (center, course_radius) = course_visual_center_and_radius(blueprint);
    let mut sky_rng = RunRng::new(blueprint.seed ^ 0xC1A0_DA7A_55AA_9911);
    let manual_moon_scale = MANUAL_MOON_SIZE_MULTIPLIER;
    let moon_heading = if blueprint.rooms.len() > 1 {
        let mut heading = blueprint.rooms[1].top - blueprint.rooms[0].top;
        heading.y = 0.0;
        heading.normalize_or_zero()
    } else {
        Vec3::new(0.0, 0.0, -1.0)
    };
    let moon_right = Vec3::new(-moon_heading.z, 0.0, moon_heading.x).normalize_or_zero();
    let sky_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.12, 0.14, 0.22),
        emissive: LinearRgba::rgb(0.1, 0.12, 0.2),
        unlit: true,
        cull_mode: None,
        ..default()
    });
    let cloud_deck_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.09, 0.1, 0.14),
        emissive: LinearRgba::rgb(0.03, 0.04, 0.06),
        unlit: true,
        cull_mode: None,
        ..default()
    });
    let star_mesh = meshes.add(Sphere::new(0.8).mesh().ico(3).unwrap());
    let dome_mesh = meshes.add(Sphere::new(1.0).mesh().ico(6).unwrap());
    let cloud_mesh = meshes.add(Sphere::new(1.0).mesh().ico(4).unwrap());
    let comet_mesh = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let star_material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        emissive: LinearRgba::rgb(2.8, 3.0, 3.8),
        unlit: true,
        ..default()
    });

    commands.spawn((
        GeneratedWorld,
        AtmosphereMaterialKind::SkyDome,
        Name::new("Sky Dome"),
        Mesh3d(dome_mesh.clone()),
        MeshMaterial3d(sky_material),
        Transform::from_translation(center + Vec3::Y * 120.0).with_scale(Vec3::splat(SKY_RADIUS)),
        NotShadowCaster,
        NotShadowReceiver,
    ));
    commands.spawn((
        GeneratedWorld,
        AtmosphereMaterialKind::CloudDeck,
        Name::new("Cloud Sea"),
        Mesh3d(cloud_mesh.clone()),
        MeshMaterial3d(cloud_deck_material.clone()),
        Transform::from_translation(Vec3::new(center.x, blueprint.death_plane - 30.0, center.z))
            .with_scale(Vec3::new(course_radius * 2.7, 24.0, course_radius * 2.7)),
        NotShadowCaster,
        NotShadowReceiver,
    ));

    let mut starfield = ColoredMeshBuilder::default();
    for star_index in 0..STAR_COUNT {
        let f = star_index as f32 / STAR_COUNT as f32;
        let theta = TAU * f * 21.0;
        let y = 1.0 - 2.0 * (star_index as f32 + 0.5) / STAR_COUNT as f32;
        let r = (1.0 - y * y).sqrt();
        let direction = Vec3::new(r * theta.cos(), y, r * theta.sin());
        let position = center + Vec3::Y * 120.0 + direction * (SKY_RADIUS * 0.9);
        let size = (0.12 + ((star_index * 37 % 17) as f32) * 0.022) * STAR_SIZE_MULTIPLIER;
        let tint = if star_index % 9 == 0 {
            Color::linear_rgb(4.2, 4.8, 6.9)
        } else if star_index % 7 == 0 {
            Color::linear_rgb(6.0, 4.8, 3.4)
        } else {
            Color::linear_rgb(4.6, 4.6, 5.2)
        };
        append_star_render_geometry(&mut starfield, position, direction, size, tint);
    }
    commands.spawn((
        GeneratedWorld,
        Name::new("Starfield"),
        Mesh3d(meshes.add(starfield.build())),
        MeshMaterial3d(star_material.clone()),
        Transform::default(),
        NotShadowCaster,
        NotShadowReceiver,
    ));

    for cluster_index in 0..STAR_CLUSTER_COUNT {
        let angle = TAU * (cluster_index as f32 / STAR_CLUSTER_COUNT as f32)
            + sky_rng.range_f32(-0.24, 0.24);
        let radial = Vec3::new(angle.cos(), 0.0, angle.sin()).normalize_or_zero();
        let tangent = Vec3::new(-radial.z, 0.0, radial.x);
        let anchor = center
            + radial * sky_rng.range_f32(course_radius + 540.0, course_radius + 960.0)
            + Vec3::Y * sky_rng.range_f32(220.0, 420.0);
        let cluster_tint = if cluster_index % 2 == 0 {
            Color::linear_rgb(4.2, 5.0, 6.6)
        } else {
            Color::linear_rgb(5.4, 4.6, 5.6)
        };
        let mut cluster_mesh = ColoredMeshBuilder::default();
        for _ in 0..sky_rng.range_usize(5, 9) {
            let offset = Vec3::new(
                sky_rng.range_f32(-18.0, 18.0),
                sky_rng.range_f32(-7.0, 7.0),
                sky_rng.range_f32(-18.0, 18.0),
            );
            let direction = (offset + Vec3::Y * 2.0).normalize_or_zero();
            append_star_render_geometry(
                &mut cluster_mesh,
                offset,
                direction,
                sky_rng.range_f32(0.28, 0.9) * STAR_CLUSTER_SIZE_MULTIPLIER,
                cluster_tint,
            );
        }
        commands.spawn((
            GeneratedWorld,
            Name::new("Star Cluster"),
            Mesh3d(meshes.add(cluster_mesh.build())),
            MeshMaterial3d(star_material.clone()),
            Transform::from_translation(anchor),
            NotShadowCaster,
            NotShadowReceiver,
            SkyDrift {
                anchor,
                primary_axis: tangent,
                secondary_axis: radial,
                primary_amplitude: sky_rng.range_f32(6.0, 20.0),
                secondary_amplitude: sky_rng.range_f32(3.0, 8.0),
                vertical_amplitude: sky_rng.range_f32(1.2, 4.0),
                speed: sky_rng.range_f32(0.03, 0.08),
                rotation_speed: sky_rng.range_f32(-0.012, 0.012),
                phase: sky_rng.range_f32(0.0, TAU),
                base_rotation: Quat::from_rotation_y(sky_rng.range_f32(0.0, TAU)),
            },
        ));
    }

    for comet_index in 0..COMET_COUNT {
        let angle =
            TAU * (comet_index as f32 / COMET_COUNT as f32) + sky_rng.range_f32(-0.18, 0.18);
        let radial = Vec3::new(angle.cos(), 0.0, angle.sin()).normalize_or_zero();
        let tangent = Vec3::new(-radial.z, 0.0, radial.x);
        let anchor = center
            + radial * sky_rng.range_f32(course_radius + 760.0, course_radius + 1_280.0)
            + Vec3::Y * sky_rng.range_f32(260.0, 520.0);
        let comet_tint = if comet_index % 2 == 0 {
            Color::linear_rgb(3.8, 4.8, 6.4)
        } else {
            Color::linear_rgb(5.0, 4.0, 5.4)
        };
        let tail_material = materials.add(StandardMaterial {
            base_color: comet_tint.with_alpha(0.08),
            emissive: LinearRgba::from(comet_tint) * 0.5,
            unlit: true,
            alpha_mode: AlphaMode::AlphaToCoverage,
            cull_mode: None,
            ..default()
        });
        let core_material = materials.add(StandardMaterial {
            base_color: comet_tint,
            emissive: LinearRgba::from(comet_tint) * 1.0,
            unlit: true,
            ..default()
        });
        let tail_length = sky_rng.range_f32(24.0, 44.0) * COMET_SIZE_MULTIPLIER;
        let tail_width = sky_rng.range_f32(1.0, 2.0) * COMET_SIZE_MULTIPLIER;
        commands
            .spawn((
                GeneratedWorld,
                Name::new("Comet"),
                Transform::from_translation(anchor).with_rotation(
                    Quat::from_rotation_y(angle + PI * 0.5)
                        * Quat::from_rotation_z(sky_rng.range_f32(-0.2, 0.2)),
                ),
                Visibility::default(),
                NotShadowCaster,
                NotShadowReceiver,
                SkyDrift {
                    anchor,
                    primary_axis: tangent,
                    secondary_axis: radial,
                    primary_amplitude: sky_rng.range_f32(18.0, 36.0),
                    secondary_amplitude: sky_rng.range_f32(6.0, 14.0),
                    vertical_amplitude: sky_rng.range_f32(3.0, 7.0),
                    speed: sky_rng.range_f32(0.02, 0.05),
                    rotation_speed: sky_rng.range_f32(-0.01, 0.01),
                    phase: sky_rng.range_f32(0.0, TAU),
                    base_rotation: Quat::from_rotation_y(angle + PI * 0.5),
                },
            ))
            .with_children(|parent| {
                parent.spawn((
                    Name::new("Comet Tail"),
                    Mesh3d(comet_mesh.clone()),
                    MeshMaterial3d(tail_material.clone()),
                    Transform::from_xyz(-tail_length * 0.2, 0.0, 0.0).with_scale(Vec3::new(
                        tail_length,
                        tail_width,
                        tail_width * 2.2,
                    )),
                ));
                parent.spawn((
                    Name::new("Comet Glow"),
                    Mesh3d(comet_mesh.clone()),
                    MeshMaterial3d(tail_material.clone()),
                    Transform::from_xyz(-tail_length * 0.06, 0.0, 0.0).with_scale(Vec3::new(
                        tail_length * 0.46,
                        tail_width * 1.8,
                        tail_width * 3.6,
                    )),
                ));
                parent.spawn((
                    Name::new("Comet Core"),
                    Mesh3d(star_mesh.clone()),
                    MeshMaterial3d(core_material),
                    Transform::from_xyz(tail_length * 0.34, 0.0, 0.0)
                        .with_scale(Vec3::splat(tail_width * 1.45)),
                ));
            });
    }

    commands.spawn((
        GeneratedWorld,
        AtmosphereMaterialKind::Celestial,
        Name::new("Hero Moon"),
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().ico(6).unwrap())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.92, 0.95, 1.0),
            emissive: LinearRgba::rgb(8.8, 10.2, 13.4),
            unlit: true,
            ..default()
        })),
        Transform::from_translation(
            center
                + moon_heading * (course_radius + 1_160.0)
                + moon_right * 360.0
                + Vec3::Y * 440.0,
        )
        .with_scale(Vec3::splat(64.0 * manual_moon_scale)),
        NotShadowCaster,
        NotShadowReceiver,
    ));
    commands.spawn((
        GeneratedWorld,
        AtmosphereMaterialKind::Celestial,
        Name::new("Great Moon"),
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().ico(5).unwrap())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.82, 0.9, 1.0),
            emissive: LinearRgba::rgb(5.4, 6.0, 8.2),
            unlit: true,
            ..default()
        })),
        Transform::from_xyz(
            center.x - course_radius - 980.0,
            center.y + 420.0,
            center.z + course_radius + 860.0,
        )
        .with_scale(Vec3::splat(34.0 * manual_moon_scale)),
        NotShadowCaster,
        NotShadowReceiver,
    ));
    commands.spawn((
        GeneratedWorld,
        AtmosphereMaterialKind::Celestial,
        Name::new("Companion Moon"),
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().ico(5).unwrap())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.62, 0.74, 0.98),
            emissive: LinearRgba::rgb(2.2, 2.8, 4.0),
            unlit: true,
            ..default()
        })),
        Transform::from_xyz(
            center.x + course_radius + 1_120.0,
            center.y + 380.0,
            center.z - course_radius - 940.0,
        )
        .with_scale(Vec3::splat(18.0 * manual_moon_scale)),
        NotShadowCaster,
        NotShadowReceiver,
    ));
    commands.spawn((
        GeneratedWorld,
        AtmosphereMaterialKind::Celestial,
        Name::new("Azure Moon"),
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().ico(5).unwrap())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.72, 0.82, 1.0),
            emissive: LinearRgba::rgb(3.0, 3.8, 5.6),
            unlit: true,
            ..default()
        })),
        Transform::from_translation(
            center - moon_right * (course_radius + 1_060.0)
                + moon_heading * 420.0
                + Vec3::Y * 520.0,
        )
        .with_scale(Vec3::splat(22.0 * manual_moon_scale)),
        NotShadowCaster,
        NotShadowReceiver,
    ));
    commands.spawn((
        GeneratedWorld,
        AtmosphereMaterialKind::Celestial,
        Name::new("Rose Moon"),
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().ico(5).unwrap())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.96, 0.76, 0.9),
            emissive: LinearRgba::rgb(3.8, 2.8, 4.2),
            unlit: true,
            ..default()
        })),
        Transform::from_translation(
            center + moon_right * (course_radius + 1_280.0) - moon_heading * 520.0
                + Vec3::Y * 400.0,
        )
        .with_scale(Vec3::splat(14.0 * manual_moon_scale)),
        NotShadowCaster,
        NotShadowReceiver,
    ));
}

fn spawn_floating_spheres(
    blueprint: &RunBlueprint,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let mesh_library = CelestialMeshLibrary {
        sphere: meshes.add(Sphere::new(1.0).mesh().ico(5).unwrap()),
        torus: meshes.add(Torus::default()),
        capsule: meshes.add(Capsule3d::default()),
        cylinder: meshes.add(Cylinder::default()),
        cone: meshes.add(Cone::default()),
        frustum: meshes.add(ConicalFrustum::default()),
        tetrahedron: meshes.add(Tetrahedron::default()),
        cuboid: meshes.add(Cuboid::new(2.0, 2.0, 2.0)),
    };
    for body in build_celestial_body_plans(blueprint) {
        spawn_floating_celestial_entity(
            commands,
            &mesh_library,
            materials,
            body.shape,
            body.radius,
            body.anchor,
            body.drift_primary,
            body.drift_secondary,
            body.primary_amplitude,
            body.secondary_amplitude,
            body.vertical_amplitude,
            body.speed,
            body.rotation_speed,
            body.phase,
            body.base_rotation,
            body.tint,
            body.glows,
            body.ringed,
        );
    }
}

#[derive(Clone)]
struct CelestialMeshLibrary {
    sphere: Handle<Mesh>,
    torus: Handle<Mesh>,
    capsule: Handle<Mesh>,
    cylinder: Handle<Mesh>,
    cone: Handle<Mesh>,
    frustum: Handle<Mesh>,
    tetrahedron: Handle<Mesh>,
    cuboid: Handle<Mesh>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CelestialShapeKind {
    Sphere,
    Torus,
    Capsule,
    Cylinder,
    Cone,
    Frustum,
    Tetrahedron,
    Cuboid,
}

impl CelestialShapeKind {
    fn mesh(self, library: &CelestialMeshLibrary) -> Handle<Mesh> {
        match self {
            Self::Sphere => library.sphere.clone(),
            Self::Torus => library.torus.clone(),
            Self::Capsule => library.capsule.clone(),
            Self::Cylinder => library.cylinder.clone(),
            Self::Cone => library.cone.clone(),
            Self::Frustum => library.frustum.clone(),
            Self::Tetrahedron => library.tetrahedron.clone(),
            Self::Cuboid => library.cuboid.clone(),
        }
    }

    fn scale(self, radius: f32) -> Vec3 {
        match self {
            Self::Sphere => Vec3::splat(radius),
            Self::Torus => Vec3::splat(radius * 0.92),
            Self::Capsule => Vec3::new(radius * 0.82, radius * 1.26, radius * 0.82),
            Self::Cylinder => Vec3::new(radius * 0.86, radius * 1.18, radius * 0.86),
            Self::Cone => Vec3::new(radius * 0.96, radius * 1.34, radius * 0.96),
            Self::Frustum => Vec3::new(radius * 1.04, radius * 1.28, radius * 1.04),
            Self::Tetrahedron => Vec3::splat(radius * 1.08),
            Self::Cuboid => Vec3::new(radius * 1.18, radius * 0.86, radius),
        }
    }

    fn halo_scale(self) -> Vec3 {
        match self {
            Self::Sphere => Vec3::splat(1.12),
            Self::Torus => Vec3::splat(1.08),
            Self::Capsule => Vec3::new(1.08, 1.14, 1.08),
            Self::Cylinder => Vec3::new(1.08, 1.12, 1.08),
            Self::Cone => Vec3::new(1.08, 1.14, 1.08),
            Self::Frustum => Vec3::new(1.08, 1.12, 1.08),
            Self::Tetrahedron => Vec3::splat(1.1),
            Self::Cuboid => Vec3::new(1.12, 1.08, 1.1),
        }
    }

    fn clearance_multiplier(self) -> f32 {
        match self {
            Self::Sphere => 1.0,
            Self::Torus => 1.18,
            Self::Capsule => 1.22,
            Self::Cylinder => 1.16,
            Self::Cone => 1.2,
            Self::Frustum => 1.18,
            Self::Tetrahedron => 1.24,
            Self::Cuboid => 1.26,
        }
    }
}

fn spawn_floating_celestial_entity(
    commands: &mut Commands,
    mesh_library: &CelestialMeshLibrary,
    materials: &mut Assets<StandardMaterial>,
    shape: CelestialShapeKind,
    radius: f32,
    anchor: Vec3,
    drift_primary: Vec3,
    drift_secondary: Vec3,
    primary_amplitude: f32,
    secondary_amplitude: f32,
    vertical_amplitude: f32,
    speed: f32,
    rotation_speed: f32,
    phase: f32,
    base_rotation: Quat,
    tint: Color,
    glows: bool,
    ringed: bool,
) {
    let mesh = shape.mesh(mesh_library);
    let ring_mesh = mesh_library.torus.clone();
    let shape_scale = shape.scale(radius);
    let shell_material = materials.add(StandardMaterial {
        base_color: tint,
        emissive: LinearRgba::from(tint) * 0.14,
        reflectance: 0.68,
        specular_tint: tint,
        clearcoat: 0.56,
        clearcoat_perceptual_roughness: 0.24,
        metallic: 0.02,
        perceptual_roughness: 0.34,
        ..default()
    });
    let core_material = materials.add(StandardMaterial {
        base_color: tint,
        emissive: LinearRgba::from(tint) * 0.24,
        reflectance: 0.82,
        metallic: 0.04,
        perceptual_roughness: 0.22,
        ..default()
    });
    let ring_material = materials.add(StandardMaterial {
        base_color: brighten(tint, 0.08),
        emissive: LinearRgba::from(tint) * 0.16,
        reflectance: 0.62,
        clearcoat: 0.44,
        clearcoat_perceptual_roughness: 0.22,
        perceptual_roughness: 0.3,
        ..default()
    });
    let mut entity = commands.spawn((
        GeneratedWorld,
        AtmosphereMaterialKind::Celestial,
        Name::new("Floating Celestial"),
        Mesh3d(mesh.clone()),
        MeshMaterial3d(shell_material),
        Transform::from_translation(anchor).with_scale(shape_scale),
        NotShadowCaster,
        NotShadowReceiver,
        SkyDrift {
            anchor,
            primary_axis: drift_primary,
            secondary_axis: drift_secondary,
            primary_amplitude,
            secondary_amplitude,
            vertical_amplitude,
            speed,
            rotation_speed,
            phase,
            base_rotation,
        },
    ));

    entity.with_children(|parent| {
        if glows {
            parent.spawn((
                Name::new("Floating Celestial Core"),
                AtmosphereMaterialKind::Celestial,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(core_material),
                Transform::from_scale(shape.halo_scale() * 0.9),
                NotShadowCaster,
                NotShadowReceiver,
            ));
        }

        if ringed {
            parent.spawn((
                Name::new("Floating Celestial Ring"),
                AtmosphereMaterialKind::Celestial,
                Mesh3d(ring_mesh),
                MeshMaterial3d(ring_material),
                Transform::from_rotation(Quat::from_rotation_x(1.2) * Quat::from_rotation_z(0.35))
                    .with_scale(Vec3::new(radius * 1.75, radius * 0.3, radius * 1.75)),
                NotShadowCaster,
                NotShadowReceiver,
            ));
        }
    });
}

#[derive(Clone)]
struct CelestialBodyPlan {
    shape: CelestialShapeKind,
    anchor: Vec3,
    radius: f32,
    drift_primary: Vec3,
    drift_secondary: Vec3,
    primary_amplitude: f32,
    secondary_amplitude: f32,
    vertical_amplitude: f32,
    speed: f32,
    rotation_speed: f32,
    phase: f32,
    base_rotation: Quat,
    tint: Color,
    glows: bool,
    ringed: bool,
}

fn celestial_vertical_bias(rng: &mut RunRng, index: usize, count: usize) -> f32 {
    let phase = TAU * (index as f32 / count.max(1) as f32);
    let band = match index % 4 {
        0 => -1.9,
        1 => -0.55,
        2 => 0.62,
        _ => 1.9,
    };
    let wave = (phase * 1.9).sin() * 0.46 + (phase * 0.7 + 1.1).cos() * 0.22;
    (band + wave + rng.range_f32(-0.28, 0.28)).clamp(-2.35, 2.35)
}

fn celestial_depth_bias(rng: &mut RunRng, index: usize, count: usize, major: bool) -> f32 {
    let phase = TAU * (index as f32 / count.max(1) as f32);
    let band = if major {
        match index % 4 {
            0 => -1.15,
            1 => -0.3,
            2 => 0.42,
            _ => 1.12,
        }
    } else {
        match index % 4 {
            0 => -0.9,
            1 => -0.18,
            2 => 0.28,
            _ => 0.88,
        }
    };
    let wave = (phase * 1.35 + 0.45).sin() * 0.2 + (phase * 2.1).cos() * 0.12;
    let limit = if major { 1.35 } else { 1.1 };
    (band + wave + rng.range_f32(-0.16, 0.16)).clamp(-limit, limit)
}

fn disperse_celestial_anchor(
    blueprint: &RunBlueprint,
    anchor: Vec3,
    radial: Vec3,
    tangent: Vec3,
    radius: f32,
    major: bool,
    vertical_bias: f32,
    depth_bias: f32,
) -> Vec3 {
    let vertical_span = if major {
        460.0 + radius * 0.28
    } else {
        320.0 + radius * 0.2
    };
    let tangential_shift = vertical_bias
        * if major {
            180.0 + radius * 0.14
        } else {
            120.0 + radius * 0.1
        };
    let radial_shift = depth_bias
        * if major {
            120.0 + radius * 0.09
        } else {
            86.0 + radius * 0.06
        };
    let minimum_clearance = if major {
        154.0 + radius * 0.06
    } else {
        126.0 + radius * 0.04
    };
    let mut displaced = anchor
        + Vec3::Y * (vertical_bias * vertical_span)
        + tangent * tangential_shift
        + radial * radial_shift;
    let clearance = celestial_course_clearance(blueprint, displaced, radius);
    if clearance < minimum_clearance {
        displaced += celestial_course_escape_direction(blueprint, displaced)
            * (minimum_clearance - clearance + 24.0);
    } else if depth_bias < 0.0 {
        let inward_budget = (clearance - minimum_clearance)
            .min(if major {
                190.0 + radius * 0.09
            } else {
                110.0 + radius * 0.05
            })
            .max(0.0);
        displaced -= radial * (inward_budget * depth_bias.abs() * 0.48);
    }
    let final_minimum_clearance = if major { 132.0 } else { 120.0 };
    let final_clearance = celestial_course_clearance(blueprint, displaced, radius);
    if final_clearance < final_minimum_clearance {
        displaced += celestial_course_escape_direction(blueprint, displaced)
            * (final_minimum_clearance - final_clearance + 16.0);
    }
    displaced
}

fn build_celestial_body_plans(blueprint: &RunBlueprint) -> Vec<CelestialBodyPlan> {
    let (center, course_radius) = course_visual_center_and_radius(blueprint);
    let mut rng = RunRng::new(blueprint.seed ^ 0x5151_AAAA_9999_7777);
    let major_shapes = [
        CelestialShapeKind::Torus,
        CelestialShapeKind::Capsule,
        CelestialShapeKind::Frustum,
        CelestialShapeKind::Cuboid,
        CelestialShapeKind::Cone,
        CelestialShapeKind::Cylinder,
        CelestialShapeKind::Tetrahedron,
        CelestialShapeKind::Cuboid,
        CelestialShapeKind::Cuboid,
        CelestialShapeKind::Torus,
        CelestialShapeKind::Capsule,
        CelestialShapeKind::Frustum,
        CelestialShapeKind::Cone,
        CelestialShapeKind::Cylinder,
        CelestialShapeKind::Tetrahedron,
        CelestialShapeKind::Capsule,
    ];
    let moon_shapes = [
        CelestialShapeKind::Cuboid,
        CelestialShapeKind::Tetrahedron,
        CelestialShapeKind::Capsule,
        CelestialShapeKind::Torus,
        CelestialShapeKind::Frustum,
        CelestialShapeKind::Cylinder,
        CelestialShapeKind::Cone,
        CelestialShapeKind::Sphere,
        CelestialShapeKind::Cuboid,
        CelestialShapeKind::Capsule,
        CelestialShapeKind::Torus,
        CelestialShapeKind::Frustum,
        CelestialShapeKind::Tetrahedron,
        CelestialShapeKind::Cylinder,
        CelestialShapeKind::Cone,
        CelestialShapeKind::Sphere,
        CelestialShapeKind::Cuboid,
        CelestialShapeKind::Capsule,
        CelestialShapeKind::Torus,
        CelestialShapeKind::Frustum,
        CelestialShapeKind::Tetrahedron,
        CelestialShapeKind::Sphere,
    ];
    let major_count = major_shapes.len();
    let moon_count = moon_shapes.len();
    let decor_count = 24;
    let mut bodies = Vec::with_capacity(major_count + moon_count + decor_count);

    for body_index in 0..major_count {
        let angle = TAU * (body_index as f32 / major_count as f32) + rng.range_f32(-0.28, 0.28);
        let radial = Vec3::new(angle.cos(), 0.0, angle.sin()).normalize_or_zero();
        let tangent = Vec3::new(-radial.z, 0.0, radial.x);
        let radius = rng.range_f32(18.0, 38.0) * MAJOR_CELESTIAL_RADIUS_MULTIPLIER;
        let shape = major_shapes[body_index];
        let clearance_radius = radius * shape.clearance_multiplier();
        let vertical_bias = celestial_vertical_bias(&mut rng, body_index, major_count);
        let depth_bias = celestial_depth_bias(&mut rng, body_index, major_count, true);
        let anchor = find_safe_celestial_anchor(
            blueprint,
            &mut rng,
            center,
            course_radius,
            radial,
            tangent,
            clearance_radius,
            true,
            vertical_bias,
            depth_bias,
        );
        let anchor = disperse_celestial_anchor(
            blueprint,
            anchor,
            radial,
            tangent,
            clearance_radius,
            true,
            vertical_bias,
            depth_bias,
        );
        let drift_secondary = (Vec3::Y * rng.range_f32(0.72, 1.0)
            + radial * rng.range_f32(-0.18, 0.18))
        .normalize_or_zero();
        bodies.push(CelestialBodyPlan {
            shape,
            anchor,
            radius,
            drift_primary: tangent,
            drift_secondary,
            primary_amplitude: rng.range_f32(3.5, 8.5),
            secondary_amplitude: rng.range_f32(1.2, 3.6),
            vertical_amplitude: 1.2 + radius * 0.04,
            speed: rng.range_f32(0.012, 0.026),
            rotation_speed: rng.range_f32(-0.0025, 0.0025),
            phase: rng.range_f32(0.0, TAU),
            base_rotation: Quat::from_rotation_y(rng.range_f32(0.0, TAU)),
            tint: floating_sphere_color(body_index),
            glows: body_index % 2 == 0 || rng.range_f32(0.0, 1.0) > 0.64,
            ringed: shape != CelestialShapeKind::Torus
                && (body_index % 3 == 0 || rng.range_f32(0.0, 1.0) > 0.7),
        });
    }

    for moon_index in 0..moon_count {
        let angle = TAU * (moon_index as f32 / moon_count as f32) + rng.range_f32(-0.36, 0.36);
        let radial = Vec3::new(angle.cos(), 0.0, angle.sin()).normalize_or_zero();
        let tangent = Vec3::new(-radial.z, 0.0, radial.x);
        let radius = rng.range_f32(8.5, 16.5) * MOON_CELESTIAL_RADIUS_MULTIPLIER;
        let mut shape = moon_shapes[moon_index];
        if shape == CelestialShapeKind::Sphere && radius >= 800.0 {
            shape = CelestialShapeKind::Capsule;
        }
        let clearance_radius = radius * shape.clearance_multiplier();
        let vertical_bias = celestial_vertical_bias(&mut rng, moon_index, moon_count);
        let depth_bias = celestial_depth_bias(&mut rng, moon_index, moon_count, false);
        let anchor = find_safe_celestial_anchor(
            blueprint,
            &mut rng,
            center,
            course_radius,
            radial,
            tangent,
            clearance_radius,
            false,
            vertical_bias,
            depth_bias,
        );
        let anchor = disperse_celestial_anchor(
            blueprint,
            anchor,
            radial,
            tangent,
            clearance_radius,
            false,
            vertical_bias,
            depth_bias,
        );
        let drift_secondary = (Vec3::Y * rng.range_f32(0.55, 0.95)
            + tangent * rng.range_f32(-0.25, 0.25))
        .normalize_or_zero();
        bodies.push(CelestialBodyPlan {
            shape,
            anchor,
            radius,
            drift_primary: tangent,
            drift_secondary,
            primary_amplitude: rng.range_f32(2.6, 6.4),
            secondary_amplitude: rng.range_f32(1.0, 2.8),
            vertical_amplitude: 0.9 + radius * 0.045,
            speed: rng.range_f32(0.014, 0.03),
            rotation_speed: rng.range_f32(-0.003, 0.003),
            phase: rng.range_f32(0.0, TAU),
            base_rotation: Quat::from_rotation_y(rng.range_f32(0.0, TAU)),
            tint: mix_color(
                floating_sphere_color(moon_index + 37),
                Color::WHITE,
                rng.range_f32(0.08, 0.24),
            ),
            glows: moon_index % 2 == 0 || rng.range_f32(0.0, 1.0) > 0.72,
            ringed: shape != CelestialShapeKind::Torus && rng.range_f32(0.0, 1.0) > 0.8,
        });
    }

    let decor_shapes = [
        CelestialShapeKind::Torus,
        CelestialShapeKind::Capsule,
        CelestialShapeKind::Cylinder,
        CelestialShapeKind::Cone,
        CelestialShapeKind::Frustum,
        CelestialShapeKind::Tetrahedron,
        CelestialShapeKind::Cuboid,
    ];
    for decor_index in 0..decor_count {
        let angle = TAU * (decor_index as f32 / decor_count as f32) + rng.range_f32(-0.24, 0.24);
        let radial = Vec3::new(angle.cos(), 0.0, angle.sin()).normalize_or_zero();
        let tangent = Vec3::new(-radial.z, 0.0, radial.x);
        let shape = decor_shapes[decor_index % decor_shapes.len()];
        let radius = rng.range_f32(5.5, 14.5) * DECOR_CELESTIAL_RADIUS_MULTIPLIER;
        let clearance_radius = radius * shape.clearance_multiplier();
        let vertical_bias = celestial_vertical_bias(&mut rng, decor_index, decor_count);
        let depth_bias = celestial_depth_bias(&mut rng, decor_index, decor_count, false);
        let mut anchor = find_safe_celestial_anchor(
            blueprint,
            &mut rng,
            center,
            course_radius,
            radial,
            tangent,
            clearance_radius,
            false,
            vertical_bias,
            depth_bias,
        );
        anchor = disperse_celestial_anchor(
            blueprint,
            anchor,
            radial,
            tangent,
            clearance_radius,
            false,
            vertical_bias,
            depth_bias,
        ) + tangent * rng.range_f32(-72.0, 72.0);
        let decor_clearance = celestial_course_clearance(blueprint, anchor, clearance_radius);
        if decor_clearance < 120.0 {
            anchor += radial * (120.0 - decor_clearance + 24.0);
        }
        let drift_secondary = (Vec3::Y * rng.range_f32(0.42, 0.88)
            + tangent * rng.range_f32(-0.48, 0.48)
            + radial * rng.range_f32(-0.18, 0.18))
        .normalize_or_zero();
        bodies.push(CelestialBodyPlan {
            shape,
            anchor,
            radius,
            drift_primary: tangent,
            drift_secondary,
            primary_amplitude: rng.range_f32(3.8, 9.0),
            secondary_amplitude: rng.range_f32(1.6, 4.0),
            vertical_amplitude: rng.range_f32(1.2, 3.2),
            speed: rng.range_f32(0.016, 0.034),
            rotation_speed: rng.range_f32(0.04, 0.12)
                * if decor_index % 2 == 0 { 1.0 } else { -1.0 },
            phase: rng.range_f32(0.0, TAU),
            base_rotation: Quat::from_euler(
                EulerRot::YXZ,
                rng.range_f32(0.0, TAU),
                rng.range_f32(0.0, TAU),
                rng.range_f32(0.0, TAU),
            ),
            tint: floating_sphere_color(decor_index + major_count + moon_count),
            glows: decor_index % 2 == 0 || rng.range_f32(0.0, 1.0) > 0.76,
            ringed: false,
        });
    }

    bodies
}

fn spawn_macro_spectacle(
    blueprint: &RunBlueprint,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    if !ENABLE_MACRO_SPECTACLE {
        return;
    }
    if blueprint.rooms.len() < 2 {
        return;
    }

    let (center, course_radius) = course_visual_center_and_radius(blueprint);
    let center_xz = Vec3::new(center.x, 0.0, center.z);
    let mut rng = RunRng::new(blueprint.seed ^ 0xA11C_E5C4_7A51_D00D);
    let entry_heading = direction_from_delta(blueprint.rooms[1].top - blueprint.rooms[0].top);
    let entry_right = Vec3::new(-entry_heading.z, 0.0, entry_heading.x).normalize_or_zero();
    let course_min_y = blueprint
        .rooms
        .iter()
        .map(|room| room.top.y)
        .fold(f32::INFINITY, f32::min);
    let course_max_y = blueprint
        .rooms
        .iter()
        .map(|room| room.top.y)
        .fold(f32::NEG_INFINITY, f32::max);
    let course_height_span = (course_max_y - course_min_y).max(220.0);
    let helix_start_height = course_min_y - rng.range_f32(180.0, 360.0);
    let helix_end_height = course_max_y + rng.range_f32(140.0, 320.0);
    let helix_vertical_wave = rng.range_f32(48.0, 160.0);
    let helix_lateral_wave = rng.range_f32(24.0, 88.0);

    let mut helix_builder = ColoredMeshBuilder::default();
    let helix_turns = rng.range_f32(2.2, 3.4) * if rng.chance(0.5) { 1.0 } else { -1.0 };
    let helix_phase = rng.range_f32(0.0, TAU);
    let helix_samples = 96;
    let helix_radius = course_radius + rng.range_f32(82.0, 176.0);
    for lane in 0..2 {
        let radius = helix_radius + lane as f32 * 42.0;
        let width = 11.0 - lane as f32 * 2.4;
        let height = 1.2 - lane as f32 * 0.2;
        let color = if lane == 0 {
            Color::srgba(0.28, 0.88, 1.0, 0.18)
        } else {
            Color::srgba(1.0, 0.4, 0.86, 0.16)
        };
        let mut previous = None;
        for sample in 0..=helix_samples {
            let t = sample as f32 / helix_samples as f32;
            let angle = helix_phase + helix_turns * TAU * t + lane as f32 * PI;
            let radial = Vec3::new(angle.cos(), 0.0, angle.sin()).normalize_or_zero();
            let point = center_xz
                + radial * radius
                + entry_right * ((t * TAU * 1.6 + helix_phase).sin() * helix_lateral_wave)
                + Vec3::Y
                    * (lerp(helix_start_height, helix_end_height, t)
                        + (t * TAU * 2.0 + helix_phase).sin() * helix_vertical_wave);
            if let Some(last) = previous {
                append_beam_segment(
                    &mut helix_builder,
                    last,
                    point,
                    width,
                    height,
                    radial,
                    t * PI * (0.45 + lane as f32 * 0.12),
                    color,
                );
            }
            previous = Some(point);
        }
    }
    if !helix_builder.is_empty() {
        commands.spawn((
            GeneratedWorld,
            AtmosphereMaterialKind::Megastructure,
            Name::new("Grand Helix"),
            Mesh3d(meshes.add(helix_builder.build())),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::WHITE,
                emissive: LinearRgba::rgb(0.26, 0.24, 0.42),
                cull_mode: None,
                alpha_mode: AlphaMode::AlphaToCoverage,
                perceptual_roughness: 0.28,
                reflectance: 0.56,
                ..default()
            })),
            Transform::default(),
            NotShadowCaster,
            NotShadowReceiver,
        ));
    }

    let mut mobius_builder = ColoredMeshBuilder::default();
    let mobius_center = center_xz
        + entry_right * rng.range_f32(-120.0, 120.0)
        + entry_heading * rng.range_f32(-72.0, 72.0)
        + Vec3::Y
            * (lerp(course_min_y, course_max_y, rng.range_f32(0.18, 0.82))
                + rng.range_f32(-course_height_span * 0.58, course_height_span * 0.78));
    let major_radius = course_radius * rng.range_f32(0.56, 0.82) + rng.range_f32(120.0, 210.0);
    let mobius_lateral_wave = major_radius * rng.range_f32(0.1, 0.22);
    let mobius_vertical_wave = major_radius * rng.range_f32(0.12, 0.24);
    let mobius_samples = 84;
    let mut previous = None;
    for sample in 0..=mobius_samples {
        let t = sample as f32 / mobius_samples as f32;
        let angle = TAU * t;
        let radial = Vec3::new(angle.cos(), 0.0, angle.sin()).normalize_or_zero();
        let point = mobius_center
            + radial * major_radius
            + entry_right * ((angle * 2.0).cos() * mobius_lateral_wave)
            + entry_heading * ((angle + 0.7).sin() * major_radius * 0.12)
            + Vec3::Y * ((angle * 2.0).sin() * mobius_vertical_wave);
        let color = mix_color(
            Color::srgba(0.34, 0.92, 1.0, 0.18),
            Color::srgba(1.0, 0.42, 0.84, 0.18),
            t,
        );
        let up_hint =
            (radial + Vec3::Y * (angle * 2.0).sin() * 0.4 + entry_right * 0.1).normalize_or_zero();
        if let Some(last) = previous {
            append_beam_segment(
                &mut mobius_builder,
                last,
                point,
                12.5,
                1.3,
                up_hint,
                angle * 0.5,
                color,
            );
        }
        previous = Some(point);
    }
    if !mobius_builder.is_empty() {
        commands.spawn((
            GeneratedWorld,
            AtmosphereMaterialKind::Megastructure,
            Name::new("Mobius Halo"),
            Mesh3d(meshes.add(mobius_builder.build())),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::WHITE,
                emissive: LinearRgba::rgb(0.34, 0.2, 0.44),
                cull_mode: None,
                alpha_mode: AlphaMode::AlphaToCoverage,
                perceptual_roughness: 0.26,
                reflectance: 0.62,
                ..default()
            })),
            Transform::default(),
            NotShadowCaster,
            NotShadowReceiver,
        ));
    }

    let mut frame_builder = ColoredMeshBuilder::default();
    let frame_height = lerp(course_min_y, course_max_y, rng.range_f32(0.08, 0.52))
        + rng.range_f32(-course_height_span * 0.62, course_height_span * 0.42);
    let frame_center = center_xz
        + entry_right * (course_radius + rng.range_f32(90.0, 196.0))
        + entry_heading * rng.range_f32(-78.0, 48.0)
        + Vec3::Y * frame_height;
    let frame_normal =
        (Vec3::new(center.x, frame_height + rng.range_f32(-40.0, 72.0), center.z) - frame_center)
            .normalize_or_zero();
    append_rect_frame(
        &mut frame_builder,
        frame_center,
        entry_heading,
        Vec3::Y,
        frame_normal,
        Vec2::new(360.0, 240.0),
        12.0,
        Color::srgba(0.28, 0.84, 1.0, 0.2),
    );
    let upper_center = frame_center + entry_heading * 96.0 + Vec3::Y * 88.0;
    append_rect_frame(
        &mut frame_builder,
        upper_center,
        (Quat::from_axis_angle(frame_normal, 0.58) * entry_heading).normalize_or_zero(),
        Vec3::Y,
        frame_normal,
        Vec2::new(250.0, 168.0),
        10.0,
        Color::srgba(1.0, 0.42, 0.86, 0.18),
    );
    let lower_center = frame_center + entry_heading * 156.0 - Vec3::Y * 72.0;
    append_rect_frame(
        &mut frame_builder,
        lower_center,
        (Quat::from_axis_angle(frame_normal, -0.32) * entry_heading).normalize_or_zero(),
        Vec3::Y,
        frame_normal,
        Vec2::new(210.0, 142.0),
        8.0,
        Color::srgba(0.54, 0.42, 1.0, 0.16),
    );
    append_beam_segment(
        &mut frame_builder,
        frame_center + entry_heading * 160.0 + Vec3::Y * 120.0,
        lower_center - entry_heading * 24.0 + Vec3::Y * 46.0,
        8.0,
        1.0,
        frame_normal,
        0.0,
        Color::srgba(0.98, 0.58, 0.9, 0.16),
    );
    if !frame_builder.is_empty() {
        commands.spawn((
            GeneratedWorld,
            AtmosphereMaterialKind::Megastructure,
            Name::new("Impossible Frames"),
            Mesh3d(meshes.add(frame_builder.build())),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::WHITE,
                emissive: LinearRgba::rgb(0.22, 0.18, 0.36),
                cull_mode: None,
                alpha_mode: AlphaMode::AlphaToCoverage,
                perceptual_roughness: 0.34,
                reflectance: 0.48,
                ..default()
            })),
            Transform::default(),
            NotShadowCaster,
            NotShadowReceiver,
        ));
    }

    let gate_mesh = meshes.add(Torus {
        minor_radius: 0.035,
        major_radius: 1.0,
    });
    let gate_step = (blueprint.rooms.len() / 5).max(5);
    for room_index in (3..blueprint.rooms.len().saturating_sub(2)).step_by(gate_step) {
        let room = &blueprint.rooms[room_index];
        let next_room = &blueprint.rooms[(room_index + 1).min(blueprint.rooms.len() - 1)];
        let forward = direction_from_delta(next_room.top - room.top);
        let scale_radius = room.size.max_element() * 1.9 + 22.0;
        let gate_color = mix_color(
            Color::srgb(0.3, 0.88, 1.0),
            Color::srgb(1.0, 0.46, 0.84),
            room_index as f32 / blueprint.rooms.len().max(1) as f32,
        );
        commands.spawn((
            GeneratedWorld,
            AtmosphereMaterialKind::Megastructure,
            Name::new("Flow Gate"),
            Mesh3d(gate_mesh.clone()),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: gate_color.with_alpha(0.15),
                emissive: LinearRgba::from(gate_color) * 0.26,
                alpha_mode: AlphaMode::AlphaToCoverage,
                cull_mode: None,
                unlit: true,
                ..default()
            })),
            Transform::from_translation(room.top + Vec3::Y * 18.0)
                .with_rotation(Quat::from_rotation_arc(Vec3::Z, forward))
                .with_scale(Vec3::new(scale_radius, scale_radius * 0.45, scale_radius)),
            NotShadowCaster,
            NotShadowReceiver,
        ));
    }

    let landmark_ring_mesh = meshes.add(Torus {
        minor_radius: 0.032,
        major_radius: 1.0,
    });
    let landmark_box_mesh = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let landmark_material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        emissive: LinearRgba::rgb(0.22, 0.18, 0.36),
        cull_mode: None,
        alpha_mode: AlphaMode::AlphaToCoverage,
        perceptual_roughness: 0.3,
        reflectance: 0.52,
        ..default()
    });

    for zone in &blueprint.zones {
        let start_room_index = zone
            .start_segment
            .min(blueprint.rooms.len().saturating_sub(1));
        let end_room_index = (zone.end_segment + 1).min(blueprint.rooms.len().saturating_sub(1));
        let start_room = &blueprint.rooms[start_room_index];
        let end_room = &blueprint.rooms[end_room_index];
        let zone_forward = direction_from_delta(end_room.top - start_room.top);
        let zone_forward = if zone_forward == Vec3::ZERO {
            entry_heading
        } else {
            zone_forward
        };
        let zone_right = Vec3::new(-zone_forward.z, 0.0, zone_forward.x).normalize_or_zero();
        let zone_center = start_room.top.lerp(end_room.top, 0.5);
        let zone_span = start_room.top.distance(end_room.top).max(120.0);
        let landmark_radius = LANDMARK_OFFSET_RADIUS
            + zone_span * rng.range_f32(0.04, 0.18)
            + match zone.index % 3 {
                0 => -56.0,
                1 => 12.0,
                _ => 104.0,
            };
        let landmark_vertical_offset = match zone.index % 4 {
            0 => LANDMARK_VERTICAL_OFFSET - 220.0,
            1 => LANDMARK_VERTICAL_OFFSET - 36.0,
            2 => LANDMARK_VERTICAL_OFFSET + 154.0,
            _ => LANDMARK_VERTICAL_OFFSET + 332.0,
        } + rng.range_f32(-56.0, 76.0);
        let anchor = zone_center
            + zone_right * zone.flow.drift_sign * landmark_radius
            + zone_forward * rng.range_f32(-48.0, 48.0)
            + Vec3::Y * landmark_vertical_offset;
        let zone_color = match zone.role {
            ZoneRole::Accelerator => Color::srgba(0.26, 0.88, 1.0, 0.16),
            ZoneRole::Technical => Color::srgba(1.0, 0.42, 0.82, 0.16),
            ZoneRole::Recovery => Color::srgba(0.54, 0.72, 1.0, 0.14),
            ZoneRole::Spectacle => Color::srgba(0.88, 0.48, 1.0, 0.18),
        };

        match zone.landmark {
            LandmarkKind::BrokenRing => {
                let mut builder = ColoredMeshBuilder::default();
                let radius = zone_span * 0.72 + 110.0;
                let gap_start = rng.range_f32(0.1, 0.32);
                let gap_end = gap_start + rng.range_f32(0.16, 0.28);
                let samples = 56;
                let mut previous = None;
                for sample in 0..=samples {
                    let t = sample as f32 / samples as f32;
                    if (gap_start..gap_end).contains(&t) {
                        previous = None;
                        continue;
                    }
                    let angle = TAU * t + zone.index as f32 * 0.33;
                    let radial = Vec3::new(angle.cos(), 0.0, angle.sin()).normalize_or_zero();
                    let point = anchor
                        + radial * radius
                        + Vec3::Y * ((t * TAU * 2.0).sin() * radius * 0.06);
                    if let Some(last) = previous {
                        append_beam_segment(
                            &mut builder,
                            last,
                            point,
                            10.0,
                            1.0,
                            radial,
                            angle * 0.25,
                            zone_color,
                        );
                    }
                    previous = Some(point);
                }
                commands.spawn((
                    GeneratedWorld,
                    AtmosphereMaterialKind::Megastructure,
                    Name::new("Zone Broken Ring"),
                    Mesh3d(meshes.add(builder.build())),
                    MeshMaterial3d(landmark_material.clone()),
                    Transform::default(),
                    NotShadowCaster,
                    NotShadowReceiver,
                ));
            }
            LandmarkKind::ImpossibleBridge => {
                let mut builder = ColoredMeshBuilder::default();
                let bridge_start = anchor - zone_forward * (zone_span * 0.55) + Vec3::Y * 22.0;
                let bridge_end = anchor + zone_forward * (zone_span * 0.55) - Vec3::Y * 18.0;
                append_beam_segment(
                    &mut builder,
                    bridge_start,
                    bridge_end,
                    16.0,
                    1.6,
                    zone_right,
                    0.0,
                    zone_color,
                );
                append_beam_segment(
                    &mut builder,
                    bridge_start + zone_right * 48.0 + Vec3::Y * 72.0,
                    bridge_end - zone_right * 56.0 + Vec3::Y * 48.0,
                    12.0,
                    1.2,
                    zone_right,
                    0.0,
                    brighten(zone_color, 0.12),
                );
                append_beam_segment(
                    &mut builder,
                    bridge_start + zone_right * 48.0 + Vec3::Y * 72.0,
                    bridge_start,
                    8.0,
                    1.0,
                    zone_forward,
                    0.0,
                    deepen(zone_color, 0.18),
                );
                commands.spawn((
                    GeneratedWorld,
                    AtmosphereMaterialKind::Megastructure,
                    Name::new("Zone Impossible Bridge"),
                    Mesh3d(meshes.add(builder.build())),
                    MeshMaterial3d(landmark_material.clone()),
                    Transform::default(),
                    NotShadowCaster,
                    NotShadowReceiver,
                ));
            }
            LandmarkKind::CorkscrewTower => {
                let mut builder = ColoredMeshBuilder::default();
                let tower_top = anchor + Vec3::Y * (zone_span * 0.24 + 120.0);
                append_beam_segment(
                    &mut builder,
                    anchor - Vec3::Y * 120.0,
                    tower_top,
                    14.0,
                    2.0,
                    zone_right,
                    0.0,
                    zone_color,
                );
                let turns = 2.4 * zone.flow.drift_sign;
                let samples = 48;
                let radius = zone_span * 0.18 + 34.0;
                let mut previous = None;
                for sample in 0..=samples {
                    let t = sample as f32 / samples as f32;
                    let angle = turns * TAU * t;
                    let radial = Quat::from_axis_angle(Vec3::Y, angle) * zone_right;
                    let point = anchor
                        + radial * radius
                        + Vec3::Y * lerp(-80.0, zone_span * 0.24 + 80.0, t);
                    if let Some(last) = previous {
                        append_beam_segment(
                            &mut builder,
                            last,
                            point,
                            9.0,
                            0.9,
                            radial,
                            angle * 0.3,
                            brighten(zone_color, 0.1),
                        );
                    }
                    previous = Some(point);
                }
                commands.spawn((
                    GeneratedWorld,
                    AtmosphereMaterialKind::Megastructure,
                    Name::new("Zone Corkscrew Tower"),
                    Mesh3d(meshes.add(builder.build())),
                    MeshMaterial3d(landmark_material.clone()),
                    Transform::default(),
                    NotShadowCaster,
                    NotShadowReceiver,
                ));
            }
            LandmarkKind::FloatingMonolithCluster => {
                for monolith in 0..5 {
                    let phase = TAU * (monolith as f32 / 5.0) + zone.index as f32 * 0.25;
                    let radial = Vec3::new(phase.cos(), 0.0, phase.sin()).normalize_or_zero();
                    let monolith_anchor = anchor
                        + radial * (36.0 + monolith as f32 * 22.0)
                        + Vec3::Y * (monolith as f32 * 18.0 - 24.0);
                    commands.spawn((
                        GeneratedWorld,
                        AtmosphereMaterialKind::Megastructure,
                        Name::new("Zone Monolith"),
                        Mesh3d(landmark_box_mesh.clone()),
                        MeshMaterial3d(landmark_material.clone()),
                        Transform::from_translation(monolith_anchor)
                            .with_rotation(Quat::from_euler(
                                EulerRot::YXZ,
                                phase,
                                0.0,
                                monolith as f32 * 0.08,
                            ))
                            .with_scale(Vec3::new(
                                18.0 + monolith as f32 * 5.0,
                                120.0 + monolith as f32 * 18.0,
                                18.0 + monolith as f32 * 3.0,
                            )),
                        NotShadowCaster,
                        NotShadowReceiver,
                    ));
                }
            }
            LandmarkKind::MovingMegastructure => {
                commands.spawn((
                    GeneratedWorld,
                    AtmosphereMaterialKind::Megastructure,
                    Name::new("Zone Moving Megastructure"),
                    Mesh3d(landmark_ring_mesh.clone()),
                    MeshMaterial3d(landmark_material.clone()),
                    Transform::from_translation(anchor)
                        .with_rotation(Quat::from_rotation_x(1.04) * Quat::from_rotation_y(0.3))
                        .with_scale(Vec3::new(
                            zone_span * 0.9 + 120.0,
                            zone_span * 0.18 + 24.0,
                            zone_span * 0.9 + 120.0,
                        )),
                    NotShadowCaster,
                    NotShadowReceiver,
                    SkyDrift {
                        anchor,
                        primary_axis: zone_forward,
                        secondary_axis: zone_right,
                        primary_amplitude: 36.0,
                        secondary_amplitude: 18.0,
                        vertical_amplitude: 12.0,
                        speed: 0.018,
                        rotation_speed: 0.018,
                        phase: rng.range_f32(0.0, TAU),
                        base_rotation: Quat::from_rotation_x(1.04)
                            * Quat::from_rotation_y(zone.index as f32 * 0.21),
                    },
                ));
            }
        }
    }
}

fn find_safe_celestial_anchor(
    blueprint: &RunBlueprint,
    rng: &mut RunRng,
    center: Vec3,
    course_radius: f32,
    radial: Vec3,
    tangent: Vec3,
    radius: f32,
    major: bool,
    vertical_bias: f32,
    depth_bias: f32,
) -> Vec3 {
    let near_orbit_min = if major {
        course_radius + 56.0 + radius * 0.24
    } else {
        course_radius + 68.0 + radius * 0.26
    };
    let near_orbit_max = if major {
        course_radius + 150.0 + radius * 0.46
    } else {
        course_radius + 140.0 + radius * 0.4
    };
    let far_orbit_min = if major {
        course_radius + 160.0 + radius * 0.58
    } else {
        course_radius + 170.0 + radius * 0.46
    };
    let far_orbit_max = if major {
        course_radius + 380.0 + radius * 0.9
    } else {
        course_radius + 300.0 + radius * 0.72
    };
    let orbit_min = if depth_bias < -0.35 {
        near_orbit_min
    } else if depth_bias > 0.45 {
        far_orbit_min
    } else {
        lerp(near_orbit_min, far_orbit_min, 0.45)
    };
    let orbit_max = if depth_bias < -0.35 {
        near_orbit_max
    } else if depth_bias > 0.45 {
        far_orbit_max
    } else {
        lerp(near_orbit_max, far_orbit_max, 0.58)
    };
    let altitude_center = vertical_bias
        * if major {
            320.0 + radius * 0.24
        } else {
            220.0 + radius * 0.18
        };
    let altitude_jitter = if major {
        120.0 + radius * 0.08
    } else {
        96.0 + radius * 0.06
    };
    let tangential_span = if major {
        180.0 + radius * 0.025
    } else {
        120.0 + radius * 0.02
    };
    let desired_clearance = if major {
        164.0 + radius * 0.055
    } else {
        134.0 + radius * 0.04
    };
    let desired_view_clearance = if major {
        196.0 + radius * 0.05
    } else {
        110.0 + radius * 0.035
    };

    let mut best = center + radial * orbit_max + Vec3::Y * altitude_center;
    let mut best_clearance = f32::NEG_INFINITY;
    let mut best_view_clearance = f32::NEG_INFINITY;
    let mut best_score = f32::NEG_INFINITY;
    let desired_orbit = lerp(orbit_min, orbit_max, (depth_bias * 0.5 + 0.5).clamp(0.0, 1.0));

    for _ in 0..24 {
        let orbit = rng.range_f32(orbit_min, orbit_max);
        let altitude = altitude_center + rng.range_f32(-altitude_jitter, altitude_jitter);
        let candidate = center
            + radial * orbit
            + tangent * rng.range_f32(-tangential_span, tangential_span)
            + Vec3::Y * altitude;
        let clearance = celestial_course_clearance(blueprint, candidate, radius);
        let view_clearance = celestial_entry_view_clearance(blueprint, candidate, radius);
        let orbit_alignment = 1.0 - ((orbit - desired_orbit).abs() / (orbit_max - orbit_min).max(1.0));
        let altitude_alignment =
            1.0 - ((altitude - altitude_center).abs() / altitude_jitter.max(1.0));
        let score = clearance.min(view_clearance)
            + orbit_alignment.max(0.0) * 28.0
            + altitude_alignment.max(0.0) * 22.0;
        if score > best_score {
            best = candidate;
            best_clearance = clearance;
            best_view_clearance = view_clearance;
            best_score = score;
        }
        if clearance >= desired_clearance
            && view_clearance >= desired_view_clearance
            && orbit_alignment >= 0.3
            && altitude_alignment >= 0.2
        {
            return candidate;
        }
    }

    if best_clearance < desired_clearance {
        let push = desired_clearance - best_clearance + 18.0;
        best += radial * push;
        let vertical_sign = if vertical_bias == 0.0 {
            1.0
        } else {
            vertical_bias.signum()
        };
        best.y += push * 0.2 * vertical_sign;
    }
    if best_view_clearance < desired_view_clearance {
        let push = desired_view_clearance - best_view_clearance + 36.0;
        let tangential_sign = {
            let sign = (best - center).dot(tangent).signum();
            if sign == 0.0 { 1.0 } else { sign }
        };
        best += tangent * (push * tangential_sign) + radial * (push * 0.35);
        best.y += push * 0.12;
    }

    best
}

fn celestial_entry_view_clearance(blueprint: &RunBlueprint, point: Vec3, radius: f32) -> f32 {
    if blueprint.rooms.len() < 2 {
        return f32::INFINITY;
    }

    let (center, course_radius) = course_visual_center_and_radius(blueprint);
    let forward = direction_from_delta(blueprint.rooms[1].top - blueprint.rooms[0].top);
    let eye = blueprint.spawn + Vec3::Y * (PLAYER_SPAWN_CLEARANCE + 1.4);
    let target = center
        + forward * (course_radius * 0.6)
        + Vec3::Y * ((blueprint.rooms[0].top.y - blueprint.spawn.y).abs() * 0.12 + 44.0);
    let corridor = target - eye;
    let length = corridor.length();
    if length <= f32::EPSILON {
        return f32::INFINITY;
    }

    let direction = corridor / length;
    let along = (point - eye).dot(direction).clamp(0.0, length);
    let closest = eye + direction * along;
    point.distance(closest) - radius
}

fn celestial_course_clearance(blueprint: &RunBlueprint, point: Vec3, radius: f32) -> f32 {
    let room_clearance = blueprint
        .rooms
        .iter()
        .map(|room| {
            let room_radius = room.size.max_element() * 0.9 + ROOM_CLEARANCE_HEIGHT * 0.5;
            point.distance(room.top) - room_radius - radius
        })
        .fold(f32::INFINITY, f32::min);
    let spawn_clearance =
        point.distance(blueprint.spawn) - (radius + PLAYER_SPAWN_CLEARANCE + 32.0);
    let summit_clearance = point.distance(blueprint.summit) - (radius + SUMMIT_RADIUS + 28.0);
    room_clearance.min(spawn_clearance).min(summit_clearance)
}

fn celestial_course_escape_direction(blueprint: &RunBlueprint, point: Vec3) -> Vec3 {
    let mut best_clearance = f32::INFINITY;
    let mut best_direction = Vec3::Y;

    for room in &blueprint.rooms {
        let room_radius = room.size.max_element() * 0.9 + ROOM_CLEARANCE_HEIGHT * 0.5;
        let delta = point - room.top;
        let clearance = delta.length() - room_radius;
        if clearance < best_clearance {
            best_clearance = clearance;
            best_direction = delta.normalize_or_zero();
        }
    }

    for (source, source_radius) in [
        (blueprint.spawn, PLAYER_SPAWN_CLEARANCE + 32.0),
        (blueprint.summit, SUMMIT_RADIUS + 28.0),
    ] {
        let delta = point - source;
        let clearance = delta.length() - source_radius;
        if clearance < best_clearance {
            best_clearance = clearance;
            best_direction = delta.normalize_or_zero();
        }
    }

    if best_direction == Vec3::ZERO {
        Vec3::Y
    } else {
        best_direction
    }
}

fn spawn_layout(
    layout: &ModuleLayout,
    chunk: Option<WorldChunkKey>,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_cache: &mut WorldAssetCache,
) {
    let mut render_batches = HashMap::<GameMaterialKey, ColoredMeshBuilder>::default();
    for solid in &layout.solids {
        if is_batchable_static_render(solid) {
            append_static_render_geometry(&mut render_batches, solid);
            spawn_box_collider_spec(solid, commands, chunk);
        } else if is_collider_only_static(solid) {
            spawn_box_collider_spec(solid, commands, chunk);
        } else {
            spawn_box_spec(solid, chunk, commands, meshes, materials, asset_cache);
        }
    }

    for (material_key, builder) in render_batches {
        if builder.is_empty() {
            continue;
        }
        let mut entity = commands.spawn((
            GeneratedWorld,
            Name::new("Static Render Batch"),
            Mesh3d(meshes.add(builder.build())),
            MeshMaterial3d(cached_game_material(asset_cache, materials, material_key)),
            Transform::default(),
        ));
        if material_key.surf {
            entity.insert(NotShadowCaster);
        }
        if let Some(chunk) = chunk {
            entity.insert(ChunkMember(chunk));
        }
    }

    for feature in &layout.features {
        match feature {
            FeatureSpec::CheckpointPad { center, index } => {
                let mut entity = commands.spawn((
                    GeneratedWorld,
                    Name::new("Checkpoint Pad"),
                    Transform::from_translation(*center),
                    CheckpointPad { index: *index },
                ));
                if let Some(chunk) = chunk {
                    entity.insert(ChunkMember(chunk));
                }
            }
            FeatureSpec::WindZone {
                center,
                size,
                direction,
                strength,
                gust,
            } => {
                let mut entity = commands.spawn((
                    GeneratedWorld,
                    Name::new("Wind Zone"),
                    Mesh3d(cached_cuboid_mesh(asset_cache, meshes, *size)),
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color: Color::srgba(0.4, 0.75, 1.0, 0.14),
                        alpha_mode: AlphaMode::AlphaToCoverage,
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
                if let Some(chunk) = chunk {
                    entity.insert(ChunkMember(chunk));
                }
            }
            FeatureSpec::PointLight {
                center,
                intensity,
                range,
                color,
            } => {
                let mut entity = commands.spawn((
                    GeneratedWorld,
                    Name::new("Beacon Light"),
                    PointLight {
                        intensity: *intensity,
                        range: *range,
                        color: *color,
                        shadows_enabled: false,
                        ..default()
                    },
                    Transform::from_translation(*center),
                ));
                if let Some(chunk) = chunk {
                    entity.insert(ChunkMember(chunk));
                }
            }
        }
    }
}

fn spawn_box_spec(
    spec: &SolidSpec,
    chunk: Option<WorldChunkKey>,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_cache: &mut WorldAssetCache,
) {
    if matches!(&spec.body, SolidBody::StaticSurfStrip { .. }) {
        return;
    }

    if let Err(reason) = validate_solid_spec(spec) {
        eprintln!("Skipping invalid solid '{}': {}", spec.label, reason);
        return;
    }

    let mesh = match &spec.body {
        SolidBody::StaticSurfWedge { render_points, .. } => meshes.add(build_surf_wedge_mesh(
            render_points,
            paint_base_color(spec.paint, false),
            paint_stripe_color(spec.paint),
        )),
        SolidBody::StaticSphere => meshes.add(
            Sphere::new(spec.size.x * 0.5)
                .mesh()
                .ico(5)
                .expect("icosphere build should succeed"),
        ),
        SolidBody::StaticCylinder => meshes.add(Cylinder::new(spec.size.x * 0.5, spec.size.y)),
        SolidBody::StaticTrapezoid { top_scale } => meshes.add(build_trapezoid_mesh(
            spec.size,
            *top_scale,
            paint_base_color(spec.paint, false),
        )),
        _ => cached_cuboid_mesh(asset_cache, meshes, spec.size),
    };
    let material = cached_game_material(
        asset_cache,
        materials,
        GameMaterialKey {
            paint: spec.paint,
            ghost: false,
            surf: matches!(&spec.body, SolidBody::StaticSurfWedge { .. }),
            vertex_colored: matches!(
                &spec.body,
                SolidBody::StaticSurfWedge { .. } | SolidBody::StaticTrapezoid { .. }
            ),
        },
    );

    let transform = Transform::from_translation(spec.center);

    let mut entity = commands.spawn((
        GeneratedWorld,
        Name::new(spec.label.clone()),
        Mesh3d(mesh),
        MeshMaterial3d(material),
        transform,
    ));
    if let Some(chunk) = chunk {
        entity.insert(ChunkMember(chunk));
    }

    match &spec.body {
        SolidBody::Static => {
            entity.insert((
                RigidBody::Static,
                Collider::cuboid(spec.size.x, spec.size.y, spec.size.z),
                CollisionLayers::new(CollisionLayer::Default, LayerMask::ALL),
            ));
        }
        SolidBody::StaticSphere => {
            entity.insert((
                RigidBody::Static,
                Collider::sphere(spec.size.x * 0.5),
                CollisionLayers::new(CollisionLayer::Default, LayerMask::ALL),
            ));
        }
        SolidBody::StaticCylinder => {
            entity.insert((
                RigidBody::Static,
                Collider::cylinder(spec.size.x * 0.5, spec.size.y),
                CollisionLayers::new(CollisionLayer::Default, LayerMask::ALL),
            ));
        }
        SolidBody::StaticTrapezoid { top_scale } => {
            if let Some(collider) = Collider::convex_hull(trapezoid_points(spec.size, *top_scale)) {
                entity.insert((
                    RigidBody::Static,
                    collider,
                    CollisionLayers::new(CollisionLayer::Default, LayerMask::ALL),
                ));
            }
        }
        SolidBody::StaticSurfWedge { .. } => {}
        SolidBody::StaticSurfStrip {
            collider_strip_points,
            ..
        } => {
            if let Some(mesh) =
                build_surf_strip_collider_mesh(collider_strip_points, SURF_COLLIDER_COLUMNS)
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
            }
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
    }

    if let Some(friction) = spec.friction {
        entity.insert(Friction::new(friction));
    }

    match spec.extra {
        ExtraKind::None => {}
        ExtraKind::SummitGoal => {
            entity.insert(SummitGoal);
        }
    }
}

#[derive(Default)]
struct ColoredMeshBuilder {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
}

impl ColoredMeshBuilder {
    fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

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

fn mix_color(from: Color, to: Color, amount: f32) -> Color {
    Color::from(LinearRgba::from(from).mix(&LinearRgba::from(to), amount.clamp(0.0, 1.0)))
}

fn brighten(color: Color, amount: f32) -> Color {
    mix_color(color, Color::WHITE, amount)
}

fn deepen(color: Color, amount: f32) -> Color {
    mix_color(color, Color::BLACK, amount)
}

fn dim_linear(color: Color, scale: f32, alpha: f32) -> Color {
    let linear = LinearRgba::from(color);
    Color::from(LinearRgba::new(
        linear.red * scale,
        linear.green * scale,
        linear.blue * scale,
        alpha,
    ))
}

fn is_batchable_static_render(spec: &SolidSpec) -> bool {
    matches!(spec.extra, ExtraKind::None)
        && matches!(&spec.body, SolidBody::Static | SolidBody::StaticSurfWedge { .. })
}

fn is_collider_only_static(spec: &SolidSpec) -> bool {
    matches!(spec.extra, ExtraKind::None) && matches!(&spec.body, SolidBody::StaticSurfStrip { .. })
}

fn append_static_render_geometry(
    batches: &mut HashMap<GameMaterialKey, ColoredMeshBuilder>,
    spec: &SolidSpec,
) {
    let material_key = GameMaterialKey {
        paint: spec.paint,
        ghost: false,
        surf: matches!(&spec.body, SolidBody::StaticSurfWedge { .. }),
        vertex_colored: true,
    };
    let builder = batches.entry(material_key).or_default();
    let base_color = paint_base_color(spec.paint, false);

    match &spec.body {
        SolidBody::Static => {
            append_box_render_geometry(builder, spec.center, spec.size, base_color)
        }
        SolidBody::StaticSurfWedge { render_points, .. } if !render_points.is_empty() => {
            append_surf_wedge_render_geometry(
                builder,
                spec.center,
                render_points,
                base_color,
                paint_stripe_color(spec.paint),
            );
        }
        _ => {}
    }
}

fn append_oriented_box_render_geometry(
    builder: &mut ColoredMeshBuilder,
    center: Vec3,
    half_extents: Vec3,
    right: Vec3,
    up: Vec3,
    forward: Vec3,
    color: Color,
) {
    let right = right.normalize_or_zero();
    let up = up.normalize_or_zero();
    let forward = forward.normalize_or_zero();
    if right == Vec3::ZERO || up == Vec3::ZERO || forward == Vec3::ZERO {
        return;
    }

    let hx = right * half_extents.x;
    let hy = up * half_extents.y;
    let hz = forward * half_extents.z;
    let nbl = center - hx - hy - hz;
    let nbr = center + hx - hy - hz;
    let nfr = center + hx - hy + hz;
    let nfl = center - hx - hy + hz;
    let tbl = center - hx + hy - hz;
    let tbr = center + hx + hy - hz;
    let tfr = center + hx + hy + hz;
    let tfl = center - hx + hy + hz;

    let top = brighten(color, 0.12);
    let bottom = deepen(color, 0.45);
    let front = brighten(color, 0.04);
    let back = deepen(color, 0.22);
    let left = deepen(color, 0.12);
    let right_face = brighten(color, 0.08);

    builder.push_quad(tfl, tfr, tbr, tbl, top);
    builder.push_quad(nbl, nbr, nfr, nfl, bottom);
    builder.push_quad(nfl, nfr, tfr, tfl, front);
    builder.push_quad(nbr, nbl, tbl, tbr, back);
    builder.push_quad(nbl, nfl, tfl, tbl, left);
    builder.push_quad(nfr, nbr, tbr, tfr, right_face);
}

fn append_beam_segment(
    builder: &mut ColoredMeshBuilder,
    from: Vec3,
    to: Vec3,
    width: f32,
    height: f32,
    up_hint: Vec3,
    roll: f32,
    color: Color,
) {
    let delta = to - from;
    let length = delta.length();
    if length < 0.01 {
        return;
    }

    let forward = delta / length;
    let mut right = forward.cross(up_hint).normalize_or_zero();
    if right == Vec3::ZERO {
        right = forward.cross(Vec3::Y).normalize_or_zero();
    }
    if right == Vec3::ZERO {
        right = forward.cross(Vec3::X).normalize_or_zero();
    }
    if right == Vec3::ZERO {
        return;
    }

    let mut up = right.cross(forward).normalize_or_zero();
    if up == Vec3::ZERO {
        return;
    }

    let roll_rotation = Quat::from_axis_angle(forward, roll);
    let right = roll_rotation * right;
    up = roll_rotation * up;

    append_oriented_box_render_geometry(
        builder,
        (from + to) * 0.5,
        Vec3::new(width * 0.5, height * 0.5, length * 0.5),
        right,
        up,
        forward,
        color,
    );
}

fn append_rect_frame(
    builder: &mut ColoredMeshBuilder,
    center: Vec3,
    right: Vec3,
    up: Vec3,
    normal: Vec3,
    size: Vec2,
    thickness: f32,
    color: Color,
) {
    let right = right.normalize_or_zero();
    let up = up.normalize_or_zero();
    let normal = normal.normalize_or_zero();
    if right == Vec3::ZERO || up == Vec3::ZERO || normal == Vec3::ZERO {
        return;
    }

    let half_width = right * (size.x * 0.5);
    let half_height = up * (size.y * 0.5);
    let top_left = center - half_width + half_height;
    let top_right = center + half_width + half_height;
    let bottom_left = center - half_width - half_height;
    let bottom_right = center + half_width - half_height;

    append_beam_segment(
        builder,
        top_left,
        top_right,
        thickness,
        thickness * 0.5,
        normal,
        0.0,
        color,
    );
    append_beam_segment(
        builder,
        bottom_left,
        bottom_right,
        thickness,
        thickness * 0.5,
        normal,
        0.0,
        color,
    );
    append_beam_segment(
        builder,
        top_left,
        bottom_left,
        thickness,
        thickness * 0.5,
        normal,
        0.0,
        color,
    );
    append_beam_segment(
        builder,
        top_right,
        bottom_right,
        thickness,
        thickness * 0.5,
        normal,
        0.0,
        color,
    );
}

fn append_box_render_geometry(
    builder: &mut ColoredMeshBuilder,
    center: Vec3,
    size: Vec3,
    color: Color,
) {
    let half = size * 0.5;
    let nbl = center + Vec3::new(-half.x, -half.y, -half.z);
    let nbr = center + Vec3::new(half.x, -half.y, -half.z);
    let nfr = center + Vec3::new(half.x, -half.y, half.z);
    let nfl = center + Vec3::new(-half.x, -half.y, half.z);
    let tbl = center + Vec3::new(-half.x, half.y, -half.z);
    let tbr = center + Vec3::new(half.x, half.y, -half.z);
    let tfr = center + Vec3::new(half.x, half.y, half.z);
    let tfl = center + Vec3::new(-half.x, half.y, half.z);

    let top = brighten(color, 0.12);
    let bottom = deepen(color, 0.45);
    let front = brighten(color, 0.04);
    let back = deepen(color, 0.22);
    let left = deepen(color, 0.12);
    let right = brighten(color, 0.08);

    builder.push_quad(tfl, tfr, tbr, tbl, top);
    builder.push_quad(nbl, nbr, nfr, nfl, bottom);
    builder.push_quad(nfl, nfr, tfr, tfl, front);
    builder.push_quad(nbr, nbl, tbl, tbr, back);
    builder.push_quad(nbl, nfl, tfl, tbl, left);
    builder.push_quad(nfr, nbr, tbr, tfr, right);
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

    let ridge_face = brighten(base_color, 0.14).with_alpha(0.42);
    let outer_face = brighten(base_color, 0.08).with_alpha(0.34);
    let underside = deepen(base_color, 0.28).with_alpha(0.28);
    let side_shadow = deepen(base_color, 0.18).with_alpha(0.3);

    builder.push_quad(e, g, h, f, underside);
    builder.push_quad(a, c, g, e, outer_face);
    builder.push_quad(b, f, h, d, side_shadow);
    builder.push_quad(a, e, f, b, deepen(base_color, 0.08).with_alpha(0.24));
    builder.push_quad(c, d, h, g, deepen(base_color, 0.14).with_alpha(0.22));

    let front_ridge = a;
    let back_ridge = b;
    let front_outer = c;
    let back_outer = d;

    let stripe_core = mix_color(ridge_face, stripe_color.with_alpha(0.64), 0.82);
    let stripe_glow = mix_color(ridge_face, dim_linear(stripe_color, 0.52, 0.5), 0.52);
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

fn trapezoid_points(size: Vec3, top_scale: f32) -> Vec<Vec3> {
    let top_scale = top_scale.clamp(0.08, 0.92);
    let half = size * 0.5;
    let top_half = Vec3::new(half.x * top_scale, half.y, half.z * top_scale);
    let bottom_half = Vec3::new(half.x, half.y, half.z);
    vec![
        Vec3::new(-top_half.x, top_half.y, -top_half.z),
        Vec3::new(top_half.x, top_half.y, -top_half.z),
        Vec3::new(top_half.x, top_half.y, top_half.z),
        Vec3::new(-top_half.x, top_half.y, top_half.z),
        Vec3::new(-bottom_half.x, -bottom_half.y, -bottom_half.z),
        Vec3::new(bottom_half.x, -bottom_half.y, -bottom_half.z),
        Vec3::new(bottom_half.x, -bottom_half.y, bottom_half.z),
        Vec3::new(-bottom_half.x, -bottom_half.y, bottom_half.z),
    ]
}

fn build_trapezoid_mesh(size: Vec3, top_scale: f32, base_color: Color) -> Mesh {
    let points = trapezoid_points(size, top_scale);
    let [a, b, c, d, e, f, g, h]: [Vec3; 8] = points.try_into().unwrap();
    let mut builder = ColoredMeshBuilder::default();
    builder.push_quad(a, b, c, d, brighten(base_color, 0.12));
    builder.push_quad(e, h, g, f, deepen(base_color, 0.32));
    builder.push_quad(a, e, f, b, deepen(base_color, 0.08));
    builder.push_quad(b, f, g, c, brighten(base_color, 0.03));
    builder.push_quad(c, g, h, d, deepen(base_color, 0.16));
    builder.push_quad(d, h, e, a, brighten(base_color, 0.07));
    builder.build()
}

fn spawn_box_collider_spec(
    spec: &SolidSpec,
    commands: &mut Commands,
    chunk: Option<WorldChunkKey>,
) {
    let should_spawn = match &spec.body {
        SolidBody::Static => true,
        SolidBody::StaticSurfStrip {
            collider_strip_points,
            ..
        } => !collider_strip_points.is_empty(),
        _ => false,
    };
    if !should_spawn {
        return;
    }

    let mut entity = commands.spawn((
        GeneratedWorld,
        Name::new(spec.label.clone()),
        Transform::from_translation(spec.center),
        GlobalTransform::default(),
    ));
    if let Some(chunk) = chunk {
        entity.insert(ChunkMember(chunk));
    }

    match &spec.body {
        SolidBody::Static => {
            entity.insert((
                RigidBody::Static,
                Collider::cuboid(spec.size.x, spec.size.y, spec.size.z),
                CollisionLayers::new(CollisionLayer::Default, LayerMask::ALL),
            ));
        }
        SolidBody::StaticSurfStrip {
            collider_strip_points,
            ..
        } => {
            if let Some(mesh) =
                build_surf_strip_collider_mesh(collider_strip_points, SURF_COLLIDER_COLUMNS)
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
            }
        }
        _ => {}
    }

    if let Some(friction) = spec.friction {
        entity.insert(Friction::new(friction));
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

fn cached_game_material(
    cache: &mut WorldAssetCache,
    materials: &mut Assets<StandardMaterial>,
    key: GameMaterialKey,
) -> Handle<StandardMaterial> {
    if let Some(material) = cache.gameplay_materials.get(&key) {
        return material.clone();
    }

    let mut material = material_for_paint(key.paint, key.ghost);
    if key.vertex_colored {
        material.base_color = Color::WHITE;
    }
    if key.surf {
        material.base_color = material.base_color.with_alpha(0.34);
        material.cull_mode = None;
        material.perceptual_roughness = 0.12;
        material.reflectance = 0.76;
        material.clearcoat = 0.88;
        material.clearcoat_perceptual_roughness = 0.08;
        material.metallic = 0.0;
        material.specular_tint = brighten(paint_base_color(key.paint, key.ghost), 0.12);
        material.emissive = LinearRgba::BLACK;
        material.alpha_mode = AlphaMode::Blend;
        material.specular_transmission = 0.0;
        material.diffuse_transmission = 0.0;
        material.thickness = 0.0;
        material.ior = 1.0;
    }

    let handle = materials.add(material);
    cache.gameplay_materials.insert(key, handle.clone());
    handle
}

fn paint_base_color(paint: PaintStyle, ghost: bool) -> Color {
    material_for_paint(paint, ghost).base_color
}

fn validate_solid_spec(spec: &SolidSpec) -> Result<(), String> {
    if !spec.center.is_finite() {
        return Err(format!("non-finite center {:?}", spec.center));
    }
    if !spec.size.is_finite() {
        return Err(format!("non-finite size {:?}", spec.size));
    }
    if spec.size.x <= 0.01 || spec.size.y <= 0.01 || spec.size.z <= 0.01 {
        return Err(format!("non-positive size {:?}", spec.size));
    }

    if let SolidBody::Moving { end, .. } = &spec.body
        && !end.is_finite()
    {
        return Err(format!("non-finite mover end {:?}", end));
    }

    let needs_collider = !matches!(
        &spec.body,
        SolidBody::Decoration | SolidBody::StaticSurfWedge { .. }
    );
    if !needs_collider {
        return Ok(());
    }

    let aabb = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match &spec.body {
        SolidBody::StaticSphere => {
            Collider::sphere(spec.size.x * 0.5).aabb(spec.center, Quat::IDENTITY)
        }
        SolidBody::StaticCylinder => {
            Collider::cylinder(spec.size.x * 0.5, spec.size.y).aabb(spec.center, Quat::IDENTITY)
        }
        SolidBody::StaticTrapezoid { top_scale } => {
            Collider::convex_hull(trapezoid_points(spec.size, *top_scale))
                .unwrap()
                .aabb(spec.center, Quat::IDENTITY)
        }
        SolidBody::StaticSurfStrip {
            collider_strip_points,
            ..
        } => build_surf_strip_collider_mesh(collider_strip_points, SURF_COLLIDER_COLUMNS)
            .and_then(|mesh| {
                Collider::trimesh_from_mesh_with_config(&mesh, TrimeshFlags::FIX_INTERNAL_EDGES)
            })
            .unwrap()
            .aabb(spec.center, Quat::IDENTITY),
        _ => Collider::cuboid(spec.size.x, spec.size.y, spec.size.z)
            .aabb(spec.center, Quat::IDENTITY),
    }))
    .map_err(|_| "collider AABB construction panicked".to_string())?;

    if !aabb.min.is_finite() || !aabb.max.is_finite() {
        return Err(format!(
            "non-finite collider AABB min {:?} max {:?}",
            aabb.min, aabb.max
        ));
    }

    Ok(())
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
        paint: PaintStyle::SectionPlatform(room.theme),
        body: SolidBody::Static,
        friction: None,
        extra: ExtraKind::None,
    });

    if let Some(index) = room.checkpoint_slot {
        layout.features.push(FeatureSpec::CheckpointPad {
            center: room.top,
            index,
        });
    }

    match room.section {
        RoomSectionKind::OpenPad => {}
        RoomSectionKind::SplitPad => {
            for side in [-1.0, 1.0] {
                layout.solids.push(SolidSpec {
                    owner,
                    label: "Split Rest Wing".into(),
                    center: top_to_center(
                        room.top + Vec3::new(side * room.size.x * 0.23, 0.28, 0.0),
                        0.4,
                    ),
                    size: Vec3::new(room.size.x * 0.34, 0.4, room.size.y * 0.46),
                    paint: PaintStyle::SectionPlatform(room.theme),
                    body: SolidBody::Static,
                    friction: None,
                    extra: ExtraKind::None,
                });
            }
            layout.solids.push(SolidSpec {
                owner,
                label: "Split Rest Spine".into(),
                center: top_to_center(room.top + Vec3::Y * 0.22, 0.28),
                size: Vec3::new(room.size.x * 0.18, 0.28, room.size.y * 0.74),
                paint: PaintStyle::SectionPlatform(room.theme),
                body: SolidBody::Static,
                friction: None,
                extra: ExtraKind::None,
            });
        }
        RoomSectionKind::Terrace => {
            let along_x = room.seed & 1 == 0;
            let sign = if room.seed & 2 == 0 { 1.0 } else { -1.0 };
            let offset = if along_x {
                Vec3::X * sign * room.size.x * 0.16
            } else {
                Vec3::Z * sign * room.size.y * 0.16
            };
            let terrace_size = if along_x {
                Vec3::new(room.size.x * 0.42, 0.5, room.size.y * 0.72)
            } else {
                Vec3::new(room.size.x * 0.72, 0.5, room.size.y * 0.42)
            };
            layout.solids.push(SolidSpec {
                owner,
                label: "Terrace Shelf".into(),
                center: top_to_center(room.top + offset + Vec3::Y * 0.34, 0.5),
                size: terrace_size,
                paint: PaintStyle::SectionPlatform(room.theme),
                body: SolidBody::Static,
                friction: None,
                extra: ExtraKind::None,
            });
        }
        RoomSectionKind::CornerPerches => {
            for x in [-1.0, 1.0] {
                for z in [-1.0, 1.0] {
                    layout.solids.push(SolidSpec {
                        owner,
                        label: "Corner Perch".into(),
                        center: top_to_center(
                            room.top
                                + Vec3::new(x * room.size.x * 0.23, 0.18, z * room.size.y * 0.23),
                            0.34,
                        ),
                        size: Vec3::new(room.size.x * 0.24, 0.34, room.size.y * 0.24),
                        paint: PaintStyle::SectionPlatform(room.theme),
                        body: SolidBody::Static,
                        friction: None,
                        extra: ExtraKind::None,
                    });
                }
            }
        }
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

fn surf_path_style_from_flow(flow: FlowFieldProfile) -> PathLateralStyle {
    match flow.path_style {
        PathLateralStyle::Straight => PathLateralStyle::Straight,
        PathLateralStyle::Arc | PathLateralStyle::OneSidedArc => flow.path_style,
        PathLateralStyle::Serpentine | PathLateralStyle::Switchback => {
            PathLateralStyle::OneSidedArc
        }
    }
}

fn append_connector_geometry(
    layout: &mut ModuleLayout,
    owner: OwnerTag,
    _rng: &mut RunRng,
    connector: ConnectorKind,
    start: Vec3,
    end: Vec3,
    forward: Vec3,
    right: Vec3,
    theme: Theme,
    _difficulty: f32,
    flow: FlowFieldProfile,
) {
    let along_x = forward.x.abs() > 0.5;
    let span = (end - start).xz().length().max(8.0);
    let center = start.lerp(end, 0.5);
    let flow_bias = flow.width_scale.clamp(0.9, 1.3);
    match connector {
        ConnectorKind::Funnel => {
            for side in [-1.0_f32, 1.0] {
                layout.solids.push(SolidSpec {
                    owner,
                    label: format!("Connector Funnel Rail {side}"),
                    center: center + right * side * (4.6 * flow_bias) + Vec3::Y * 0.08,
                    size: axis_box_size(along_x, span * 0.18, 0.08, 0.08),
                    paint: PaintStyle::ThemeAccent(theme),
                    body: SolidBody::Decoration,
                    friction: None,
                    extra: ExtraKind::None,
                });
            }
        }
        ConnectorKind::Booster => {}
        ConnectorKind::Transfer => {}
        ConnectorKind::Collector => {
            for side in [-1.0_f32, 1.0] {
                layout.solids.push(SolidSpec {
                    owner,
                    label: format!("Connector Collector Rail {side}"),
                    center: end - forward * 1.4 + right * side * (4.2 * flow_bias) + Vec3::Y * 0.06,
                    size: axis_box_size(along_x, span.min(10.0) * 0.16, 0.06, 0.08),
                    paint: PaintStyle::ThemeFloor(theme),
                    body: SolidBody::Decoration,
                    friction: None,
                    extra: ExtraKind::None,
                });
            }
        }
        ConnectorKind::Splitter => {}
        ConnectorKind::Crossover => {}
    }
}

fn signature_lane_span(from: &RoomPlan, to: &RoomPlan, signature: ZoneSignature) -> f32 {
    let base = from.size.min_element().min(to.size.min_element()) * 0.22;
    let signature_scale = match signature {
        ZoneSignature::BowlCollector => 1.18,
        ZoneSignature::GiantCorkscrew => 0.88,
        ZoneSignature::BraidLanes => 1.14,
        ZoneSignature::WaveRamps => 1.0,
        ZoneSignature::SplitTransfer => 1.08,
        ZoneSignature::PillarForest => 1.22,
        ZoneSignature::ShapeGarden => 1.28,
    };
    (base * signature_scale).clamp(ROUTE_LINE_LATERAL_SPAN * 0.7, ROUTE_LINE_LATERAL_SPAN * 1.9)
}

fn route_line_priority(line: RouteLine) -> usize {
    match line {
        RouteLine::Safe => 0,
        RouteLine::Speed => 1,
        RouteLine::Trick => 2,
    }
}

fn route_line_label_prefix(line: RouteLine) -> &'static str {
    match line {
        RouteLine::Safe => "Safe Line",
        RouteLine::Speed => "Speed Line",
        RouteLine::Trick => "Trick Line",
    }
}

fn route_line_from_label(label: &str) -> Option<RouteLine> {
    if label.starts_with("Safe Line ") {
        Some(RouteLine::Safe)
    } else if label.starts_with("Speed Line ") {
        Some(RouteLine::Speed)
    } else if label.starts_with("Trick Line ") {
        Some(RouteLine::Trick)
    } else {
        None
    }
}

fn route_line_module_half_width(kind: ModuleKind, line: RouteLine) -> f32 {
    let base = match kind {
        ModuleKind::SurfRamp => 8.8,
        ModuleKind::ShapeGauntlet => 7.8,
        ModuleKind::MovingPlatformRun => 5.2,
        ModuleKind::PillarAirstrafe => 4.8,
        ModuleKind::WindowHop => 4.6,
        ModuleKind::HeadcheckRun => 4.4,
        ModuleKind::SpeedcheckRun => 4.2,
        ModuleKind::StairRun => 4.0,
        _ => 4.4,
    };
    match line {
        RouteLine::Safe => base * 1.04,
        RouteLine::Speed => base,
        RouteLine::Trick => base * 0.96,
    }
}

fn route_line_lane_gap(signature: ZoneSignature) -> f32 {
    match signature {
        ZoneSignature::BowlCollector => ROUTE_LINE_MIN_CORRIDOR_GAP + 0.8,
        ZoneSignature::GiantCorkscrew => ROUTE_LINE_MIN_CORRIDOR_GAP + 1.0,
        ZoneSignature::BraidLanes => ROUTE_LINE_MIN_CORRIDOR_GAP + 1.4,
        ZoneSignature::WaveRamps => ROUTE_LINE_MIN_CORRIDOR_GAP + 0.4,
        ZoneSignature::SplitTransfer => ROUTE_LINE_MIN_CORRIDOR_GAP + 1.0,
        ZoneSignature::PillarForest => ROUTE_LINE_MIN_CORRIDOR_GAP + 1.6,
        ZoneSignature::ShapeGarden => ROUTE_LINE_MIN_CORRIDOR_GAP + 2.1,
    }
}

fn route_line_pair_gap(left: ModuleKind, right: ModuleKind, signature: ZoneSignature) -> f32 {
    let mut gap = route_line_lane_gap(signature);
    if matches!(left, ModuleKind::SurfRamp) || matches!(right, ModuleKind::SurfRamp) {
        gap += 5.4;
    }
    if matches!(left, ModuleKind::ShapeGauntlet) || matches!(right, ModuleKind::ShapeGauntlet) {
        gap += 4.6;
    }
    if matches!(left, ModuleKind::MovingPlatformRun)
        || matches!(right, ModuleKind::MovingPlatformRun)
    {
        gap += 1.8;
    }
    gap
}

fn route_line_vertical_bias(kind: ModuleKind, line: RouteLine) -> f32 {
    match line {
        RouteLine::Safe => match kind {
            ModuleKind::SurfRamp => -0.2,
            _ => -0.9,
        },
        RouteLine::Speed => 0.0,
        RouteLine::Trick => {
            ROUTE_LINE_TRICK_VERTICAL_BIAS
                + match kind {
                    ModuleKind::ShapeGauntlet => 1.2,
                    ModuleKind::MovingPlatformRun => 0.7,
                    ModuleKind::PillarAirstrafe => 0.45,
                    _ => 0.25,
                }
        }
    }
}

fn route_line_forward_margins(kind: ModuleKind, line: RouteLine, distance: f32) -> (f32, f32) {
    match kind {
        ModuleKind::SurfRamp => match line {
            RouteLine::Speed => (
                (distance * 0.014).clamp(1.8, 4.2),
                (distance * 0.012).clamp(1.6, 3.8),
            ),
            _ => (
                (distance * 0.035).clamp(3.4, 7.6),
                (distance * 0.03).clamp(3.0, 6.8),
            ),
        },
        ModuleKind::ShapeGauntlet => (
            (distance * 0.085).clamp(8.0, 16.0),
            (distance * 0.075).clamp(7.0, 14.0),
        ),
        ModuleKind::MovingPlatformRun => (
            (distance * 0.07).clamp(6.8, 13.0),
            (distance * 0.062).clamp(6.0, 11.5),
        ),
        _ if matches!(line, RouteLine::Speed) => (
            (distance * 0.03).clamp(3.0, 6.0),
            (distance * 0.026).clamp(2.6, 5.2),
        ),
        _ => (
            (distance * 0.072).clamp(7.0, 13.5),
            (distance * 0.064).clamp(6.2, 12.0),
        ),
    }
}

fn route_line_center_offsets(
    segment: &SegmentPlan,
    from: &RoomPlan,
    to: &RoomPlan,
) -> Vec<(RouteLine, ModuleKind, f32)> {
    let lane_span = signature_lane_span(from, to, segment.zone_signature);
    let mut reservations = segment
        .route_lines
        .iter()
        .copied()
        .into_iter()
        .map(|line| {
            let kind = route_line_module_kind(segment.zone_signature, line, segment.kind);
            let half_width = route_line_module_half_width(kind, line);
            (line, kind, half_width, route_line_priority(line))
        })
        .collect::<Vec<_>>();

    if reservations.len() >= 3
        && reservations
            .iter()
            .any(|(_, kind, _, _)| matches!(kind, ModuleKind::SurfRamp))
    {
        let drop_line = match segment.zone_role {
            ZoneRole::Recovery => RouteLine::Trick,
            ZoneRole::Accelerator | ZoneRole::Technical | ZoneRole::Spectacle => RouteLine::Safe,
        };
        if let Some(position) = reservations
            .iter()
            .position(|(line, _, _, _)| *line == drop_line)
        {
            reservations.remove(position);
        }
    }

    reservations.sort_by(|a, b| {
        b.2.partial_cmp(&a.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.3.cmp(&b.3))
    });

    let arranged = match reservations.len() {
        0 | 1 => reservations,
        2 => vec![reservations[1], reservations[0]],
        3 => vec![reservations[1], reservations[2], reservations[0]],
        _ => {
            let mut left = Vec::new();
            let mut center = Vec::new();
            let mut right = Vec::new();
            for (index, reservation) in reservations.into_iter().enumerate() {
                match index % 3 {
                    0 => right.push(reservation),
                    1 => left.push(reservation),
                    _ => center.push(reservation),
                }
            }
            left.reverse();
            left.into_iter()
                .chain(center)
                .chain(right)
                .collect::<Vec<_>>()
        }
    };

    let total_width = arranged
        .iter()
        .map(|(_, _, half, _)| half * 2.0)
        .sum::<f32>()
        + arranged
            .windows(2)
            .map(|pair| route_line_pair_gap(pair[0].1, pair[1].1, segment.zone_signature))
            .sum::<f32>();
    let signature_bias = match segment.zone_signature {
        ZoneSignature::BowlCollector => lane_span * 0.08,
        ZoneSignature::GiantCorkscrew => lane_span * 0.14,
        ZoneSignature::BraidLanes => lane_span * 0.12,
        ZoneSignature::WaveRamps => lane_span * 0.04,
        ZoneSignature::SplitTransfer => lane_span * 0.1,
        ZoneSignature::PillarForest => lane_span * 0.16,
        ZoneSignature::ShapeGarden => lane_span * 0.2,
    };

    let mut cursor = -total_width * 0.5 + signature_bias;
    let mut centers = Vec::with_capacity(arranged.len());
    for (index, (line, kind, half_width, _)) in arranged.iter().copied().enumerate() {
        let center = cursor + half_width;
        centers.push((line, kind, center));
        cursor += half_width * 2.0;
        if let Some((_, next_kind, _, _)) = arranged.get(index + 1) {
            cursor += route_line_pair_gap(kind, *next_kind, segment.zone_signature);
        }
    }

    centers
}

fn route_line_offsets(signature: ZoneSignature, center: f32, zone_local_t: f32) -> (f32, f32) {
    let start_taper = if zone_local_t < 0.22 {
        ROUTE_LINE_EDGE_TAPER
    } else {
        1.0
    };
    let end_taper = if zone_local_t > 0.78 {
        ROUTE_LINE_EDGE_TAPER
    } else {
        1.0
    };
    match signature {
        ZoneSignature::BraidLanes => (center * start_taper * 1.04, center * end_taper * 1.04),
        ZoneSignature::SplitTransfer => (center * start_taper * 0.96, center * end_taper * 1.08),
        ZoneSignature::BowlCollector => (center * start_taper, center * end_taper * 0.9),
        ZoneSignature::GiantCorkscrew => (center * start_taper * 1.02, center * end_taper * 1.02),
        ZoneSignature::WaveRamps => (center * start_taper, center * end_taper),
        ZoneSignature::PillarForest | ZoneSignature::ShapeGarden => {
            (center * start_taper * 1.08, center * end_taper * 1.08)
        }
    }
}

fn route_line_module_kind(
    signature: ZoneSignature,
    line: RouteLine,
    _fallback: ModuleKind,
) -> ModuleKind {
    match signature {
        ZoneSignature::BowlCollector => match line {
            RouteLine::Safe => ModuleKind::SurfRamp,
            RouteLine::Speed => ModuleKind::SurfRamp,
            RouteLine::Trick => ModuleKind::SpeedcheckRun,
        },
        ZoneSignature::GiantCorkscrew => match line {
            RouteLine::Safe => ModuleKind::SurfRamp,
            RouteLine::Speed => ModuleKind::SurfRamp,
            RouteLine::Trick => ModuleKind::WindowHop,
        },
        ZoneSignature::BraidLanes => match line {
            RouteLine::Safe => ModuleKind::StairRun,
            RouteLine::Speed => ModuleKind::SurfRamp,
            RouteLine::Trick => ModuleKind::WindowHop,
        },
        ZoneSignature::WaveRamps => match line {
            RouteLine::Safe => ModuleKind::SpeedcheckRun,
            RouteLine::Speed => ModuleKind::SurfRamp,
            RouteLine::Trick => ModuleKind::WindowHop,
        },
        ZoneSignature::SplitTransfer => match line {
            RouteLine::Safe => ModuleKind::StairRun,
            RouteLine::Speed => ModuleKind::SurfRamp,
            RouteLine::Trick => ModuleKind::WindowHop,
        },
        ZoneSignature::PillarForest => match line {
            RouteLine::Safe => ModuleKind::WindowHop,
            RouteLine::Speed => ModuleKind::SpeedcheckRun,
            RouteLine::Trick => ModuleKind::StairRun,
        },
        ZoneSignature::ShapeGarden => match line {
            RouteLine::Safe => ModuleKind::SpeedcheckRun,
            RouteLine::Speed => ModuleKind::WindowHop,
            RouteLine::Trick => ModuleKind::StairRun,
        },
    }
}

fn supports_parallel_route_choice(kind: ModuleKind) -> bool {
    matches!(
        kind,
        ModuleKind::StairRun | ModuleKind::WindowHop | ModuleKind::SpeedcheckRun
    )
}

fn route_line_flow(
    base: FlowFieldProfile,
    signature: ZoneSignature,
    line: RouteLine,
) -> FlowFieldProfile {
    let mut flow = base;
    match line {
        RouteLine::Safe => {
            flow.width_scale *= 0.86;
            flow.curvature *= 0.88;
            flow.lateral_amplitude *= 0.82;
            flow.vertical_wave *= 0.76;
            flow.noise_amplitude *= 0.74;
        }
        RouteLine::Speed => {
            flow.width_scale *= 0.74;
            flow.noise_amplitude *= 0.68;
        }
        RouteLine::Trick => {
            flow.width_scale *= 0.72;
            flow.curvature *= 1.08;
            flow.lateral_amplitude *= 1.16;
            flow.vertical_wave *= 1.12;
            flow.noise_amplitude *= 0.92;
            flow.dynamic_bias += 0.08;
        }
    }

    flow.lateral_amplitude = flow.lateral_amplitude.clamp(
        0.18,
        match line {
            RouteLine::Safe => 0.48,
            RouteLine::Speed => 0.42,
            RouteLine::Trick => 0.62,
        },
    );
    flow.vertical_wave = flow.vertical_wave.clamp(
        0.02,
        match line {
            RouteLine::Safe => 0.12,
            RouteLine::Speed => 0.1,
            RouteLine::Trick => 0.16,
        },
    );

    match signature {
        ZoneSignature::BowlCollector => {
            flow.path_style = PathLateralStyle::Arc;
            if matches!(line, RouteLine::Trick) {
                flow.path_style = PathLateralStyle::OneSidedArc;
            }
        }
        ZoneSignature::GiantCorkscrew => {
            flow.route_curve = RouteCurveArchetype::Carve;
            flow.path_style = PathLateralStyle::OneSidedArc;
            flow.layered_offset += 0.12;
        }
        ZoneSignature::BraidLanes => {
            flow.route_curve = RouteCurveArchetype::Switchback;
            flow.path_style = if matches!(line, RouteLine::Speed) {
                PathLateralStyle::Arc
            } else {
                PathLateralStyle::OneSidedArc
            };
        }
        ZoneSignature::WaveRamps => {
            flow.path_style = if matches!(line, RouteLine::Speed) {
                PathLateralStyle::OneSidedArc
            } else {
                PathLateralStyle::Arc
            };
            flow.vertical_wave += 0.04;
        }
        ZoneSignature::SplitTransfer => {
            flow.path_style = if matches!(line, RouteLine::Speed) {
                PathLateralStyle::Arc
            } else {
                PathLateralStyle::OneSidedArc
            };
        }
        ZoneSignature::PillarForest | ZoneSignature::ShapeGarden => {
            flow.route_curve = RouteCurveArchetype::Switchback;
            flow.path_style = PathLateralStyle::Arc;
            flow.lateral_amplitude *= 0.86;
        }
    }

    flow
}

fn append_route_line_module(
    layout: &mut ModuleLayout,
    owner: OwnerTag,
    rng: &mut RunRng,
    kind: ModuleKind,
    start: Vec3,
    end: Vec3,
    forward: Vec3,
    right: Vec3,
    difficulty: f32,
    theme: Theme,
    flow: FlowFieldProfile,
    outer_surf_side: Option<f32>,
) {
    match kind {
        ModuleKind::StairRun => append_bhop_platform_sequence(
            layout, owner, rng, start, end, forward, right, theme, flow,
        ),
        ModuleKind::SurfRamp => append_css_surf_sequence(
            layout,
            owner,
            rng,
            start,
            end,
            forward,
            right,
            theme,
            flow,
            true,
            outer_surf_side,
        ),
        ModuleKind::WindowHop => {
            append_window_hop_sequence(layout, owner, rng, start, end, forward, right, theme, flow)
        }
        ModuleKind::PillarAirstrafe => append_pillar_air_strafe_sequence(
            layout, owner, rng, start, end, forward, right, theme, flow,
        ),
        ModuleKind::HeadcheckRun => {
            append_headcheck_sequence(layout, owner, rng, start, end, forward, right, theme, flow)
        }
        ModuleKind::SpeedcheckRun => {
            append_speedcheck_sequence(layout, owner, rng, start, end, forward, right, theme, flow)
        }
        ModuleKind::MovingPlatformRun => append_moving_platform_sequence(
            layout, owner, rng, start, end, forward, right, difficulty, theme, flow,
        ),
        ModuleKind::ShapeGauntlet => append_shape_gauntlet_sequence(
            layout, owner, rng, start, end, forward, right, theme, flow,
        ),
        _ => append_descending_pad_sequence(
            layout,
            owner,
            rng,
            start,
            end,
            forward,
            right,
            PaintStyle::ThemeFloor(theme),
            |_| SolidBody::Static,
            None,
            "Fallback Flow Pad",
            10.0,
            4.4,
            0.55,
            2.1,
            0.95,
        ),
    }
}

fn append_signature_landscape_features(
    layout: &mut ModuleLayout,
    owner: OwnerTag,
    signature: ZoneSignature,
    start: Vec3,
    end: Vec3,
    forward: Vec3,
    right: Vec3,
    theme: Theme,
) {
    let along_x = forward.x.abs() > 0.5;
    let center = start.lerp(end, 0.5);
    let span = start.distance(end).max(16.0);
    match signature {
        ZoneSignature::BowlCollector => {
            layout.solids.push(SolidSpec {
                owner,
                label: "Bowl Collector Shelf".into(),
                center: top_to_center(end - forward * 2.2 + Vec3::Y * 0.2, 0.44),
                size: axis_box_size(along_x, span.min(16.0), 0.44, 14.0),
                paint: PaintStyle::ThemeAccent(theme),
                body: SolidBody::Static,
                friction: None,
                extra: ExtraKind::None,
            });
        }
        ZoneSignature::GiantCorkscrew => {
            layout.solids.push(SolidSpec {
                owner,
                label: "Corkscrew Tower".into(),
                center: center + right * 18.0 + Vec3::Y * 10.0,
                size: Vec3::new(4.2, 24.0, 4.2),
                paint: PaintStyle::ThemeShadow(theme),
                body: SolidBody::Decoration,
                friction: None,
                extra: ExtraKind::None,
            });
        }
        ZoneSignature::BraidLanes => {
            for side in [-1.0_f32, 1.0] {
                layout.solids.push(SolidSpec {
                    owner,
                    label: format!("Braid Rail {side}"),
                    center: center + right * side * 10.5 + Vec3::Y * 0.26,
                    size: axis_box_size(along_x, span * 0.72, 0.22, 0.18),
                    paint: PaintStyle::ThemeAccent(theme),
                    body: SolidBody::Decoration,
                    friction: None,
                    extra: ExtraKind::None,
                });
            }
        }
        ZoneSignature::WaveRamps => {
            for side in [-1.0_f32, 1.0] {
                layout.solids.push(SolidSpec {
                    owner,
                    label: format!("Wave Fin {side}"),
                    center: center + right * side * 12.0 + Vec3::Y * 2.6,
                    size: axis_box_size(along_x, span * 0.48, 5.2, 0.18),
                    paint: PaintStyle::ThemeShadow(theme),
                    body: SolidBody::Decoration,
                    friction: None,
                    extra: ExtraKind::None,
                });
            }
        }
        ZoneSignature::SplitTransfer => {
            layout.solids.push(SolidSpec {
                owner,
                label: "Splitter Spire".into(),
                center: start.lerp(end, 0.28) + Vec3::Y * 3.2,
                size: axis_box_size(along_x, 0.22, 6.0, 7.6),
                paint: PaintStyle::ThemeAccent(theme),
                body: SolidBody::Decoration,
                friction: None,
                extra: ExtraKind::None,
            });
            layout.solids.push(SolidSpec {
                owner,
                label: "Collector Spire".into(),
                center: start.lerp(end, 0.78) + Vec3::Y * 2.8,
                size: axis_box_size(along_x, 0.2, 5.2, 6.2),
                paint: PaintStyle::ThemeFloor(theme),
                body: SolidBody::Decoration,
                friction: None,
                extra: ExtraKind::None,
            });
        }
        ZoneSignature::PillarForest => {
            for side in [-1.0_f32, 1.0] {
                layout.solids.push(SolidSpec {
                    owner,
                    label: format!("Forest Monolith {side}"),
                    center: center + right * side * 16.0 + Vec3::Y * 8.0,
                    size: Vec3::new(2.2, 18.0, 2.2),
                    paint: PaintStyle::ThemeShadow(theme),
                    body: SolidBody::Decoration,
                    friction: None,
                    extra: ExtraKind::None,
                });
            }
        }
        ZoneSignature::ShapeGarden => {
            for side in [-1.0_f32, 1.0] {
                layout.solids.push(SolidSpec {
                    owner,
                    label: format!("Shape Garden Anchor {side}"),
                    center: center + right * side * 18.0 + Vec3::Y * 4.8,
                    size: Vec3::new(8.0, 8.0, 8.0),
                    paint: PaintStyle::ThemeShadow(theme),
                    body: SolidBody::Decoration,
                    friction: None,
                    extra: ExtraKind::None,
                });
            }
        }
    }
}

fn append_signature_route_choice(
    layout: &mut ModuleLayout,
    owner: OwnerTag,
    segment: &SegmentPlan,
    from: &RoomPlan,
    to: &RoomPlan,
    start: Vec3,
    end: Vec3,
    forward: Vec3,
    right: Vec3,
    theme: Theme,
) -> bool {
    if segment.route_lines.len() <= 1
        || !is_primary_speed_section(segment.kind)
        || !supports_parallel_route_choice(segment.kind)
        || !ENABLE_PARALLEL_ROUTE_GEOMETRY
    {
        return false;
    }

    let line_specs = route_line_center_offsets(segment, from, to);
    if line_specs.len() > 1
        && (!line_specs
            .iter()
            .all(|(_, kind, _)| supports_parallel_route_choice(*kind))
            || line_specs
                .iter()
                .any(|(_, kind, _)| matches!(kind, ModuleKind::SurfRamp)))
    {
        return false;
    }

    append_signature_landscape_features(
        layout,
        owner,
        segment.zone_signature,
        start,
        end,
        forward,
        right,
        theme,
    );

    for (line_index, (line, line_kind, lane_center)) in line_specs.into_iter().enumerate() {
        let line_distance = start.distance(end).max(12.0);
        let (entry_margin, exit_margin) =
            route_line_forward_margins(line_kind, line, line_distance);
        let (start_offset, end_offset) =
            route_line_offsets(segment.zone_signature, lane_center, segment.zone_local_t);
        let vertical_bias = route_line_vertical_bias(line_kind, line);
        let line_start =
            start + forward * entry_margin + right * start_offset + Vec3::Y * vertical_bias;
        let line_end =
            end - forward * exit_margin + right * end_offset + Vec3::Y * vertical_bias * 0.72;
        let line_flow = route_line_flow(segment.flow, segment.zone_signature, line);
        let mut line_rng = RunRng::new(
            segment.seed ^ ((line_index as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15)),
        );
        let solid_start = layout.solids.len();
        append_route_line_module(
            layout,
            owner,
            &mut line_rng,
            line_kind,
            line_start,
            line_end,
            forward,
            right,
            segment.difficulty,
            theme,
            line_flow,
            if matches!(line_kind, ModuleKind::SurfRamp) && lane_center.abs() > 0.01 {
                Some(lane_center.signum())
            } else {
                None
            },
        );
        let line_prefix = route_line_label_prefix(line);
        for solid in &mut layout.solids[solid_start..] {
            solid.label = format!("{line_prefix} {}", solid.label);
        }
    }

    true
}

fn build_segment_layout(segment: &SegmentPlan, rooms: &[RoomPlan]) -> ModuleLayout {
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
    append_connector_geometry(
        &mut layout,
        owner,
        &mut rng,
        segment.connector,
        start,
        end,
        forward,
        right,
        to.theme,
        segment.difficulty,
        segment.flow,
    );

    let used_route_choice = append_signature_route_choice(
        &mut layout,
        owner,
        segment,
        from,
        to,
        start,
        end,
        forward,
        right,
        to.theme,
    );

    if !used_route_choice {
        match segment.kind {
            ModuleKind::StairRun => {
                append_bhop_platform_sequence(
                    &mut layout,
                    owner,
                    &mut rng,
                    start,
                    end,
                    forward,
                    right,
                    to.theme,
                    segment.flow,
                );
            }
            ModuleKind::SurfRamp => {
                append_css_surf_sequence(
                    &mut layout,
                    owner,
                    &mut rng,
                    start,
                    end,
                    forward,
                    right,
                    to.theme,
                    segment.flow,
                    true,
                    None,
                );
            }
            ModuleKind::WindowHop => {
                append_window_hop_sequence(
                    &mut layout,
                    owner,
                    &mut rng,
                    start,
                    end,
                    forward,
                    right,
                    to.theme,
                    segment.flow,
                );
            }
            ModuleKind::PillarAirstrafe => {
                append_pillar_air_strafe_sequence(
                    &mut layout,
                    owner,
                    &mut rng,
                    start,
                    end,
                    forward,
                    right,
                    to.theme,
                    segment.flow,
                );
            }
            ModuleKind::HeadcheckRun => {
                append_headcheck_sequence(
                    &mut layout,
                    owner,
                    &mut rng,
                    start,
                    end,
                    forward,
                    right,
                    to.theme,
                    segment.flow,
                );
            }
            ModuleKind::SpeedcheckRun => {
                append_speedcheck_sequence(
                    &mut layout,
                    owner,
                    &mut rng,
                    start,
                    end,
                    forward,
                    right,
                    to.theme,
                    segment.flow,
                );
            }
            ModuleKind::MovingPlatformRun => {
                append_moving_platform_sequence(
                    &mut layout,
                    owner,
                    &mut rng,
                    start,
                    end,
                    forward,
                    right,
                    segment.difficulty,
                    to.theme,
                    segment.flow,
                );
            }
            ModuleKind::ShapeGauntlet => {
                append_shape_gauntlet_sequence(
                    &mut layout,
                    owner,
                    &mut rng,
                    start,
                    end,
                    forward,
                    right,
                    to.theme,
                    segment.flow,
                );
            }
            ModuleKind::MantleStack => {
                append_descending_pad_sequence(
                    &mut layout,
                    owner,
                    &mut rng,
                    start,
                    end,
                    forward,
                    right,
                    PaintStyle::ThemeAccent(to.theme),
                    |_| SolidBody::Static,
                    None,
                    "Mantle Ledge",
                    8.8,
                    4.6,
                    0.92,
                    3.4,
                    1.35,
                );

                let wall_height = (from.top.y - to.top.y).abs() + 6.8;
                let wall_mid = start.lerp(end, 0.62) + right * 2.8;
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
                let shaft_center = start.lerp(end, 0.5);
                let shaft_floor_top = Vec3::new(shaft_center.x, start.y - 0.55, shaft_center.z);
                let shaft_height = (from.top.y - to.top.y).abs() + 7.2;
                let wall_length = (end - start).xz().length().max(14.0);
                let wall_thickness = 0.72;
                let gap_half = 1.08;

                layout.solids.push(SolidSpec {
                    owner,
                    label: "Shaft Entry".into(),
                    center: top_to_center(start.lerp(end, 0.24) + Vec3::Y * 0.24, 0.5),
                    size: axis_box_size(along_x, 5.2, 0.5, 3.4),
                    paint: PaintStyle::ThemeAccent(to.theme),
                    body: SolidBody::Static,
                    friction: None,
                    extra: ExtraKind::None,
                });

                for side in [-1.0, 1.0] {
                    layout.solids.push(SolidSpec {
                        owner,
                        label: "Shaft Wall".into(),
                        center: Vec3::new(
                            shaft_center.x + right.x * side * (gap_half + wall_thickness * 0.5),
                            shaft_floor_top.y + shaft_height * 0.5 - 0.2,
                            shaft_center.z + right.z * side * (gap_half + wall_thickness * 0.5),
                        ),
                        size: axis_box_size(along_x, wall_length, shaft_height, wall_thickness),
                        paint: PaintStyle::ThemeShadow(to.theme),
                        body: SolidBody::Static,
                        friction: None,
                        extra: ExtraKind::None,
                    });
                }

                layout.solids.push(SolidSpec {
                    owner,
                    label: "Shaft Floor".into(),
                    center: top_to_center(shaft_floor_top, 0.46),
                    size: axis_box_size(along_x, wall_length - 0.2, 0.46, gap_half * 1.7),
                    paint: PaintStyle::ThemeFloor(to.theme),
                    body: SolidBody::Static,
                    friction: None,
                    extra: ExtraKind::None,
                });
                layout.solids.push(SolidSpec {
                    owner,
                    label: "Shaft Exit".into(),
                    center: top_to_center(end + Vec3::Y * 0.22, 0.48),
                    size: axis_box_size(along_x, 5.0, 0.48, 3.4),
                    paint: PaintStyle::ThemeFloor(to.theme),
                    body: SolidBody::Static,
                    friction: None,
                    extra: ExtraKind::None,
                });
                append_descending_pad_sequence(
                    &mut layout,
                    owner,
                    &mut rng,
                    start.lerp(end, 0.12),
                    end.lerp(start, 0.12),
                    forward,
                    right,
                    PaintStyle::ThemeFloor(to.theme),
                    |_| SolidBody::Static,
                    None,
                    "Shaft Floor Pad",
                    10.0,
                    3.8,
                    0.42,
                    gap_half * 1.55,
                    0.28,
                );
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
                    let support_height = lerp(5.6, 8.4, segment.difficulty);
                    layout.solids.push(SolidSpec {
                        owner,
                        label: "Lift Support".into(),
                        center: Vec3::new(
                            anchor.x - right.x * 3.2,
                            support_top - support_height * 0.5,
                            anchor.z - right.z * 3.2,
                        ),
                        size: Vec3::new(1.4, support_height, 1.4),
                        paint: PaintStyle::ThemeShadow(to.theme),
                        body: SolidBody::Static,
                        friction: None,
                        extra: ExtraKind::None,
                    });
                }
            }
            ModuleKind::CrumbleBridge => {
                let delay = lerp(0.9, 0.45, segment.difficulty);
                let sink_speed = lerp(2.8, 5.0, segment.difficulty);
                append_descending_pad_sequence(
                    &mut layout,
                    owner,
                    &mut rng,
                    start,
                    end,
                    forward,
                    right,
                    PaintStyle::Hazard,
                    |_| SolidBody::Crumbling { delay, sink_speed },
                    None,
                    "Crumble Span",
                    8.0,
                    4.4,
                    0.55,
                    2.3,
                    0.95,
                );
            }
            ModuleKind::WindTunnel => {
                append_descending_pad_sequence(
                    &mut layout,
                    owner,
                    &mut rng,
                    start,
                    end,
                    forward,
                    right,
                    PaintStyle::ThemeFloor(to.theme),
                    |_| SolidBody::Static,
                    None,
                    "Wind Perch",
                    8.8,
                    4.2,
                    0.58,
                    1.95,
                    1.15,
                );
                layout.features.push(FeatureSpec::WindZone {
                    center: start.lerp(end, 0.52) + Vec3::Y * (rise * 0.55 + 1.5),
                    size: axis_box_size(along_x, (end - start).xz().length() + 6.0, 3.8, 7.4),
                    direction: right * if rng.chance(0.5) { 1.0 } else { -1.0 } + Vec3::Y * 0.1,
                    strength: lerp(6.0, 11.0, segment.difficulty),
                    gust: lerp(1.2, 2.8, segment.difficulty),
                });
            }
            ModuleKind::IceSpine => {
                append_descending_pad_sequence(
                    &mut layout,
                    owner,
                    &mut rng,
                    start,
                    end,
                    forward,
                    right,
                    PaintStyle::Ice,
                    |_| SolidBody::Static,
                    Some(0.02),
                    "Ice Spine",
                    7.6,
                    4.6,
                    0.58,
                    1.85,
                    0.88,
                );
            }
            ModuleKind::WaterGarden => {
                let basin_top = from.top.y.min(to.top.y) - 2.3;
                let basin_mid = start.lerp(end, 0.5);
                let basin_size =
                    axis_box_size(along_x, (end - start).xz().length() + 6.5, 0.78, 9.2);
                let water_size = Vec3::new(
                    (basin_size.x - 0.7).max(2.5),
                    2.1,
                    (basin_size.z - 0.7).max(2.5),
                );
                layout.solids.push(SolidSpec {
                    owner,
                    label: "Water Basin".into(),
                    center: top_to_center(
                        Vec3::new(basin_mid.x, basin_top, basin_mid.z),
                        basin_size.y,
                    ),
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
        }
    }

    layout.clearances.push(ClearanceProbe {
        owner,
        center: start.lerp(end, 0.5) + Vec3::Y * (rise * 0.5 + 2.5),
        size: axis_box_size(along_x, (end - start).xz().length() * 0.6, 2.8, 3.4),
    });
    let _ = template;
    layout
}

fn append_css_surf_sequence(
    layout: &mut ModuleLayout,
    owner: OwnerTag,
    rng: &mut RunRng,
    start: Vec3,
    end: Vec3,
    forward: Vec3,
    right: Vec3,
    theme: Theme,
    flow: FlowFieldProfile,
    intense: bool,
    outer_only_side: Option<f32>,
) {
    let direct_distance = start.distance(end).max(12.0);
    let entry_margin = if intense {
        (direct_distance * 0.025).clamp(2.2, 6.0)
    } else {
        (direct_distance * 0.02).clamp(1.8, 4.8)
    };
    let exit_margin = if intense {
        (direct_distance * 0.02).clamp(1.8, 4.8)
    } else {
        (direct_distance * 0.016).clamp(1.4, 4.0)
    };
    let mut surf_start = start + forward * entry_margin + Vec3::Y * 0.4;
    let mut surf_end = end - forward * exit_margin + Vec3::Y * 0.18;
    let total_distance = surf_start.distance(surf_end).max(12.0);
    let mut surf_flow = flow;
    surf_flow.path_style = surf_path_style_from_flow(flow);
    surf_flow.curvature = (flow.curvature * if intense { 1.08 } else { 0.98 }).clamp(0.82, 1.36);
    surf_flow.weave_cycles = match surf_flow.path_style {
        PathLateralStyle::Straight => flow.weave_cycles.clamp(0.25, 0.8),
        PathLateralStyle::Arc => flow.weave_cycles.clamp(0.45, 1.15),
        PathLateralStyle::OneSidedArc => flow.weave_cycles.clamp(0.08, 0.4),
        PathLateralStyle::Serpentine | PathLateralStyle::Switchback => unreachable!(),
    };
    surf_flow.lateral_amplitude =
        (flow.lateral_amplitude * if intense { 1.22 } else { 1.08 }).clamp(0.32, 2.8);
    surf_flow.vertical_wave =
        (flow.vertical_wave * if intense { 1.24 } else { 1.06 }).clamp(0.08, 0.48);
    let flow_seed = rng.next_u64();
    let ramp_span = if intense {
        rng.range_f32(9.6, 13.8) * surf_flow.width_scale.clamp(0.9, 1.26)
    } else {
        rng.range_f32(7.6, 10.6) * surf_flow.width_scale.clamp(0.92, 1.22)
    };
    let ramp_drop = if intense {
        rng.range_f32(16.0, 24.0) * surf_flow.curvature.clamp(0.92, 1.22)
    } else {
        rng.range_f32(12.0, 18.0) * surf_flow.curvature.clamp(0.92, 1.18)
    };
    let ridge_lift = if intense {
        rng.range_f32(3.8, 5.8) + surf_flow.vertical_wave * 6.0
    } else {
        rng.range_f32(2.6, 4.2) + surf_flow.vertical_wave * 4.2
    };
    if let Some(side) = outer_only_side {
        let outward_shift = right * side.signum() * ramp_span * 0.58;
        surf_start += outward_shift;
        surf_end += outward_shift;
    }

    let centerline_point = |t: f32| {
        let envelope = (t * PI).sin().max(0.0).powf(0.85);
        let sample = sample_flow_field(surf_flow, flow_seed, t, envelope);
        let offset = right * sample.lateral;
        let lift = Vec3::Y * (ridge_lift * envelope + sample.vertical * ramp_drop * 0.14);
        surf_start.lerp(surf_end, t) + offset + lift
    };
    let base_segment_count = if intense {
        ((total_distance / 4.6).ceil() as usize).clamp(44, 132)
    } else {
        ((total_distance / 5.2).ceil() as usize).clamp(36, 116)
    };
    let mut segment_count = base_segment_count;
    let mut centerline = Vec::new();
    let fallback_tangent = (surf_end - surf_start).normalize_or_zero();
    let fallback_tangent = if fallback_tangent == Vec3::ZERO {
        forward
    } else {
        fallback_tangent
    };

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
            if let Some(outer_side) = outer_only_side
                && side.signum() != outer_side.signum()
            {
                continue;
            }
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
                owner,
                label: if intense {
                    format!("Surf Wedge Render {} {}", index, side)
                } else {
                    format!("Flow Wedge Render {} {}", index, side)
                },
                center: wedge.center,
                size: wedge.bounds,
                paint: if side < 0.0 {
                    PaintStyle::ThemeAccent(theme)
                } else {
                    PaintStyle::ThemeFloor(theme)
                },
                body: SolidBody::StaticSurfWedge {
                    wall_side: side,
                    render_points: wedge.render_points,
                },
                friction: Some(0.0),
                extra: ExtraKind::None,
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
        if let Some(outer_side) = outer_only_side
            && side.signum() != outer_side.signum()
        {
            continue;
        }
        let strip = surf_strip_from_path(
            &collider_centerline,
            &collider_tangents,
            &collider_outwards,
            side,
            ramp_span,
            ramp_drop,
        );
        layout.solids.push(SolidSpec {
            owner,
            label: if intense {
                format!("Surf Strip Collider {}", side)
            } else {
                format!("Flow Strip Collider {}", side)
            },
            center: strip.center,
            size: strip.bounds,
            paint: if side < 0.0 {
                PaintStyle::ThemeAccent(theme)
            } else {
                PaintStyle::ThemeFloor(theme)
            },
            body: SolidBody::StaticSurfStrip {
                wall_side: side,
                collider_strip_points: strip.collider_strip_points,
                columns: SURF_COLLIDER_COLUMNS,
            },
            friction: Some(0.0),
            extra: ExtraKind::None,
        });
    }
}

fn append_bhop_platform_sequence(
    layout: &mut ModuleLayout,
    owner: OwnerTag,
    rng: &mut RunRng,
    start: Vec3,
    end: Vec3,
    forward: Vec3,
    right: Vec3,
    theme: Theme,
    flow: FlowFieldProfile,
) {
    let along_x = forward.x.abs() > 0.5;
    let distance = start.distance(end).max(18.0);
    let start_margin = bhop_path_margin(distance, 0.15, 0.85);
    let end_margin = bhop_path_margin(distance, 0.12, 0.75);
    let path_start = start + forward * start_margin + Vec3::Y * 0.24;
    let path_end = end - forward * end_margin + Vec3::Y * 0.18;
    let requested_count =
        ((distance / scaled_bhop_cadence(4.8, 6.6, rng)).round() as usize).clamp(6, 16);
    let pad_count =
        clamp_platform_count_for_spacing(distance, requested_count, scaled_bhop_size(4.0), 4);
    let style = choose_bhop_path_style(rng, ModuleKind::StairRun);
    let weave_cycles = rng.range_f32(1.6, 3.2);
    let phase = rng.range_f32(0.0, TAU);
    let lateral_amplitude = scaled_bhop_size(rng.range_f32(1.0, 1.8));
    let vertical_wave = scaled_bhop_size(rng.range_f32(0.06, 0.22));
    let mut section_flow = flow;
    section_flow.lateral_amplitude = section_flow.lateral_amplitude.clamp(0.48, 1.45);
    section_flow.vertical_wave = section_flow.vertical_wave.clamp(0.05, 0.2);
    let flow_seed = rng.next_u64();
    let tops = sample_descending_platform_tops(
        path_start,
        path_end,
        right,
        pad_count,
        style,
        weave_cycles,
        phase,
        lateral_amplitude,
        vertical_wave,
        Some((section_flow, flow_seed)),
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
        let paint = if catch_platform || step % 4 == 0 {
            PaintStyle::ThemeAccent(theme)
        } else {
            PaintStyle::ThemeFloor(theme)
        };

        layout.solids.push(SolidSpec {
            owner,
            label: if catch_platform {
                format!("Bhop Catch Pad {step}")
            } else {
                format!("Bhop Pad {step}")
            },
            center: top_to_center(top, pad_height),
            size: axis_box_size(along_x, square_side, pad_height, square_side),
            paint,
            body: SolidBody::Static,
            friction: None,
            extra: ExtraKind::None,
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
    flow: Option<(FlowFieldProfile, u64)>,
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
        let (lateral_offset, vertical_offset) = if let Some((profile, seed)) = flow {
            let sample = sample_flow_field(profile, seed, t, envelope * endpoint_factor);
            (
                sample.lateral * lateral_amplitude.max(1.0),
                sample.vertical * vertical_wave.max(0.01) * 3.2,
            )
        } else {
            (
                path_lateral_offset(
                    style,
                    t,
                    envelope * endpoint_factor,
                    phase,
                    weave_cycles,
                    lateral_amplitude,
                ),
                (t * TAU * 1.35 + phase * 0.7).sin() * vertical_wave * envelope * endpoint_factor,
            )
        };
        points.push(start.lerp(end, t) + right * lateral_offset + Vec3::Y * vertical_offset);
    }
    points
}

fn scaled_bhop_size(value: f32) -> f32 {
    value * BHOP_OBJECT_SCALE
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

fn choose_bhop_path_style(rng: &mut RunRng, kind: ModuleKind) -> PathLateralStyle {
    let straight_weight = if matches!(kind, ModuleKind::SpeedcheckRun) {
        4
    } else {
        1
    };
    let arc_weight = if matches!(
        kind,
        ModuleKind::MovingPlatformRun | ModuleKind::PillarAirstrafe
    ) {
        6
    } else {
        4
    };
    let switch_weight = if matches!(
        kind,
        ModuleKind::StairRun | ModuleKind::WindowHop | ModuleKind::ShapeGauntlet
    ) {
        8
    } else {
        3
    };
    rng.weighted_choice(&[
        (PathLateralStyle::Straight, straight_weight),
        (PathLateralStyle::Serpentine, 6),
        (PathLateralStyle::Switchback, switch_weight),
        (PathLateralStyle::Arc, arc_weight),
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

fn scaled_bhop_cadence(min: f32, max: f32, rng: &mut RunRng) -> f32 {
    rng.range_f32(min * BHOP_CADENCE_SCALE, max * BHOP_CADENCE_SCALE)
}

fn append_window_hop_sequence(
    layout: &mut ModuleLayout,
    owner: OwnerTag,
    rng: &mut RunRng,
    start: Vec3,
    end: Vec3,
    forward: Vec3,
    right: Vec3,
    theme: Theme,
    flow: FlowFieldProfile,
) {
    let along_x = forward.x.abs() > 0.5;
    let distance = start.distance(end).max(18.0);
    let path_start = start + forward * bhop_path_margin(distance, 0.15, 0.75) + Vec3::Y * 0.22;
    let path_end = end - forward * bhop_path_margin(distance, 0.12, 0.7) + Vec3::Y * 0.18;
    let requested_count =
        ((distance / scaled_bhop_cadence(5.8, 7.8, rng)).round() as usize).clamp(5, 12);
    let pad_count =
        clamp_platform_count_for_spacing(distance, requested_count, scaled_bhop_size(4.2), 4);
    let mut section_flow = flow;
    section_flow.lateral_amplitude = (section_flow.lateral_amplitude * 0.95).clamp(0.42, 1.25);
    section_flow.vertical_wave = section_flow.vertical_wave.clamp(0.04, 0.16);
    let flow_seed = rng.next_u64();
    let tops = sample_descending_platform_tops(
        path_start,
        path_end,
        right,
        pad_count,
        choose_bhop_path_style(rng, ModuleKind::WindowHop),
        rng.range_f32(1.4, 2.6),
        rng.range_f32(0.0, TAU),
        scaled_bhop_size(rng.range_f32(0.8, 1.4)),
        scaled_bhop_size(rng.range_f32(0.05, 0.16)),
        Some((section_flow, flow_seed)),
    );

    for (step, top) in tops.iter().enumerate() {
        let catch_platform = step == 0 || step + 1 == tops.len() || step % 3 == 0;
        let square_side = scaled_bhop_size(if catch_platform {
            rng.range_f32(3.8, 5.2)
        } else {
            rng.range_f32(2.3, 3.2)
        });
        let pad_height = scaled_bhop_size(if catch_platform {
            rng.range_f32(0.82, 1.05)
        } else {
            rng.range_f32(0.64, 0.86)
        }) * 0.45;
        layout.solids.push(SolidSpec {
            owner,
            label: format!("Window Hop Pad {step}"),
            center: top_to_center(*top, pad_height),
            size: axis_box_size(along_x, square_side, pad_height, square_side),
            paint: if catch_platform {
                PaintStyle::ThemeAccent(theme)
            } else {
                PaintStyle::ThemeFloor(theme)
            },
            body: SolidBody::Static,
            friction: None,
            extra: ExtraKind::None,
        });

        if step + 1 >= tops.len() {
            continue;
        }

        let next_top = tops[step + 1];
        let frame_center =
            top.lerp(next_top, 0.5) + Vec3::Y * scaled_bhop_size(rng.range_f32(0.34, 0.44));
        let frame_depth = scaled_bhop_size(rng.range_f32(0.12, 0.18));
        let frame_height = scaled_bhop_size(rng.range_f32(0.76, 0.96));
        let frame_width = scaled_bhop_size(rng.range_f32(0.72, 0.94));
        let thickness = scaled_bhop_size(rng.range_f32(0.05, 0.08));
        let side_offset = frame_width * 0.5 + thickness * 0.5;
        let base_paint = if step % 2 == 0 {
            PaintStyle::ThemeAccent(theme)
        } else {
            PaintStyle::ThemeShadow(theme)
        };

        for side in [-1.0, 1.0] {
            layout.solids.push(SolidSpec {
                owner,
                label: format!("Window Frame Post {step} {side}"),
                center: Vec3::new(
                    frame_center.x + right.x * side * side_offset,
                    frame_center.y,
                    frame_center.z + right.z * side * side_offset,
                ),
                size: axis_box_size(along_x, frame_depth, frame_height, thickness),
                paint: base_paint,
                body: SolidBody::Static,
                friction: None,
                extra: ExtraKind::None,
            });
        }
        layout.solids.push(SolidSpec {
            owner,
            label: format!("Window Frame Lintel {step}"),
            center: frame_center + Vec3::Y * (frame_height * 0.5 - thickness * 0.5),
            size: axis_box_size(
                along_x,
                frame_depth,
                thickness,
                frame_width + thickness * 2.0,
            ),
            paint: base_paint,
            body: SolidBody::Static,
            friction: None,
            extra: ExtraKind::None,
        });
    }
}

fn append_pillar_air_strafe_sequence(
    layout: &mut ModuleLayout,
    owner: OwnerTag,
    rng: &mut RunRng,
    start: Vec3,
    end: Vec3,
    _forward: Vec3,
    right: Vec3,
    theme: Theme,
    flow: FlowFieldProfile,
) {
    let distance = start.distance(end).max(18.0);
    let path_start = start + Vec3::Y * 0.26;
    let path_end = end + Vec3::Y * 0.14;
    let pillar_count = clamp_platform_count_for_spacing(
        distance,
        ((distance / scaled_bhop_cadence(5.4, 7.4, rng)).round() as usize).clamp(5, 12),
        scaled_bhop_size(4.6),
        4,
    );
    let mut section_flow = flow;
    section_flow.lateral_amplitude = (section_flow.lateral_amplitude * 1.22).clamp(0.64, 1.8);
    section_flow.vertical_wave = section_flow.vertical_wave.clamp(0.04, 0.14);
    let flow_seed = rng.next_u64();
    let tops = sample_descending_platform_tops(
        path_start,
        path_end,
        right,
        pillar_count,
        choose_bhop_path_style(rng, ModuleKind::PillarAirstrafe),
        rng.range_f32(1.1, 2.3),
        rng.range_f32(0.0, TAU),
        scaled_bhop_size(rng.range_f32(0.9, 1.8)),
        scaled_bhop_size(rng.range_f32(0.05, 0.16)),
        Some((section_flow, flow_seed)),
    );
    let hanging_base = end.y - scaled_bhop_size(rng.range_f32(1.4, 2.2));

    for (step, top) in tops.iter().enumerate() {
        let catch_pillar = step == 0 || step + 1 == tops.len() || step % 4 == 0;
        let radius = scaled_bhop_size(if catch_pillar {
            rng.range_f32(1.7, 2.4)
        } else {
            rng.range_f32(1.1, 1.7)
        });
        let cap_height = scaled_bhop_size(if catch_pillar {
            rng.range_f32(5.0, 7.4)
        } else {
            rng.range_f32(3.6, 6.0)
        }) * 0.5;
        let base_y = hanging_base.min(top.y - cap_height + scaled_bhop_size(0.2));
        let center = Vec3::new(top.x, base_y + cap_height * 0.5, top.z);
        layout.solids.push(SolidSpec {
            owner,
            label: format!("Airstrafe Pillar {step}"),
            center,
            size: Vec3::new(radius * 2.0, cap_height, radius * 2.0),
            paint: if catch_pillar {
                PaintStyle::ThemeAccent(theme)
            } else {
                PaintStyle::ThemeFloor(theme)
            },
            body: SolidBody::StaticCylinder,
            friction: None,
            extra: ExtraKind::None,
        });
    }
}

fn append_headcheck_sequence(
    layout: &mut ModuleLayout,
    owner: OwnerTag,
    rng: &mut RunRng,
    start: Vec3,
    end: Vec3,
    forward: Vec3,
    right: Vec3,
    theme: Theme,
    flow: FlowFieldProfile,
) {
    let along_x = forward.x.abs() > 0.5;
    let distance = start.distance(end).max(18.0);
    let path_start = start + forward * bhop_path_margin(distance, 0.2, 0.9) + Vec3::Y * 0.24;
    let path_end = end - forward * bhop_path_margin(distance, 0.18, 0.82) + Vec3::Y * 0.18;
    let pad_count = clamp_platform_count_for_spacing(
        distance,
        ((distance / scaled_bhop_cadence(5.0, 6.8, rng)).round() as usize).clamp(5, 12),
        scaled_bhop_size(4.4),
        4,
    );
    let mut section_flow = flow;
    section_flow.lateral_amplitude = (section_flow.lateral_amplitude * 0.84).clamp(0.34, 1.05);
    section_flow.vertical_wave = section_flow.vertical_wave.clamp(0.03, 0.12);
    let flow_seed = rng.next_u64();
    let tops = sample_descending_platform_tops(
        path_start,
        path_end,
        right,
        pad_count,
        choose_bhop_path_style(rng, ModuleKind::HeadcheckRun),
        rng.range_f32(1.5, 2.8),
        rng.range_f32(0.0, TAU),
        scaled_bhop_size(rng.range_f32(0.7, 1.2)),
        scaled_bhop_size(rng.range_f32(0.03, 0.11)),
        Some((section_flow, flow_seed)),
    );

    for (step, top) in tops.iter().enumerate() {
        let catch_platform = step == 0 || step + 1 == tops.len() || step % 4 == 0;
        let pad_length = scaled_bhop_size(if catch_platform {
            rng.range_f32(4.2, 5.2)
        } else {
            rng.range_f32(2.8, 3.8)
        });
        let pad_height = scaled_bhop_size(if catch_platform { 0.92 } else { 0.72 }) * 0.45;
        layout.solids.push(SolidSpec {
            owner,
            label: format!("Headcheck Pad {step}"),
            center: top_to_center(*top, pad_height),
            size: axis_box_size(along_x, pad_length, pad_height, pad_length),
            paint: if catch_platform {
                PaintStyle::ThemeAccent(theme)
            } else {
                PaintStyle::ThemeFloor(theme)
            },
            body: SolidBody::Static,
            friction: None,
            extra: ExtraKind::None,
        });

        if step + 1 >= tops.len() || step % 2 == 0 {
            continue;
        }

        let blocker_center =
            top.lerp(tops[step + 1], 0.5) + Vec3::Y * scaled_bhop_size(rng.range_f32(0.34, 0.4));
        let blocker_height = scaled_bhop_size(rng.range_f32(0.08, 0.11));
        layout.solids.push(SolidSpec {
            owner,
            label: format!("Headcheck Blocker {step}"),
            center: blocker_center,
            size: axis_box_size(
                along_x,
                scaled_bhop_size(rng.range_f32(0.92, 1.32)),
                blocker_height,
                scaled_bhop_size(0.78),
            ),
            paint: PaintStyle::ThemeShadow(theme),
            body: SolidBody::Static,
            friction: None,
            extra: ExtraKind::None,
        });
    }
}

fn append_speedcheck_sequence(
    layout: &mut ModuleLayout,
    owner: OwnerTag,
    rng: &mut RunRng,
    start: Vec3,
    end: Vec3,
    forward: Vec3,
    right: Vec3,
    theme: Theme,
    flow: FlowFieldProfile,
) {
    let along_x = forward.x.abs() > 0.5;
    let distance = start.distance(end).max(18.0);
    let path_start = start + forward * bhop_path_margin(distance, 0.16, 0.8) + Vec3::Y * 0.2;
    let path_end = end - forward * bhop_path_margin(distance, 0.14, 0.74) + Vec3::Y * 0.12;
    let longjump = rng.chance(0.55);
    let requested_count = if longjump {
        ((distance / scaled_bhop_cadence(7.2, 9.8, rng)).round() as usize).clamp(4, 8)
    } else {
        ((distance / scaled_bhop_cadence(6.0, 8.0, rng)).round() as usize).clamp(4, 7)
    };
    let pad_count = clamp_platform_count_for_spacing(
        distance,
        requested_count,
        if longjump {
            scaled_bhop_size(5.8)
        } else {
            scaled_bhop_size(4.6)
        },
        4,
    );
    let mut section_flow = flow;
    if matches!(
        section_flow.path_style,
        PathLateralStyle::Serpentine | PathLateralStyle::Switchback
    ) {
        section_flow.path_style = PathLateralStyle::Arc;
    }
    section_flow.lateral_amplitude =
        (section_flow.lateral_amplitude * if longjump { 0.58 } else { 0.76 }).clamp(0.18, 0.92);
    section_flow.vertical_wave = section_flow.vertical_wave.clamp(0.02, 0.08);
    let flow_seed = rng.next_u64();
    let tops = sample_descending_platform_tops(
        path_start,
        path_end,
        right,
        pad_count,
        choose_bhop_path_style(rng, ModuleKind::SpeedcheckRun),
        rng.range_f32(1.0, 2.2),
        rng.range_f32(0.0, TAU),
        if longjump {
            scaled_bhop_size(rng.range_f32(0.22, 0.44))
        } else {
            scaled_bhop_size(rng.range_f32(0.34, 0.56))
        },
        scaled_bhop_size(rng.range_f32(0.02, 0.07)),
        Some((section_flow, flow_seed)),
    );

    for (step, mut top) in tops.into_iter().enumerate() {
        if !longjump && step > 0 && step < pad_count.saturating_sub(1) && step % 2 == 1 {
            top.y += scaled_bhop_size(rng.range_f32(0.36, 0.68));
        }
        let catch_platform = step == 0 || step + 1 == pad_count || step % 3 == 0;
        let pad_length = scaled_bhop_size(if longjump {
            if catch_platform {
                rng.range_f32(4.2, 5.6)
            } else {
                rng.range_f32(2.8, 3.8)
            }
        } else if catch_platform {
            rng.range_f32(3.8, 5.2)
        } else {
            rng.range_f32(2.4, 3.2)
        });
        let pad_height = scaled_bhop_size(if catch_platform { 0.92 } else { 0.74 }) * 0.45;
        layout.solids.push(SolidSpec {
            owner,
            label: if longjump {
                format!("Longjump Pad {step}")
            } else {
                format!("Highjump Pad {step}")
            },
            center: top_to_center(top, pad_height),
            size: axis_box_size(along_x, pad_length, pad_height, pad_length),
            paint: if longjump {
                PaintStyle::ThemeFloor(theme)
            } else {
                PaintStyle::ThemeAccent(theme)
            },
            body: SolidBody::Static,
            friction: None,
            extra: ExtraKind::None,
        });
    }
}

fn append_moving_platform_sequence(
    layout: &mut ModuleLayout,
    owner: OwnerTag,
    rng: &mut RunRng,
    start: Vec3,
    end: Vec3,
    forward: Vec3,
    right: Vec3,
    difficulty: f32,
    theme: Theme,
    flow: FlowFieldProfile,
) {
    let along_x = forward.x.abs() > 0.5;
    let distance = start.distance(end).max(18.0);
    let path_start = start + forward * bhop_path_margin(distance, 0.18, 0.82) + Vec3::Y * 0.24;
    let path_end = end - forward * bhop_path_margin(distance, 0.18, 0.82) + Vec3::Y * 0.14;
    let pad_count = clamp_platform_count_for_spacing(
        distance,
        ((distance / scaled_bhop_cadence(5.8, 7.8, rng)).round() as usize).clamp(4, 9),
        scaled_bhop_size(4.8),
        4,
    );
    let mut section_flow = flow;
    section_flow.lateral_amplitude = (section_flow.lateral_amplitude * 1.12).clamp(0.42, 1.5);
    section_flow.vertical_wave = section_flow.vertical_wave.clamp(0.03, 0.12);
    let flow_seed = rng.next_u64();
    let tops = sample_descending_platform_tops(
        path_start,
        path_end,
        right,
        pad_count,
        choose_bhop_path_style(rng, ModuleKind::MovingPlatformRun),
        rng.range_f32(1.2, 2.2),
        rng.range_f32(0.0, TAU),
        scaled_bhop_size(rng.range_f32(0.8, 1.4)),
        scaled_bhop_size(rng.range_f32(0.02, 0.06)),
        Some((section_flow, flow_seed)),
    );

    for (step, top) in tops.iter().enumerate() {
        let mover = step > 0 && step + 1 < tops.len() && step % 2 == 1;
        let pad_size = scaled_bhop_size(rng.range_f32(3.2, 4.8));
        let pad_height = scaled_bhop_size(rng.range_f32(0.72, 0.96)) * 0.45;
        let center = top_to_center(*top, pad_height);
        let body = if mover {
            let travel = if rng.chance(0.55) {
                right * scaled_bhop_size(rng.range_f32(0.56, 1.04))
            } else {
                Vec3::Y * scaled_bhop_size(rng.range_f32(0.28, 0.52))
            };
            SolidBody::Moving {
                end: center + travel,
                speed: lerp(2.1, 3.8, difficulty),
                lethal: false,
            }
        } else {
            SolidBody::Static
        };
        layout.solids.push(SolidSpec {
            owner,
            label: if mover {
                format!("Mover Hop {step}")
            } else {
                format!("Mover Catch {step}")
            },
            center,
            size: axis_box_size(along_x, pad_size, pad_height, pad_size),
            paint: if mover {
                PaintStyle::Checkpoint
            } else {
                PaintStyle::ThemeFloor(theme)
            },
            body,
            friction: None,
            extra: ExtraKind::None,
        });
    }
}

fn append_shape_gauntlet_sequence(
    layout: &mut ModuleLayout,
    owner: OwnerTag,
    rng: &mut RunRng,
    start: Vec3,
    end: Vec3,
    forward: Vec3,
    right: Vec3,
    theme: Theme,
    flow: FlowFieldProfile,
) {
    #[derive(Clone, Copy)]
    enum ShapeFamily {
        Spheres,
        Cubes,
        Trapezoids,
        Cylinders,
        Mixed,
    }

    let along_x = forward.x.abs() > 0.5;
    let distance = start.distance(end).max(18.0);
    let path_start = start + forward * bhop_path_margin(distance, 0.2, 0.95) + Vec3::Y * 0.3;
    let path_end = end - forward * bhop_path_margin(distance, 0.18, 0.88) + Vec3::Y * 0.18;
    let pad_count = clamp_platform_count_for_spacing(
        distance,
        ((distance / scaled_bhop_cadence(6.8, 8.8, rng)).round() as usize).clamp(4, 10),
        scaled_bhop_size(7.6),
        4,
    );
    let mut section_flow = flow;
    section_flow.lateral_amplitude = (section_flow.lateral_amplitude * 1.36).clamp(0.66, 1.95);
    section_flow.vertical_wave = section_flow.vertical_wave.clamp(0.03, 0.1);
    let flow_seed = rng.next_u64();
    let tops = sample_descending_platform_tops(
        path_start,
        path_end,
        right,
        pad_count,
        choose_bhop_path_style(rng, ModuleKind::ShapeGauntlet),
        rng.range_f32(1.1, 2.1),
        rng.range_f32(0.0, TAU),
        scaled_bhop_size(rng.range_f32(0.8, 1.5)),
        scaled_bhop_size(rng.range_f32(0.03, 0.09)),
        Some((section_flow, flow_seed)),
    );
    let family = rng.weighted_choice(&[
        (ShapeFamily::Spheres, 4),
        (ShapeFamily::Cubes, 2),
        (ShapeFamily::Trapezoids, 2),
        (ShapeFamily::Cylinders, 2),
        (ShapeFamily::Mixed, 3),
    ]);

    for (step, top) in tops.iter().enumerate() {
        let catch_platform = step == 0 || step + 1 == tops.len() || step % 3 == 0;
        let shape_kind = match family {
            ShapeFamily::Spheres => 0,
            ShapeFamily::Cubes => 1,
            ShapeFamily::Trapezoids => 2,
            ShapeFamily::Cylinders => 3,
            ShapeFamily::Mixed => rng.range_usize(0, 4),
        };
        match shape_kind {
            0 => {
                let radius = scaled_bhop_size(if catch_platform {
                    rng.range_f32(2.8, 4.2)
                } else {
                    rng.range_f32(2.2, 3.3)
                });
                layout.solids.push(SolidSpec {
                    owner,
                    label: format!("Sphere Gauntlet {step}"),
                    center: *top - Vec3::Y * radius,
                    size: Vec3::splat(radius * 2.0),
                    paint: if catch_platform {
                        PaintStyle::ThemeAccent(theme)
                    } else {
                        PaintStyle::ThemeFloor(theme)
                    },
                    body: SolidBody::StaticSphere,
                    friction: None,
                    extra: ExtraKind::None,
                });
            }
            1 => {
                let side = scaled_bhop_size(if catch_platform {
                    rng.range_f32(4.4, 6.2)
                } else {
                    rng.range_f32(3.0, 4.4)
                });
                layout.solids.push(SolidSpec {
                    owner,
                    label: format!("Cube Gauntlet {step}"),
                    center: top_to_center(*top, side),
                    size: axis_box_size(along_x, side, side, side),
                    paint: if catch_platform {
                        PaintStyle::ThemeAccent(theme)
                    } else {
                        PaintStyle::ThemeFloor(theme)
                    },
                    body: SolidBody::Static,
                    friction: None,
                    extra: ExtraKind::None,
                });
            }
            2 => {
                let height = scaled_bhop_size(if catch_platform {
                    rng.range_f32(3.4, 5.4)
                } else {
                    rng.range_f32(2.6, 4.2)
                }) * 0.5;
                let base = scaled_bhop_size(if catch_platform {
                    rng.range_f32(5.0, 7.2)
                } else {
                    rng.range_f32(3.6, 5.2)
                });
                let top_scale = if catch_platform {
                    rng.range_f32(0.45, 0.72)
                } else {
                    rng.range_f32(0.18, 0.48)
                };
                layout.solids.push(SolidSpec {
                    owner,
                    label: format!("Trapezoid Gauntlet {step}"),
                    center: top_to_center(*top, height),
                    size: Vec3::new(base, height, base),
                    paint: if catch_platform {
                        PaintStyle::ThemeAccent(theme)
                    } else {
                        PaintStyle::ThemeFloor(theme)
                    },
                    body: SolidBody::StaticTrapezoid { top_scale },
                    friction: None,
                    extra: ExtraKind::None,
                });
            }
            _ => {
                let radius = scaled_bhop_size(if catch_platform {
                    rng.range_f32(1.9, 2.8)
                } else {
                    rng.range_f32(1.4, 2.1)
                });
                let height = scaled_bhop_size(if catch_platform {
                    rng.range_f32(4.2, 6.4)
                } else {
                    rng.range_f32(3.1, 4.8)
                }) * 0.5;
                layout.solids.push(SolidSpec {
                    owner,
                    label: format!("Cylinder Gauntlet {step}"),
                    center: top_to_center(*top, height),
                    size: Vec3::new(radius * 2.0, height, radius * 2.0),
                    paint: if catch_platform {
                        PaintStyle::ThemeAccent(theme)
                    } else {
                        PaintStyle::ThemeFloor(theme)
                    },
                    body: SolidBody::StaticCylinder,
                    friction: None,
                    extra: ExtraKind::None,
                });
            }
        }
    }
}

fn append_descending_pad_sequence(
    layout: &mut ModuleLayout,
    owner: OwnerTag,
    rng: &mut RunRng,
    start: Vec3,
    end: Vec3,
    forward: Vec3,
    right: Vec3,
    paint: PaintStyle,
    body: impl Fn(f32) -> SolidBody,
    friction: Option<f32>,
    label_prefix: &str,
    cadence: f32,
    pad_length: f32,
    pad_height: f32,
    pad_width: f32,
    lateral_amplitude: f32,
) {
    let along_x = forward.x.abs() > 0.5;
    let distance = start.distance(end).max(12.0);
    let pad_count = ((distance / cadence).round() as usize).clamp(6, 28);
    let weave_cycles = rng.range_f32(1.15, 2.35);
    let phase = rng.range_f32(0.0, TAU);

    for step in 0..pad_count {
        let t = (step + 1) as f32 / (pad_count + 1) as f32;
        let envelope = (t * PI).sin().max(0.0).powf(0.75);
        let weave = (t * weave_cycles * TAU + phase).sin();
        let mut top = start.lerp(end, t) + right * weave * lateral_amplitude * 1.18 * envelope;
        top.y += pad_height * 0.45;
        layout.solids.push(SolidSpec {
            owner,
            label: format!("{label_prefix} {step}"),
            center: top_to_center(top, pad_height),
            size: axis_box_size(along_x, pad_length, pad_height, pad_width),
            paint,
            body: body(t),
            friction,
            extra: ExtraKind::None,
        });
    }
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
        color: tailwind::SKY_300.into(),
    });
    layout
}

fn material_for_paint(paint: PaintStyle, ghost: bool) -> StandardMaterial {
    match paint {
        PaintStyle::ThemeFloor(theme) => StandardMaterial {
            base_color: theme_floor_color(theme),
            emissive: LinearRgba::from(theme_glow_color(theme)) * 0.1,
            reflectance: 0.62,
            specular_tint: theme_glow_color(theme),
            clearcoat: 0.18,
            clearcoat_perceptual_roughness: 0.58,
            perceptual_roughness: 0.66,
            metallic: 0.02,
            ..default()
        },
        PaintStyle::ThemeAccent(theme) => StandardMaterial {
            base_color: theme_accent_color(theme),
            emissive: LinearRgba::from(theme_glow_color(theme)) * 0.16,
            reflectance: 0.76,
            specular_tint: theme_glow_color(theme),
            clearcoat: 0.62,
            clearcoat_perceptual_roughness: 0.2,
            perceptual_roughness: 0.28,
            metallic: 0.08,
            ..default()
        },
        PaintStyle::ThemeShadow(theme) => StandardMaterial {
            base_color: theme_shadow_color(theme),
            emissive: LinearRgba::from(theme_glow_color(theme)) * 0.065,
            reflectance: 0.18,
            specular_tint: theme_glow_color(theme),
            clearcoat: 0.08,
            clearcoat_perceptual_roughness: 0.72,
            perceptual_roughness: 0.86,
            ..default()
        },
        PaintStyle::SectionPlatform(theme) => {
            let pane_color = mix_color(theme_floor_color(theme), theme_glow_color(theme), 0.38)
                .with_alpha(if ghost { 0.08 } else { 0.16 });
            StandardMaterial {
                base_color: pane_color,
                alpha_mode: AlphaMode::AlphaToCoverage,
                emissive: LinearRgba::from(theme_glow_color(theme)) * 0.12,
                reflectance: 0.84,
                specular_tint: brighten(theme_glow_color(theme), 0.08),
                clearcoat: 0.82,
                clearcoat_perceptual_roughness: 0.14,
                perceptual_roughness: 0.18,
                metallic: 0.0,
                ..default()
            }
        }
        PaintStyle::Summit => StandardMaterial {
            base_color: Color::srgb(0.7, 0.64, 0.38),
            emissive: LinearRgba::rgb(0.52, 0.4, 0.16),
            reflectance: 0.72,
            clearcoat: 0.75,
            clearcoat_perceptual_roughness: 0.18,
            perceptual_roughness: 0.18,
            metallic: 0.16,
            ..default()
        },
        PaintStyle::Checkpoint => StandardMaterial {
            base_color: Color::srgb(0.18, 0.52, 0.5),
            emissive: LinearRgba::rgb(0.2, 0.46, 0.44),
            reflectance: 0.58,
            clearcoat: 0.72,
            clearcoat_perceptual_roughness: 0.16,
            perceptual_roughness: 0.18,
            ..default()
        },
        PaintStyle::Hazard => StandardMaterial {
            base_color: Color::srgb(0.62, 0.16, 0.18),
            emissive: LinearRgba::rgb(0.22, 0.05, 0.08),
            reflectance: 0.42,
            clearcoat: 0.48,
            clearcoat_perceptual_roughness: 0.22,
            perceptual_roughness: 0.28,
            ..default()
        },
        PaintStyle::Shortcut => StandardMaterial {
            base_color: if ghost {
                Color::srgba(0.22, 0.68, 0.92, 0.24)
            } else {
                Color::srgb(0.2, 0.66, 0.9)
            },
            alpha_mode: if ghost {
                AlphaMode::AlphaToCoverage
            } else {
                AlphaMode::Opaque
            },
            emissive: LinearRgba::rgb(0.22, 0.5, 0.62),
            reflectance: 0.74,
            clearcoat: 0.8,
            clearcoat_perceptual_roughness: 0.16,
            perceptual_roughness: 0.16,
            ..default()
        },
        PaintStyle::Ice => StandardMaterial {
            base_color: Color::srgba(0.56, 0.78, 0.94, 0.7),
            alpha_mode: AlphaMode::AlphaToCoverage,
            emissive: LinearRgba::rgb(0.1, 0.2, 0.26),
            reflectance: 0.94,
            clearcoat: 1.0,
            clearcoat_perceptual_roughness: 0.04,
            perceptual_roughness: 0.06,
            ..default()
        },
        PaintStyle::Water => StandardMaterial {
            base_color: Color::srgba(0.02, 0.16, 0.34, 0.5),
            alpha_mode: AlphaMode::AlphaToCoverage,
            emissive: LinearRgba::rgb(0.06, 0.16, 0.24),
            reflectance: 1.0,
            clearcoat: 1.0,
            clearcoat_perceptual_roughness: 0.02,
            perceptual_roughness: 0.04,
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
        Theme::Stone => Color::srgb(0.2, 0.22, 0.29),
        Theme::Overgrown => Color::srgb(0.09, 0.18, 0.15),
        Theme::Frost => Color::srgb(0.15, 0.22, 0.32),
        Theme::Ember => Color::srgb(0.24, 0.13, 0.17),
    }
}

fn theme_accent_color(theme: Theme) -> Color {
    match theme {
        Theme::Stone => Color::srgb(0.52, 0.58, 0.84),
        Theme::Overgrown => Color::srgb(0.24, 0.62, 0.48),
        Theme::Frost => Color::srgb(0.32, 0.72, 0.94),
        Theme::Ember => Color::srgb(0.92, 0.42, 0.5),
    }
}

fn theme_shadow_color(theme: Theme) -> Color {
    match theme {
        Theme::Stone => Color::srgb(0.08, 0.09, 0.15),
        Theme::Overgrown => Color::srgb(0.04, 0.08, 0.06),
        Theme::Frost => Color::srgb(0.06, 0.1, 0.16),
        Theme::Ember => Color::srgb(0.1, 0.04, 0.07),
    }
}

fn theme_glow_color(theme: Theme) -> Color {
    match theme {
        Theme::Stone => Color::srgb(0.72, 0.8, 1.0),
        Theme::Overgrown => Color::srgb(0.46, 0.98, 0.74),
        Theme::Frost => Color::srgb(0.76, 0.96, 1.0),
        Theme::Ember => Color::srgb(1.0, 0.64, 0.72),
    }
}

fn paint_stripe_color(paint: PaintStyle) -> Color {
    match paint {
        PaintStyle::ThemeFloor(theme)
        | PaintStyle::ThemeAccent(theme)
        | PaintStyle::ThemeShadow(theme)
        | PaintStyle::SectionPlatform(theme) => {
            let glow = LinearRgba::from(theme_glow_color(theme));
            Color::linear_rgb(glow.red * 1.1, glow.green * 1.1, glow.blue * 1.1)
        }
        PaintStyle::Summit => Color::linear_rgb(1.3, 0.98, 0.34),
        PaintStyle::Checkpoint => Color::linear_rgb(0.42, 1.2, 0.98),
        PaintStyle::Hazard => Color::linear_rgb(1.18, 0.24, 0.34),
        PaintStyle::Shortcut => Color::linear_rgb(0.42, 1.1, 1.28),
        PaintStyle::Ice => Color::linear_rgb(0.66, 1.1, 1.3),
        PaintStyle::Water => Color::linear_rgb(0.3, 0.68, 1.22),
    }
}

fn projected_gap(step_distance: f32, from_size: Vec2, to_size: Vec2) -> f32 {
    step_distance - (from_size.max_element() + to_size.max_element()) * 0.5
}

fn edge_gap(from: &RoomPlan, to: &RoomPlan) -> f32 {
    let forward = direction_from_delta(to.top - from.top);
    let center_gap = (to.top - from.top).xz().length();
    center_gap - (room_forward_extent(from, forward) + room_forward_extent(to, -forward)) * 0.5
}

fn room_edge(room: &RoomPlan, forward: Vec3) -> Vec3 {
    let extent = room_forward_extent(room, forward);
    room.top + forward * (extent * 0.5 - 1.05)
}

fn underside_y(top: f32, thickness: f32) -> f32 {
    top - thickness
}

fn direction_from_delta(delta: Vec3) -> Vec3 {
    Vec3::new(delta.x, 0.0, delta.z).normalize_or_zero()
}

fn room_forward_extent(room: &RoomPlan, forward: Vec3) -> f32 {
    let dir = forward.xz().normalize_or_zero();
    dir.x.abs() * room.size.x + dir.y.abs() * room.size.y
}

fn room_grid_cell(top: Vec3) -> IVec2 {
    IVec2::new(
        (top.x / ROOM_GRID_SIZE).round() as i32,
        (top.z / ROOM_GRID_SIZE).round() as i32,
    )
}

fn course_visual_center_and_radius(blueprint: &RunBlueprint) -> (Vec3, f32) {
    let center = blueprint
        .rooms
        .iter()
        .map(|room| room.top)
        .fold(Vec3::ZERO, |acc, top| acc + top)
        / blueprint.rooms.len().max(1) as f32;
    let radius = blueprint
        .rooms
        .iter()
        .map(|room| {
            Vec2::new(room.top.x - center.x, room.top.z - center.z).length()
                + room.size.max_element()
        })
        .fold(CELL_SIZE * 2.5, f32::max);
    (center, radius)
}

fn floating_sphere_color(index: usize) -> Color {
    match index % 8 {
        0 => Color::srgb(0.38, 0.62, 1.0),
        1 => Color::srgb(0.28, 0.82, 0.62),
        2 => Color::srgb(0.8, 0.5, 0.98),
        3 => Color::srgb(1.0, 0.58, 0.34),
        4 => Color::srgb(0.98, 0.82, 0.42),
        5 => Color::srgb(0.36, 0.9, 0.94),
        6 => Color::srgb(0.96, 0.44, 0.72),
        _ => Color::srgb(0.86, 0.9, 1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_flow() -> FlowFieldProfile {
        default_flow_profile()
    }

    fn test_room(index: usize, cell: IVec2, top: Vec3, seed: u64) -> RoomPlan {
        RoomPlan {
            index,
            cell,
            top,
            size: Vec2::splat(15.0),
            theme: Theme::Frost,
            seed,
            section: RoomSectionKind::OpenPad,
            checkpoint_slot: Some(index),
            biome: BiomeStyle::NeonCyber,
            zone_index: 0,
            layer_index: 0,
        }
    }

    fn test_segment(index: usize, kind: ModuleKind, seed: u64) -> SegmentPlan {
        SegmentPlan {
            index,
            from: 0,
            to: 1,
            kind,
            difficulty: 0.52,
            seed,
            zone_role: ZoneRole::Accelerator,
            zone_signature: ZoneSignature::WaveRamps,
            exit_socket: module_template(kind).exit,
            zone_index: 0,
            biome: BiomeStyle::NeonCyber,
            connector: ConnectorKind::Transfer,
            flow: test_flow(),
            route_lines: vec![RouteLine::Speed],
            zone_local_t: 0.5,
        }
    }

    fn test_zone(end_segment: usize) -> ZonePlan {
        ZonePlan {
            index: 0,
            kind: ZoneKind::Accelerator,
            role: ZoneRole::Accelerator,
            signature: ZoneSignature::WaveRamps,
            biome: BiomeStyle::NeonCyber,
            start_segment: 0,
            end_segment,
            flow: test_flow(),
            entry_connector: ConnectorKind::Funnel,
            exit_connector: ConnectorKind::Collector,
            landmark: LandmarkKind::BrokenRing,
            route_lines: vec![RouteLine::Safe, RouteLine::Speed, RouteLine::Trick],
        }
    }

    #[test]
    fn graph_stage_generates_consistent_topology() {
        let graph = build_course_graph(42);
        assert!(graph.rooms.len() >= 2);
        assert_eq!(graph.segments.len(), graph.rooms.len() - 1);
        assert!(!graph.zones.is_empty());
    }

    #[test]
    fn compile_stage_derives_spawn_and_summit_from_graph() {
        let graph = build_course_graph(42);
        let blueprint = compile_course_graph(graph.clone());

        assert!(blueprint.spawn.y > graph.rooms[0].top.y);
        assert_eq!(blueprint.spawn.x, graph.rooms[0].top.x);
        assert!(blueprint.summit.y > graph.rooms.last().unwrap().top.y);
    }

    #[test]
    fn layout_stage_builds_room_segment_and_summit_chunks() {
        let blueprint = build_run_blueprint(42);

        let room_layout = build_chunk_layout(WorldChunkKey::Room(0), &blueprint).unwrap();
        let segment_layout = build_chunk_layout(WorldChunkKey::Segment(0), &blueprint).unwrap();
        let summit_layout = build_chunk_layout(WorldChunkKey::Summit, &blueprint).unwrap();

        assert!(!room_layout.solids.is_empty());
        assert!(!segment_layout.solids.is_empty());
        assert!(!summit_layout.solids.is_empty());
    }

    #[test]
    fn room_layout_uses_glass_section_platform_without_checkpoint_marker_solid() {
        let room = test_room(3, IVec2::new(0, 0), Vec3::new(0.0, 32.0, 0.0), 77);
        let layout = build_room_layout(&room);

        let room_platform = layout
            .solids
            .iter()
            .find(|solid| solid.label == "Room 3")
            .expect("room platform should exist");

        assert!(matches!(
            room_platform.paint,
            PaintStyle::SectionPlatform(Theme::Frost)
        ));
        assert!(matches!(room_platform.extra, ExtraKind::None));
        assert!(is_batchable_static_render(room_platform));
        assert!(
            layout.solids.iter().all(|solid| solid.label != "Checkpoint 3"),
            "checkpoint marker slab should no longer be spawned"
        );
        assert!(
            layout.features.iter().any(|feature| matches!(
                feature,
                FeatureSpec::CheckpointPad { index, .. } if *index == 3
            )),
            "checkpoint tracking should be preserved as a non-rendered feature"
        );
    }

    #[test]
    fn section_platform_material_is_translucent_glass() {
        let material = material_for_paint(PaintStyle::SectionPlatform(Theme::Stone), false);

        assert!(matches!(material.alpha_mode, AlphaMode::AlphaToCoverage));
        assert!(material.base_color.alpha() < 1.0);
        assert!(material.reflectance >= 0.8);
        assert!(material.clearcoat >= 0.8);
    }

    #[test]
    fn surf_material_is_transparent_and_glossy() {
        let mut cache = WorldAssetCache::default();
        let mut materials = Assets::<StandardMaterial>::default();
        let handle = cached_game_material(
            &mut cache,
            &mut materials,
            GameMaterialKey {
                paint: PaintStyle::ThemeFloor(Theme::Frost),
                ghost: false,
                surf: true,
                vertex_colored: true,
            },
        );
        let material = materials
            .get(&handle)
            .expect("cached surf material should exist");

        assert!(matches!(material.alpha_mode, AlphaMode::Blend));
        assert!(material.base_color.alpha() < 1.0);
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
    fn surf_wedge_stripes_stay_within_wedge_bounds() {
        let points = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(8.0, 0.0, 0.0),
            Vec3::new(0.0, -2.1, 5.8),
            Vec3::new(8.0, -1.6, 5.8),
            Vec3::new(0.0, -0.16, 0.0),
            Vec3::new(8.0, -0.16, 0.0),
            Vec3::new(0.0, -2.26, 5.8),
            Vec3::new(8.0, -1.76, 5.8),
        ];
        let mut builder = ColoredMeshBuilder::default();
        append_surf_wedge_render_geometry(
            &mut builder,
            Vec3::ZERO,
            &points,
            Color::srgb(0.2, 0.3, 0.5),
            Color::srgb(0.9, 0.9, 1.0),
        );

        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for point in &points {
            min = min.min(*point);
            max = max.max(*point);
        }

        for vertex in &builder.positions {
            let point = Vec3::new(vertex[0], vertex[1], vertex[2]);
            assert!(
                point.x >= min.x - 0.001
                    && point.x <= max.x + 0.001
                    && point.y >= min.y - 0.001
                    && point.y <= max.y + 0.001
                    && point.z >= min.z - 0.001
                    && point.z <= max.z + 0.001,
                "generated stripe vertex {:?} escaped wedge bounds {:?}..{:?}",
                point,
                min,
                max
            );
        }
    }

    #[test]
    fn surf_wedges_are_batchable_static_render() {
        let surf = SolidSpec {
            owner: OwnerTag::Segment(0),
            label: "Surf".into(),
            center: Vec3::ZERO,
            size: Vec3::new(6.0, 2.0, 4.0),
            paint: PaintStyle::ThemeFloor(Theme::Frost),
            body: SolidBody::StaticSurfWedge {
                wall_side: 1.0,
                render_points: vec![
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(6.0, 0.0, 0.0),
                    Vec3::new(0.0, -1.2, 4.0),
                    Vec3::new(6.0, -1.2, 4.0),
                    Vec3::new(0.0, -0.24, 0.0),
                    Vec3::new(6.0, -0.24, 0.0),
                    Vec3::new(0.0, -1.44, 4.0),
                    Vec3::new(6.0, -1.44, 4.0),
                ],
            },
            friction: Some(0.0),
            extra: ExtraKind::None,
        };

        assert!(is_batchable_static_render(&surf));
    }

    #[test]
    fn theme_palette_stays_varied() {
        let floors = [
            theme_floor_color(Theme::Stone),
            theme_floor_color(Theme::Overgrown),
            theme_floor_color(Theme::Frost),
            theme_floor_color(Theme::Ember),
        ];
        let glows = [
            theme_glow_color(Theme::Stone),
            theme_glow_color(Theme::Overgrown),
            theme_glow_color(Theme::Frost),
            theme_glow_color(Theme::Ember),
        ];

        for pair in floors.windows(2) {
            assert_ne!(pair[0], pair[1]);
        }
        for pair in glows.windows(2) {
            assert_ne!(pair[0], pair[1]);
        }
    }

    #[test]
    fn landmark_selection_avoids_parallel_bridge_variant() {
        for seed in 0_u64..32 {
            let mut rng = RunRng::new(seed);
            assert!(
                !matches!(
                    choose_landmark_kind(&mut rng, ZoneRole::Technical, ZoneSignature::BraidLanes),
                    LandmarkKind::ImpossibleBridge
                )
            );
            assert!(
                !matches!(
                    choose_landmark_kind(
                        &mut rng,
                        ZoneRole::Technical,
                        ZoneSignature::SplitTransfer
                    ),
                    LandmarkKind::ImpossibleBridge
                )
            );
        }
    }

    #[test]
    fn zone_generation_cycles_roles_and_assigns_landmarks() {
        let graph = build_course_graph(42);
        assert!(graph.zones.len() >= 4);

        let roles = graph
            .zones
            .iter()
            .take(4)
            .map(|zone| zone.role)
            .collect::<Vec<_>>();
        assert_eq!(
            roles,
            vec![
                ZoneRole::Accelerator,
                ZoneRole::Technical,
                ZoneRole::Recovery,
                ZoneRole::Spectacle
            ]
        );
        assert!(
            graph
                .zones
                .iter()
                .all(|zone| !zone.route_lines.is_empty()),
            "every zone should expose at least one route line"
        );
    }

    #[test]
    fn generated_run_contains_multi_line_choice_segments() {
        let blueprint = build_run_blueprint(1337);
        assert!(
            blueprint
                .segments
                .iter()
                .any(|segment| segment.route_lines.len() >= 2),
            "expected at least one route-choice segment"
        );
        assert!(
            blueprint.segments.iter().any(|segment| {
                segment.route_lines.contains(&RouteLine::Safe)
                    && segment.route_lines.contains(&RouteLine::Speed)
                    && segment.route_lines.contains(&RouteLine::Trick)
            }),
            "expected at least one full safe/speed/trick segment"
        );
    }

    #[test]
    fn generated_solids_have_valid_collider_bounds() {
        for seed in 0_u64..256 {
            let blueprint = build_run_blueprint(seed);

            for room in &blueprint.rooms {
                let layout = build_chunk_layout(WorldChunkKey::Room(room.index), &blueprint).unwrap();
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
                let layout =
                    build_chunk_layout(WorldChunkKey::Segment(segment.index), &blueprint).unwrap();
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

            let summit = build_chunk_layout(WorldChunkKey::Summit, &blueprint).unwrap();
            for solid in &summit.solids {
                assert!(
                    validate_solid_spec(solid).is_ok(),
                    "seed {seed} summit solid '{}' invalid: {}",
                    solid.label,
                    validate_solid_spec(solid).unwrap_err()
                );
            }
        }
    }

    #[test]
    fn celestial_bodies_keep_clear_of_the_course() {
        for seed in 0_u64..256 {
            let blueprint = build_run_blueprint(seed);
            for body in build_celestial_body_plans(&blueprint) {
                let clearance = celestial_course_clearance(
                    &blueprint,
                    body.anchor,
                    body.radius * body.shape.clearance_multiplier(),
                );
                assert!(
                    clearance >= 115.0,
                    "seed {seed} celestial body at {:?} radius {} only had clearance {}",
                    body.anchor,
                    body.radius * body.shape.clearance_multiplier(),
                    clearance
                );
            }
        }
    }

    #[test]
    fn seed_12_celestials_keep_clear_of_the_course() {
        let blueprint = build_run_blueprint(12);
        for body in build_celestial_body_plans(&blueprint) {
            let clearance = celestial_course_clearance(
                &blueprint,
                body.anchor,
                body.radius * body.shape.clearance_multiplier(),
            );
            assert!(
                clearance >= 115.0,
                "seed 12 celestial body at {:?} radius {} only had clearance {}",
                body.anchor,
                body.radius * body.shape.clearance_multiplier(),
                clearance
            );
        }
    }

    #[test]
    fn generated_celestials_avoid_the_entry_view_corridor() {
        for seed in 0_u64..256 {
            let blueprint = build_run_blueprint(seed);
            for body in build_celestial_body_plans(&blueprint) {
                let view_clearance =
                    celestial_entry_view_clearance(&blueprint, body.anchor, body.radius);
                let minimum = if body.radius >= 800.0 { 220.0 } else { 100.0 };
                assert!(
                    view_clearance >= minimum,
                    "seed {seed} celestial body {:?} radius {} intruded into entry view corridor with clearance {}",
                    body.anchor,
                    body.radius,
                    view_clearance,
                );
                assert!(
                    !(body.radius >= 800.0 && body.shape == CelestialShapeKind::Sphere),
                    "seed {seed} generated a giant central-prone sphere with radius {}",
                    body.radius,
                );
            }
        }
    }

    #[test]
    fn seed_0_celestials_avoid_the_entry_view_corridor() {
        let blueprint = build_run_blueprint(0);
        for body in build_celestial_body_plans(&blueprint) {
            let view_clearance = celestial_entry_view_clearance(&blueprint, body.anchor, body.radius);
            let minimum = if body.radius >= 800.0 { 220.0 } else { 100.0 };
            assert!(
                view_clearance >= minimum,
                "seed 0 celestial body {:?} radius {} intruded into entry view corridor with clearance {}",
                body.anchor,
                body.radius,
                view_clearance,
            );
            assert!(
                !(body.radius >= 800.0 && body.shape == CelestialShapeKind::Sphere),
                "seed 0 generated a giant central-prone sphere with radius {}",
                body.radius,
            );
        }
    }

    #[test]
    fn giant_celestials_span_high_low_and_close_to_the_course() {
        let mut closest_clearance_seen = f32::INFINITY;
        for seed in 0_u64..16 {
            let blueprint = build_run_blueprint(seed);
            let (center, _) = course_visual_center_and_radius(&blueprint);
            let giant_bodies: Vec<_> = build_celestial_body_plans(&blueprint)
                .into_iter()
                .filter(|body| body.radius >= 800.0)
                .collect();

            let highest = giant_bodies
                .iter()
                .map(|body| body.anchor.y - center.y)
                .fold(f32::NEG_INFINITY, f32::max);
            let lowest = giant_bodies
                .iter()
                .map(|body| body.anchor.y - center.y)
                .fold(f32::INFINITY, f32::min);
            let closest_clearance = giant_bodies
                .iter()
                .map(|body| {
                    celestial_course_clearance(
                        &blueprint,
                        body.anchor,
                        body.radius * body.shape.clearance_multiplier(),
                    )
                })
                .fold(f32::INFINITY, f32::min);
            closest_clearance_seen = closest_clearance_seen.min(closest_clearance);

            assert!(
                highest >= 420.0,
                "seed {seed} only reached highest giant offset {highest}"
            );
            assert!(
                lowest <= -220.0,
                "seed {seed} only reached lowest giant offset {lowest}"
            );
        }
        assert!(
            closest_clearance_seen <= 580.0,
            "sample never brought a giant body within 580 units of the course; closest was {closest_clearance_seen}"
        );
    }

    #[test]
    fn celestial_budget_restores_dense_sky() {
        let blueprint = build_run_blueprint(42);
        let bodies = build_celestial_body_plans(&blueprint);
        let average_radius =
            bodies.iter().map(|body| body.radius).sum::<f32>() / bodies.len().max(1) as f32;

        assert!(
            bodies.len() >= 50,
            "celestial density regressed to {} bodies",
            bodies.len()
        );
        assert!(
            average_radius >= 700.0,
            "average celestial radius dropped to {average_radius}"
        );
        assert!(
            bodies.iter().any(|body| body.radius >= 2_000.0),
            "expected at least one very large ambient shape"
        );
    }

    #[test]
    fn new_speed_modules_generate_valid_solids() {
        let rooms = vec![
            test_room(0, IVec2::ZERO, Vec3::new(0.0, 120.0, 0.0), 1),
            test_room(1, IVec2::new(4, 1), Vec3::new(92.0, 84.0, 26.0), 2),
        ];

        for (index, kind) in [
            ModuleKind::StairRun,
            ModuleKind::WindowHop,
            ModuleKind::PillarAirstrafe,
            ModuleKind::HeadcheckRun,
            ModuleKind::SpeedcheckRun,
            ModuleKind::MovingPlatformRun,
            ModuleKind::ShapeGauntlet,
        ]
        .into_iter()
        .enumerate()
        {
            let mut segment = test_segment(index, kind, 0xC0DE_BAAD_u64 ^ index as u64);
            segment.difficulty = 0.52;
            let layout = build_segment_layout(&segment, &rooms);
            assert!(
                !layout.solids.is_empty(),
                "module {:?} should generate gameplay solids",
                kind
            );
            for solid in &layout.solids {
                assert!(
                    validate_solid_spec(solid).is_ok(),
                    "module {:?} solid '{}' invalid: {}",
                    kind,
                    solid.label,
                    validate_solid_spec(solid).unwrap_err()
                );
            }
        }
    }

    #[test]
    fn bhop_platform_sampling_hugs_segment_endcaps() {
        let start = Vec3::new(0.0, 10.0, 0.0);
        let end = Vec3::new(100.0, -10.0, 0.0);
        let points = sample_descending_platform_tops(
            start,
            end,
            Vec3::Z,
            6,
            PathLateralStyle::Serpentine,
            2.0,
            1.3,
            50.0,
            20.0,
            None,
        );

        assert_eq!(points.len(), 6);
        assert!(
            points.first().unwrap().distance(start) < 0.001,
            "first bhop point drifted too far from the entry endcap"
        );
        assert!(
            points.last().unwrap().distance(end) < 0.001,
            "last bhop point drifted too far from the exit endcap"
        );
    }

    #[test]
    fn bhop_modules_start_close_to_the_entry_platform() {
        let rooms = vec![
            test_room(0, IVec2::ZERO, Vec3::new(0.0, 120.0, 0.0), 1),
            test_room(1, IVec2::new(4, 1), Vec3::new(92.0, 84.0, 26.0), 2),
        ];
        let forward = direction_from_delta(rooms[1].top - rooms[0].top);
        let start = room_edge(&rooms[0], forward);

        for (index, kind) in [ModuleKind::StairRun, ModuleKind::ShapeGauntlet]
            .into_iter()
            .enumerate()
        {
            let mut segment = test_segment(index, kind, 0xA11C_7000_u64 ^ index as u64);
            segment.difficulty = 0.52;
            let layout = build_segment_layout(&segment, &rooms);
            let closest_progress = layout
                .solids
                .iter()
                .map(|solid| {
                    let top = solid.center + Vec3::Y * (solid.size.y * 0.5);
                    (top - start).dot(forward)
                })
                .filter(|progress| *progress > -0.5)
                .fold(f32::INFINITY, f32::min);

            assert!(
                closest_progress <= 1.6,
                "module {:?} started too far from the entry platform: {}",
                kind,
                closest_progress
            );
        }
    }

    #[test]
    fn route_generation_varies_turn_strength() {
        let mut saw_straight = false;
        let mut saw_curved = false;

        for seed in 0_u64..24 {
            let blueprint = build_run_blueprint(seed);
            for triple in blueprint.rooms.windows(3) {
                let a = direction_from_delta(triple[1].top - triple[0].top);
                let b = direction_from_delta(triple[2].top - triple[1].top);
                if a == Vec3::ZERO || b == Vec3::ZERO {
                    continue;
                }
                let turn = a.angle_between(b).to_degrees();
                if turn <= 1.6 {
                    saw_straight = true;
                }
                if turn >= 7.5 {
                    saw_curved = true;
                }
            }
        }

        assert!(
            saw_straight,
            "expected at least some near-straight route sections"
        );
        assert!(
            saw_curved,
            "expected at least some strongly curving route sections"
        );
    }

    #[test]
    fn first_segment_is_always_surf() {
        for seed in 0_u64..128 {
            let blueprint = build_run_blueprint(seed);
            assert_eq!(
                blueprint.segments.first().map(|segment| segment.kind),
                Some(ModuleKind::SurfRamp),
                "seed {seed} did not start with a surf segment"
            );
        }
    }

    #[test]
    fn infinite_append_adds_more_rooms_and_segments() {
        let mut blueprint = build_run_blueprint(0x1eed_5eed);
        let original_room_count = blueprint.rooms.len();
        let original_segment_count = blueprint.segments.len();
        let original_tail_y = blueprint.rooms.last().unwrap().top.y;

        append_run_blueprint(&mut blueprint, 8);

        assert_eq!(blueprint.rooms.len(), original_room_count + 8);
        assert_eq!(blueprint.segments.len(), original_segment_count + 8);
        assert!(blueprint.rooms.last().unwrap().top.y < original_tail_y);
        assert_eq!(blueprint.rooms.len(), blueprint.segments.len() + 1);
    }

    #[test]
    fn endless_chunk_window_never_spawns_summit_chunk() {
        let blueprint = build_run_blueprint(0x51de_cafe);
        for focus_room in [
            0,
            blueprint.rooms.len() / 2,
            blueprint.rooms.len().saturating_sub(1),
        ] {
            assert!(
                !desired_chunk_window(&blueprint, focus_room).contains(&WorldChunkKey::Summit),
                "summit chunk was still requested for focus room {focus_room}",
            );
        }
    }

    #[test]
    fn checkpoint_death_plane_looks_ahead_for_long_drops() {
        let blueprint = RunBlueprint {
            seed: 7,
            rooms: vec![
                test_room(0, IVec2::ZERO, Vec3::new(0.0, 420.0, 0.0), 1),
                test_room(1, IVec2::new(3, 0), Vec3::new(120.0, 240.0, 0.0), 2),
                test_room(2, IVec2::new(7, 0), Vec3::new(280.0, 40.0, 0.0), 3),
            ],
            segments: Vec::new(),
            spawn: Vec3::new(0.0, 422.0, 0.0),
            summit: Vec3::new(280.0, 41.4, 0.0),
            death_plane: -999.0,
            stats: GenerationStats::default(),
            zones: vec![test_zone(1)],
            zone_edges: Vec::new(),
        };
        let death_plane = checkpoint_death_plane(&blueprint, 0, 0);

        assert!(
            death_plane < blueprint.rooms[1].top.y - 40.0,
            "death plane {} still sits too high for a long descending opener",
            death_plane
        );
        assert!(
            death_plane <= blueprint.rooms[2].top.y - CHECKPOINT_DEATH_MARGIN,
            "death plane {} failed to look ahead to the deeper generated rooms",
            death_plane
        );
    }

    #[test]
    fn giant_bhop_sections_do_not_self_overlap() {
        let rooms = vec![
            test_room(0, IVec2::ZERO, Vec3::new(0.0, 120.0, 0.0), 1),
            test_room(1, IVec2::new(6, 1), Vec3::new(180.0, 72.0, 24.0), 2),
        ];

        for (index, kind) in [ModuleKind::StairRun, ModuleKind::ShapeGauntlet]
            .into_iter()
            .enumerate()
        {
            let mut segment = test_segment(index, kind, 0x51DE_CAFE_u64 ^ index as u64);
            segment.difficulty = 0.62;
            let layout = build_segment_layout(&segment, &rooms);
            let volumes = layout
                .solids
                .iter()
                .filter_map(|solid| solid.preview_volume())
                .collect::<Vec<_>>();

            for i in 0..volumes.len() {
                for j in i + 1..volumes.len() {
                    assert!(
                        !intersects(volumes[i], volumes[j], 0.02),
                        "module {:?} produced overlapping gameplay solids between '{}' and '{}'",
                        kind,
                        layout.solids[i].label,
                        layout.solids[j].label,
                    );
                }
            }
        }
    }

    #[test]
    fn route_choice_lines_keep_distinct_corridors() {
        for seed in 0_u64..24 {
            let blueprint = build_run_blueprint(seed);
            for segment in &blueprint.segments {
                let layout = build_segment_layout(segment, &blueprint.rooms);
                assert!(
                    !layout_has_internal_route_line_overlap(&layout),
                    "seed {seed} segment {} {:?} still had cross-line overlap",
                    segment.index,
                    segment.kind,
                );
                if segment.route_lines.len() > 1
                    && layout
                        .solids
                        .iter()
                        .any(|solid| matches!(&solid.body, SolidBody::StaticSurfStrip { .. }))
                {
                    assert!(
                        layout
                            .solids
                            .iter()
                            .filter_map(|solid| route_line_from_label(&solid.label))
                            .collect::<HashSet<_>>()
                            .len()
                            <= 1,
                        "seed {seed} segment {} kept multiple surf-bearing route lines active",
                        segment.index,
                    );
                }
            }
        }
    }

    #[test]
    fn surf_segments_share_seam_edges() {
        let mut layout = ModuleLayout::default();
        let mut rng = RunRng::new(0x5eed_cafe_u64);
        append_css_surf_sequence(
            &mut layout,
            OwnerTag::Segment(0),
            &mut rng,
            Vec3::new(0.0, 40.0, 0.0),
            Vec3::new(120.0, 18.0, 26.0),
            Vec3::X,
            Vec3::Z,
            Theme::Frost,
            test_flow(),
            true,
            None,
        );

        let wedges = layout
            .solids
            .iter()
            .filter_map(|solid| match &solid.body {
                SolidBody::StaticSurfWedge {
                    wall_side,
                    render_points,
                    ..
                } if *wall_side > 0.0 && !render_points.is_empty() => {
                    Some((solid.center, render_points))
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        for pair in wedges.windows(2) {
            let (a_center, a_points) = &pair[0];
            let (b_center, b_points) = &pair[1];
            let a_back_ridge = *a_center + a_points[1];
            let a_back_outer = *a_center + a_points[3];
            let b_front_ridge = *b_center + b_points[0];
            let b_front_outer = *b_center + b_points[2];

            assert!(
                a_back_ridge.distance(b_front_ridge) < 0.001,
                "ridge seam mismatch: {:?} vs {:?}",
                a_back_ridge,
                b_front_ridge
            );
            assert!(
                a_back_outer.distance(b_front_outer) < 0.001,
                "outer seam mismatch: {:?} vs {:?}",
                a_back_outer,
                b_front_outer
            );
        }
    }

    #[test]
    fn surf_collider_strip_extends_past_render_seams() {
        let mut layout = ModuleLayout::default();
        let mut rng = RunRng::new(0x5eed_cafe_u64);
        append_css_surf_sequence(
            &mut layout,
            OwnerTag::Segment(0),
            &mut rng,
            Vec3::new(0.0, 40.0, 0.0),
            Vec3::new(120.0, 18.0, 26.0),
            Vec3::X,
            Vec3::Z,
            Theme::Frost,
            test_flow(),
            true,
            None,
        );

        let render_wedges = layout
            .solids
            .iter()
            .filter_map(|solid| match &solid.body {
                SolidBody::StaticSurfWedge { wall_side, .. } if *wall_side > 0.0 => {
                    match &solid.body {
                        SolidBody::StaticSurfWedge { render_points, .. }
                            if !render_points.is_empty() =>
                        {
                            Some((solid.center, render_points))
                        }
                        _ => None,
                    }
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        let render_start_ridge =
            render_wedges.first().unwrap().0 + render_wedges.first().unwrap().1[0];
        let render_end_ridge = render_wedges.last().unwrap().0 + render_wedges.last().unwrap().1[1];

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

        assert_eq!(
            strips.len(),
            1,
            "expected one collider strip on the positive surf wall"
        );
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
                > SURF_COLLIDER_OVERLAP_MIN * 0.75,
            "collider strip did not extend before the render start seam"
        );
        assert!(
            (strip_end_ridge - render_end_ridge).dot(end_dir) > SURF_COLLIDER_OVERLAP_MIN * 0.75,
            "collider strip did not extend past the render end seam"
        );
    }

    #[test]
    fn surf_uses_fewer_collider_segments_than_render_segments() {
        let mut layout = ModuleLayout::default();
        let mut rng = RunRng::new(0x5eed_cafe_u64);
        append_css_surf_sequence(
            &mut layout,
            OwnerTag::Segment(0),
            &mut rng,
            Vec3::new(0.0, 40.0, 0.0),
            Vec3::new(120.0, 18.0, 26.0),
            Vec3::X,
            Vec3::Z,
            Theme::Frost,
            test_flow(),
            true,
            None,
        );

        let render_count = layout
            .solids
            .iter()
            .filter(|solid| {
                matches!(
                    &solid.body,
                    SolidBody::StaticSurfWedge { render_points, .. } if !render_points.is_empty()
                )
            })
            .count();
        let collider_count = layout
            .solids
            .iter()
            .filter(|solid| {
                matches!(
                    &solid.body,
                    SolidBody::StaticSurfStrip { collider_strip_points, .. }
                        if !collider_strip_points.is_empty()
                )
            })
            .count();

        assert!(
            collider_count < render_count,
            "expected fewer collider strips than render wedges, got {collider_count} vs {render_count}"
        );
    }

    #[test]
    fn surf_collider_strip_uses_denser_samples_than_render_wedges() {
        let mut layout = ModuleLayout::default();
        let mut rng = RunRng::new(0x5eed_cafe_u64);
        append_css_surf_sequence(
            &mut layout,
            OwnerTag::Segment(0),
            &mut rng,
            Vec3::new(0.0, 40.0, 0.0),
            Vec3::new(120.0, 18.0, 26.0),
            Vec3::X,
            Vec3::Z,
            Theme::Frost,
            test_flow(),
            true,
            None,
        );

        let render_wedge_count = layout
            .solids
            .iter()
            .filter(|solid| {
                matches!(
                    &solid.body,
                    SolidBody::StaticSurfWedge { wall_side, render_points }
                        if *wall_side > 0.0 && !render_points.is_empty()
                )
            })
            .count();
        let strip_sample_count = layout
            .solids
            .iter()
            .find_map(|solid| match &solid.body {
                SolidBody::StaticSurfStrip {
                    wall_side,
                    collider_strip_points,
                    columns,
                } if *wall_side > 0.0 && !collider_strip_points.is_empty() => {
                    Some(collider_strip_points.len() / *columns)
                }
                _ => None,
            })
            .unwrap();

        assert!(
            strip_sample_count > render_wedge_count,
            "expected collider strip samples to exceed render wedge seams, got {strip_sample_count} vs {render_wedge_count}"
        );
    }

    #[test]
    fn surf_ramp_rotation_matches_shared_ridge_surf() {
        for &forward in &[Vec3::X, Vec3::Z] {
            for side in [-1.0_f32, 1.0] {
                let size = Vec3::new(8.0, 5.2, 7.0);
                let outward = Vec3::new(-forward.z, 0.0, forward.x) * side;
                let rotation = surf_ramp_rotation(forward);
                let normal = rotation * surf_wedge_surface_normal(size, side);
                let length_axis = rotation * Vec3::X;
                let slope_angle = normal.angle_between(Vec3::Y).to_degrees();

                assert!(
                    length_axis.dot(forward) > 0.9,
                    "length axis {:?} should follow forward {:?}",
                    length_axis,
                    forward
                );
                assert!(
                    normal.dot(forward).abs() < 0.05,
                    "surface normal {:?} should stay nearly constant along forward {:?}",
                    normal,
                    forward
                );
                assert!(
                    normal.dot(outward).abs() > 0.45,
                    "surface normal {:?} should lean sideways relative to wall side {:?}",
                    normal,
                    outward
                );
                assert!(
                    (32.0..48.0).contains(&slope_angle),
                    "surface angle {} should stay within the playable surf range",
                    slope_angle
                );
            }
        }
    }

    #[test]
    fn surf_one_sided_arc_does_not_flip_direction() {
        for &phase in &[0.2, 1.1, 2.6, 4.2, 5.7] {
            let mut sign = 0.0;
            for step in 0..33 {
                let t = step as f32 / 32.0;
                let offset = path_lateral_offset(
                    PathLateralStyle::OneSidedArc,
                    t,
                    (t * PI).sin().max(0.0),
                    phase,
                    0.0,
                    1.0,
                );
                if offset.abs() < 0.0001 {
                    continue;
                }
                let current = offset.signum();
                if sign == 0.0 {
                    sign = current;
                } else {
                    assert!(
                        current == sign,
                        "one-sided surf arc flipped direction for phase {phase}: {sign} then {current}"
                    );
                }
            }
        }
    }

    #[test]
    fn chunk_window_keeps_frontier_connectors_loaded() {
        let blueprint = build_run_blueprint(0x51eed_u64);
        let focus_room = 8.min(blueprint.rooms.len().saturating_sub(2));
        let window = stream_window(&blueprint, focus_room);
        let chunks = desired_chunk_window(&blueprint, focus_room);

        assert!(
            chunks.contains(&WorldChunkKey::Room(window.frontier_room)),
            "frontier room {} should be loaded",
            window.frontier_room
        );

        if window.frontier_room > 0 {
            assert!(
                chunks.contains(&WorldChunkKey::Segment(window.frontier_room - 1)),
                "connector segment {} -> {} should be loaded",
                window.frontier_room - 1,
                window.frontier_room
            );
        }

        if window.frontier_room > 1 {
            assert!(
                chunks.contains(&WorldChunkKey::Room(window.frontier_room - 1)),
                "landing room {} ahead of the previous frontier connector should be loaded",
                window.frontier_room - 1
            );
        }
    }

    #[test]
    fn chunk_window_keeps_next_landing_room_loaded() {
        let blueprint = build_run_blueprint(0x7eed_u64);
        let focus_room = 10.min(blueprint.rooms.len().saturating_sub(3));
        let window = stream_window(&blueprint, focus_room);
        let chunks = desired_chunk_window(&blueprint, focus_room);

        assert!(
            chunks.contains(&WorldChunkKey::Room(window.frontier_room)),
            "front landing room {} should be loaded",
            window.frontier_room
        );
        if window.frontier_room > 0 {
            assert!(
                chunks.contains(&WorldChunkKey::Segment(window.frontier_room - 1)),
                "segment into the front landing room {} should be loaded",
                window.frontier_room
            );
        }
        if window.frontier_room > 1 {
            assert!(
                chunks.contains(&WorldChunkKey::Segment(window.frontier_room - 2)),
                "penultimate connector into the approach room should be loaded"
            );
        }
    }

    #[test]
    fn stream_focus_advances_with_player_position() {
        let blueprint = build_run_blueprint(0x5eaf_u64);
        let checkpoint = 4.min(blueprint.rooms.len().saturating_sub(3));
        let ahead_room = (checkpoint + 3).min(blueprint.rooms.len() - 1);
        let player_position = blueprint.rooms[ahead_room].top + Vec3::new(0.0, 1.0, 0.0);
        let focus = stream_focus_room(&blueprint, checkpoint, checkpoint, player_position);

        assert!(
            focus >= ahead_room.saturating_sub(1),
            "focus room {} should advance toward the player's actual route position {}",
            focus,
            ahead_room
        );
    }

    #[test]
    fn stream_focus_handles_wide_lateral_turns() {
        let blueprint = build_run_blueprint(0x5ea7_cafe_u64);
        let checkpoint = 5.min(blueprint.rooms.len().saturating_sub(4));
        let segment_index = (checkpoint + 2).min(blueprint.segments.len().saturating_sub(1));
        let segment = &blueprint.segments[segment_index];
        let from = blueprint.rooms[segment.from].top;
        let to = blueprint.rooms[segment.to].top;
        let forward = direction_from_delta(to - from);
        let right = Vec3::new(-forward.z, 0.0, forward.x).normalize_or_zero();
        let player_position = from.lerp(to, 0.68) + right * 140.0 + Vec3::Y * 6.0;
        let focus = stream_focus_room(&blueprint, checkpoint, segment.to, player_position);

        assert!(
            focus >= segment.to.saturating_sub(1),
            "wide lateral position near segment {} -> {} regressed focus room to {}",
            segment.from,
            segment.to,
            focus
        );
    }

    #[test]
    fn sync_generated_geometry_does_not_raise_death_plane() {
        let blueprint = build_run_blueprint(0x51de_7001_u64);
        let mut run = RunState::new(&blueprint, build_run_snapshot(HashSet::new()));
        run.focus_room = 6.min(run.blueprint.rooms.len().saturating_sub(1));
        run.death_plane =
            checkpoint_death_plane(&run.blueprint, run.current_checkpoint, run.focus_room) - 72.0;
        let prior_death_plane = run.death_plane;

        run.sync_generated_geometry();

        assert!(
            run.death_plane <= prior_death_plane,
            "sync raised death plane from {} to {}",
            prior_death_plane,
            run.death_plane
        );
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

#[cfg(test)]
fn surf_wedge_surface_normal(size: Vec3, wall_side: f32) -> Vec3 {
    Vec3::new(0.0, size.z, wall_side * size.y).normalize_or_zero()
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
    let point_count = render_world_points.len();
    for point in render_world_points.iter().copied() {
        min = min.min(point);
        max = max.max(point);
        center += point;
    }
    center /= point_count as f32;

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

    let aligned_abdc = split_abdc_1.dot(split_abdc_2);
    let aligned_abcd = split_abcd_1.dot(split_abcd_2);
    aligned_abdc >= aligned_abcd
}

fn append_star_render_geometry(
    builder: &mut ColoredMeshBuilder,
    center: Vec3,
    outward: Vec3,
    size: f32,
    color: Color,
) {
    let outward = if outward == Vec3::ZERO {
        Vec3::Y
    } else {
        outward.normalize_or_zero()
    };
    let tangent = if outward.y.abs() < 0.95 {
        outward.cross(Vec3::Y).normalize_or_zero()
    } else {
        outward.cross(Vec3::X).normalize_or_zero()
    };
    let bitangent = outward.cross(tangent).normalize_or_zero();
    let top = center + bitangent * size;
    let bottom = center - bitangent * size;
    let left = center - tangent * size;
    let right = center + tangent * size;
    let front = center + outward * size * 0.55;
    let back = center - outward * size * 0.55;

    builder.push_triangle(top, right, front, color);
    builder.push_triangle(top, front, left, color);
    builder.push_triangle(top, left, back, color);
    builder.push_triangle(top, back, right, color);
    builder.push_triangle(bottom, front, right, color);
    builder.push_triangle(bottom, left, front, color);
    builder.push_triangle(bottom, back, left, color);
    builder.push_triangle(bottom, right, back, color);
}

#[cfg(test)]
fn surf_ramp_rotation(forward: Vec3) -> Quat {
    let forward = forward.normalize_or_zero();
    let right = Vec3::new(-forward.z, 0.0, forward.x);
    Quat::from_mat3(&Mat3::from_cols(forward, Vec3::Y, right))
}

fn transformed_point_bounds(origin: Vec3, rotation: Quat, points: &[Vec3]) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for point in points {
        let world = origin + rotation * *point;
        min = min.min(world);
        max = max.max(world);
    }
    (min, max)
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
