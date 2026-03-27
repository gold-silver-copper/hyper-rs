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
const WALL_RUN_FALL_SPEED: f32 = 2.25;
const WALL_RUN_MIN_SPEED: f32 = 4.0;
const WALL_RUN_DURATION: f32 = 0.95;
const WALL_RUN_COOLDOWN: f32 = 0.2;
const WALL_SHAFT_BOOST_SPEED: f32 = 8.8;
const WALL_SHAFT_REPEAT: f32 = 0.11;
const TREASURE_PICKUP_RADIUS: f32 = 1.8;
const CHECKPOINT_RADIUS: f32 = 2.8;
const SUMMIT_RADIUS: f32 = 4.5;
const SHORTCUT_TRIGGER_RADIUS: f32 = 2.0;
const SKY_RADIUS: f32 = 950.0;
const MAX_SECTION_TURN_RADIANS: f32 = 4.9_f32.to_radians();
const STAR_COUNT: usize = 1100;
const STAR_CLUSTER_COUNT: usize = 10;
const COMET_COUNT: usize = 6;
const CLOUD_PUFF_COUNT: usize = 18;
const STREAM_BEHIND_ROOMS: usize = 2;
const STREAM_AHEAD_ROOMS: usize = 12;
const PHYSICS_SUBSTEPS: u32 = 12;
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
        ))
        .add_input_context::<PlayerInput>()
        .insert_resource(ClearColor(Color::srgb(0.07, 0.078, 0.15)))
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
                tick_run_timer,
                queue_run_controls,
                activate_checkpoints,
                collect_treasures,
                activate_shortcuts,
                sync_shortcut_bridges,
                detect_summit_completion,
                detect_failures,
                animate_sky_decor,
                evolve_atmosphere_with_progress,
                stream_world_chunks,
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
            (
                normalize_surfing_motion.before(AhoySystems::MoveCharacters),
                apply_wall_run.after(AhoySystems::MoveCharacters),
            ),
        )
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
    bloom.intensity = 0.44;
    bloom.low_frequency_boost = 0.88;
    bloom.low_frequency_boost_curvature = 0.8;
    bloom.high_pass_frequency = 0.58;
    bloom.prefilter.threshold = 0.32;
    bloom.prefilter.threshold_softness = 0.28;
    bloom
}

fn night_color_grading() -> ColorGrading {
    ColorGrading::with_identical_sections(
        ColorGradingGlobal {
            exposure: 0.62,
            post_saturation: 1.1,
            ..default()
        },
        ColorGradingSection {
            saturation: 1.08,
            contrast: 1.0,
            ..default()
        },
    )
}

fn night_distance_fog() -> DistanceFog {
    DistanceFog {
        color: Color::srgba(0.1, 0.09, 0.16, 1.0),
        directional_light_color: Color::srgba(0.92, 0.78, 0.88, 0.34),
        directional_light_exponent: 10.0,
        falloff: FogFalloff::from_visibility_colors(
            2600.0,
            Color::srgb(0.1, 0.09, 0.15),
            Color::srgb(0.34, 0.28, 0.42),
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
        color: Color::srgb(0.2, 0.18, 0.24),
        brightness: 22.0,
        affects_lightmapped_meshes: true,
    });

    let blueprint = build_run_blueprint(current_run_seed());
    let initial_look = respawn_look_for_checkpoint(&blueprint, 0);
    let snapshot = spawn_run_world(
        &blueprint,
        &HashSet::default(),
        &HashSet::default(),
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
        Msaa::Off,
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
    let start_height = run
        .checkpoints
        .first()
        .map(|checkpoint| checkpoint.y)
        .unwrap_or(current_height);
    let finish_height = run.summit.y;
    let total_descent = (start_height - finish_height).max(1.0);
    let descended = (start_height - current_height).clamp(0.0, total_descent);
    let progress = (descended / total_descent).clamp(0.0, 1.0) * 100.0;
    let elapsed = run.timer.elapsed_secs();

    hud.0 = format!(
        "Chronoclimb\n\
         Seed: {seed:016x}\n\
         Floors: {floors} | Checkpoint: {checkpoint}/{checkpoint_total}\n\
         Altitude: {height:.1}m -> {finish:.1}m | Descent {descended:.1}m / {total_descent:.1}m ({progress:.0}%)\n\
         Speed: {speed:.1} u/s\n\
         Time: {elapsed:.1}s | Deaths: {deaths}\n\
         Treasures: {treasures}/{treasure_total} | Shortcuts: {shortcuts}\n\
         Gen: attempts {attempts}, repairs {repairs}, overlaps {overlaps}, clearance {clearance}, reach {reach}",
        seed = run.seed,
        floors = run.floors,
        checkpoint = run.current_checkpoint + 1,
        checkpoint_total = run.checkpoints.len(),
        height = current_height,
        finish = finish_height,
        descended = descended,
        total_descent = total_descent,
        progress = progress,
        speed = speed,
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
    );
}

fn run_descent_progress(run: &RunState, current_height: f32) -> f32 {
    let start_height = run.blueprint.rooms.first().map_or(0.0, |room| room.top.y);
    let finish_height = run
        .blueprint
        .rooms
        .last()
        .map_or(run.summit.y, |room| room.top.y);
    let total_descent = (start_height - finish_height).max(1.0);
    ((start_height - current_height) / total_descent).clamp(0.0, 1.0)
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
    mut lights: Query<(&AtmosphereLightKind, &mut DirectionalLight)>,
    atmosphere_layers: Query<(&AtmosphereMaterialKind, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Ok(player) = players.single() else {
        return;
    };

    let progress = smoothstep01(run_descent_progress(&run, player.translation.y));
    clear_color.0 = mix_color(
        Color::srgb(0.1, 0.11, 0.2),
        Color::srgb(0.16, 0.1, 0.24),
        progress,
    );

    ambient.color = mix_color(
        Color::srgb(0.18, 0.17, 0.24),
        Color::srgb(0.22, 0.18, 0.28),
        progress,
    );
    ambient.brightness = lerp(24.0, 18.0, progress);

    if let Some(camera) = camera {
        let (mut fog, mut grading, mut bloom) = camera.into_inner();
        fog.color = mix_color(
            Color::srgba(0.12, 0.11, 0.2, 1.0),
            Color::srgba(0.16, 0.1, 0.24, 1.0),
            progress,
        );
        fog.directional_light_color = mix_color(
            Color::srgba(0.98, 0.74, 0.8, 0.34),
            Color::srgba(0.76, 0.8, 1.0, 0.42),
            progress,
        );
        fog.directional_light_exponent = lerp(8.5, 11.5, progress);
        fog.falloff = FogFalloff::from_visibility_colors(
            lerp(3200.0, 2400.0, progress),
            mix_color(
                Color::srgb(0.16, 0.14, 0.22),
                Color::srgb(0.1, 0.08, 0.16),
                progress,
            ),
            mix_color(
                Color::srgb(0.4, 0.3, 0.42),
                Color::srgb(0.32, 0.22, 0.4),
                progress,
            ),
        );
        grading.global.exposure = lerp(0.7, 0.58, progress);
        grading.global.post_saturation = lerp(1.02, 1.12, progress);
        bloom.intensity = lerp(0.28, 0.4, progress);
        bloom.low_frequency_boost = lerp(0.7, 0.92, progress);
        bloom.prefilter.threshold = lerp(0.42, 0.3, progress);
        bloom.prefilter.threshold_softness = lerp(0.22, 0.3, progress);
    }

    for (kind, mut light) in &mut lights {
        match kind {
            AtmosphereLightKind::MoonKey => {
                light.illuminance = lerp(20_000.0, 26_000.0, progress);
                light.color = mix_color(
                    Color::srgb(0.92, 0.8, 0.88),
                    Color::srgb(0.72, 0.82, 1.0),
                    progress,
                );
            }
            AtmosphereLightKind::StarlightFill => {
                light.illuminance = lerp(10_000.0, 6_800.0, progress);
                light.color = mix_color(
                    Color::srgb(0.62, 0.54, 0.84),
                    Color::srgb(0.46, 0.62, 0.96),
                    progress,
                );
            }
        }
    }

    for (kind, material_handle) in &atmosphere_layers {
        let Some(material) = materials.get_mut(&material_handle.0) else {
            continue;
        };

        match kind {
            AtmosphereMaterialKind::SkyDome => {
                material.base_color = mix_color(
                    Color::srgb(0.09, 0.1, 0.18),
                    Color::srgb(0.13, 0.1, 0.2),
                    progress,
                );
                material.emissive = LinearRgba::from(mix_color(
                    Color::srgb(0.18, 0.17, 0.28),
                    Color::srgb(0.26, 0.18, 0.34),
                    progress,
                )) * 0.95;
            }
            AtmosphereMaterialKind::UpperHaze => {
                material.base_color = mix_color(
                    Color::srgba(0.26, 0.24, 0.38, 0.11),
                    Color::srgba(0.28, 0.18, 0.36, 0.12),
                    progress,
                );
                material.emissive = LinearRgba::from(mix_color(
                    Color::srgb(0.18, 0.16, 0.3),
                    Color::srgb(0.28, 0.18, 0.38),
                    progress,
                )) * 1.0;
            }
            AtmosphereMaterialKind::HorizonGlow => {
                material.base_color = mix_color(
                    Color::srgba(0.38, 0.28, 0.34, 0.16),
                    Color::srgba(0.34, 0.18, 0.38, 0.18),
                    progress,
                );
                material.emissive = LinearRgba::from(mix_color(
                    Color::srgb(0.58, 0.34, 0.42),
                    Color::srgb(0.46, 0.28, 0.56),
                    progress,
                )) * 1.08;
            }
            AtmosphereMaterialKind::Nebula => {
                material.base_color = mix_color(
                    Color::srgba(0.22, 0.18, 0.3, 0.1),
                    Color::srgba(0.26, 0.16, 0.34, 0.11),
                    progress,
                );
                material.emissive = LinearRgba::from(mix_color(
                    Color::srgb(0.34, 0.22, 0.36),
                    Color::srgb(0.46, 0.24, 0.42),
                    progress,
                )) * 0.92;
            }
            AtmosphereMaterialKind::Aurora => {
                material.emissive = LinearRgba::from(mix_color(
                    Color::srgb(0.16, 0.72, 0.82),
                    Color::srgb(0.34, 0.52, 0.96),
                    progress,
                )) * 1.06;
            }
            AtmosphereMaterialKind::CloudDeck => {
                material.base_color = mix_color(
                    Color::srgba(0.13, 0.1, 0.14, 0.17),
                    Color::srgba(0.18, 0.11, 0.18, 0.18),
                    progress,
                );
                material.emissive = LinearRgba::from(mix_color(
                    Color::srgb(0.08, 0.06, 0.1),
                    Color::srgb(0.12, 0.08, 0.14),
                    progress,
                )) * 0.8;
            }
            AtmosphereMaterialKind::Celestial => {
                material.emissive = LinearRgba::from(material.base_color.with_alpha(1.0))
                    * lerp(0.34, 0.62, progress);
            }
            AtmosphereMaterialKind::Megastructure => {
                material.emissive = LinearRgba::from(material.base_color.with_alpha(1.0))
                    * lerp(0.18, 0.34, progress);
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
            seed: run.seed,
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
                if let Some(spawn) = run.checkpoints.get(checkpoint.index).copied() {
                    spawn_marker.translation = spawn;
                    spawn_marker.rotation =
                        respawn_look_for_checkpoint(&run.blueprint, checkpoint.index).to_quat();
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
        .map(|player| stream_focus_room(&run.blueprint, run.current_checkpoint, player.translation))
        .unwrap_or(run.current_checkpoint);
    let desired_order = desired_chunk_window(&run.blueprint, focus_room);
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
                &run.collected_treasures,
                &run.unlocked_shortcuts,
                &mut commands,
                &mut meshes,
                &mut materials,
                &mut asset_cache,
            );
        }
    }

    run.spawned_chunks = desired_chunks;
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
                &run.collected_treasures,
                &run.unlocked_shortcuts,
                run.current_checkpoint,
                &mut commands,
                &mut meshes,
                &mut materials,
                &mut asset_cache,
            );
            run.spawned_chunks = snapshot.active_chunks;
        }
        RunRequestKind::RestartSameSeed | RunRequestKind::RestartNewSeed => {
            for entity in &generated {
                commands.entity(entity).despawn();
            }

            let blueprint = build_run_blueprint(request.seed);
            run.seed = request.seed;
            run.timer = Stopwatch::new();
            run.finished = false;
            run.deaths = 0;
            run.current_checkpoint = 0;
            run.collected_treasures.clear();
            run.unlocked_shortcuts.clear();

            let snapshot = spawn_run_world(
                &blueprint,
                &run.collected_treasures,
                &run.unlocked_shortcuts,
                0,
                &mut commands,
                &mut meshes,
                &mut materials,
                &mut asset_cache,
            );

            run.apply_blueprint(&blueprint, snapshot);
        }
    }

    let spawn = run
        .checkpoints
        .get(run.current_checkpoint)
        .copied()
        .unwrap_or(run.blueprint.spawn);
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
    UpperHaze,
    HorizonGlow,
    Nebula,
    Aurora,
    CloudDeck,
    Celestial,
    Megastructure,
}

#[derive(Component, Clone, Copy)]
enum AtmosphereLightKind {
    MoonKey,
    StarlightFill,
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
    Branch(usize),
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
    seed: u64,
    blueprint: RunBlueprint,
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
    spawned_chunks: HashSet<WorldChunkKey>,
    stats: GenerationStats,
}

impl RunState {
    fn new(blueprint: &RunBlueprint, snapshot: RunSnapshot) -> Self {
        let mut state = Self {
            seed: blueprint.seed,
            blueprint: blueprint.clone(),
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
            spawned_chunks: snapshot.active_chunks,
            stats: blueprint.stats.clone(),
        };
        if state.checkpoints.is_empty() {
            state.checkpoints.push(blueprint.spawn);
        }
        state
    }

    fn apply_blueprint(&mut self, blueprint: &RunBlueprint, snapshot: RunSnapshot) {
        self.seed = blueprint.seed;
        self.blueprint = blueprint.clone();
        self.floors = blueprint.floors;
        self.summit = blueprint.summit;
        self.death_plane = blueprint.death_plane;
        self.checkpoints = snapshot.checkpoints;
        self.total_treasures = snapshot.total_treasures;
        self.spawned_chunks = snapshot.active_chunks;
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
    seed: u64,
    section: RoomSectionKind,
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
    dir: Vec3,
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
    SurfRamp,
    MantleStack,
    WallRunHall,
    LiftChasm,
    CrumbleBridge,
    WindTunnel,
    IceSpine,
    WaterGarden,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BranchKind {
    TreasureAlcove,
    PropCache,
    ShortcutLever,
    RiskDetour,
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
    StaticSurfWedge {
        #[cfg_attr(not(test), allow(dead_code))]
        wall_side: f32,
        local_points: Vec<Vec3>,
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
    ShortcutBridge {
        id: u64,
        active: bool,
    },
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

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
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
    let floors = rng.range_usize(34, 49);
    let mut rooms = Vec::with_capacity(floors);
    let mut segments = Vec::with_capacity(floors.saturating_sub(1));
    let mut occupied_rooms: HashSet<IVec2> = HashSet::default();
    let mut occupied_branches = HashSet::default();
    let theme_offset = (rng.next_u64() % 4) as usize;

    let mut current_socket = SOCKET_SAFE_REST;
    let mut current_height = 420.0;
    let mut current_top = Vec3::new(0.0, current_height, 0.0);
    let heading_phase = rng.range_f32(0.0, TAU);
    let heading_frequency = rng.range_f32(0.42, 0.74);
    let spiral_direction = if rng.chance(0.5) { 1.0 } else { -1.0 };
    let spiral_turn = rng.range_f32(3.2_f32.to_radians(), 4.75_f32.to_radians()) * spiral_direction;
    let mut heading_angle = rng.range_f32(0.0, TAU);

    let cell = room_grid_cell(current_top);
    occupied_rooms.insert(cell);
    rooms.push(RoomPlan {
        index: 0,
        cell,
        top: current_top,
        size: Vec2::splat(13.5),
        theme: theme_for(0, floors, theme_offset),
        seed: rng.next_u64(),
        section: RoomSectionKind::OpenPad,
        checkpoint_slot: Some(0),
    });

    for index in 1..floors {
        let difficulty = index as f32 / (floors.saturating_sub(1).max(1)) as f32;
        let room_size = Vec2::splat(lerp(13.2, 9.8, difficulty)).max(Vec2::splat(9.2));
        let turn_wave =
            (index as f32 * heading_frequency + heading_phase).sin() * 0.72_f32.to_radians();
        let turn_jitter = rng.range_f32(-0.25_f32.to_radians(), 0.25_f32.to_radians());
        heading_angle += (spiral_turn + turn_wave + turn_jitter)
            .clamp(-MAX_SECTION_TURN_RADIANS, MAX_SECTION_TURN_RADIANS);
        let heading = Vec3::new(heading_angle.cos(), 0.0, heading_angle.sin()).normalize_or_zero();
        let mut step_distance = rng.range_f32(CELL_SIZE * 5.8, CELL_SIZE * 8.4);
        let projected_gap = projected_gap(step_distance, rooms.last().unwrap().size, room_size);
        let template = choose_module_template(rng, current_socket, difficulty, projected_gap);
        step_distance = match template.kind {
            ModuleKind::SurfRamp => rng.range_f32(CELL_SIZE * 14.0, CELL_SIZE * 22.0),
            ModuleKind::StairRun => rng.range_f32(CELL_SIZE * 9.0, CELL_SIZE * 15.0),
            ModuleKind::IceSpine | ModuleKind::CrumbleBridge | ModuleKind::WindTunnel => {
                rng.range_f32(CELL_SIZE * 7.0, CELL_SIZE * 11.5)
            }
            ModuleKind::WallRunHall => rng.range_f32(CELL_SIZE * 6.0, CELL_SIZE * 9.0),
            ModuleKind::MantleStack | ModuleKind::LiftChasm | ModuleKind::WaterGarden => {
                rng.range_f32(CELL_SIZE * 5.0, CELL_SIZE * 8.0)
            }
        };
        let descent = rng.range_f32(template.min_rise, template.max_rise)
            + lerp(8.5, 15.0, difficulty)
            + difficulty * 4.8;
        current_height -= descent;
        let right = Vec3::new(-heading.z, 0.0, heading.x);
        let bend_scale = if matches!(template.kind, ModuleKind::SurfRamp | ModuleKind::StairRun) {
            0.04 + difficulty * 0.04
        } else if matches!(
            template.kind,
            ModuleKind::IceSpine | ModuleKind::CrumbleBridge | ModuleKind::WindTunnel
        ) {
            0.08 + difficulty * 0.08
        } else {
            0.16 + difficulty * 0.14
        };
        let bend = right * rng.range_f32(-1.0, 1.0) * bend_scale * 1.25;
        let mut top = Vec3::new(current_top.x, current_height, current_top.z)
            + heading * step_distance
            + bend;
        top.y = current_height;
        let mut cell = room_grid_cell(top);
        let mut retries = 0;
        while occupied_rooms.contains(&cell) && retries < 6 {
            top += heading * (ROOM_GRID_SIZE * 0.75);
            cell = room_grid_cell(top);
            retries += 1;
        }
        occupied_rooms.insert(cell);
        current_top = top;

        if matches!(template.kind, ModuleKind::SurfRamp) {
            if let Some(previous_room) = rooms.last_mut() {
                previous_room.section = RoomSectionKind::OpenPad;
                previous_room.size = previous_room.size.max(Vec2::splat(15.4));
            }
        }

        rooms.push(RoomPlan {
            index,
            cell,
            top,
            size: if index == floors - 1 {
                Vec2::splat(15.2)
            } else if matches!(template.kind, ModuleKind::SurfRamp) {
                room_size.max(Vec2::splat(15.4))
            } else {
                room_size
            },
            theme: theme_for(index, floors, theme_offset),
            seed: rng.next_u64(),
            section: if matches!(template.kind, ModuleKind::SurfRamp) {
                RoomSectionKind::OpenPad
            } else {
                choose_room_section(rng, difficulty, index, floors)
            },
            checkpoint_slot: Some(index),
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
    let death_plane = rooms.last().unwrap().top.y - 90.0;

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
            ModuleKind::SurfRamp
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
        let adjacent_to_surf = segments
            .get(room.index.saturating_sub(1))
            .is_some_and(|segment| matches!(segment.kind, ModuleKind::SurfRamp))
            || segments
                .get(room.index)
                .is_some_and(|segment| matches!(segment.kind, ModuleKind::SurfRamp));
        if adjacent_to_surf {
            continue;
        }
        let difficulty = room.index as f32 / rooms.len().max(1) as f32;
        let main_forward = if room.index < rooms.len() - 1 {
            direction_from_delta(rooms[room.index + 1].top - room.top)
        } else {
            direction_from_delta(room.top - rooms[room.index - 1].top)
        };
        if main_forward == Vec3::ZERO {
            continue;
        }
        let branch_dirs = [
            Vec3::new(-main_forward.z, 0.0, main_forward.x),
            Vec3::new(main_forward.z, 0.0, -main_forward.x),
        ];

        for dir in branch_dirs {
            let branch_cell = room_grid_cell(room.top + dir * (CELL_SIZE * 0.72));
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

            let branch_top =
                room.top + dir * (CELL_SIZE * 0.68) + Vec3::Y * rng.range_f32(0.2, 1.8);
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
            SolidBody::StaticSurfWedge { local_points, .. } => {
                let (min, max) =
                    transformed_point_bounds(self.center, Quat::IDENTITY, local_points);
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
        if difficulty > 0.35 && matches!(template.kind, ModuleKind::StairRun) {
            weight += 4;
        }
        if difficulty < 0.35
            && matches!(
                template.kind,
                ModuleKind::SurfRamp | ModuleKind::MantleStack | ModuleKind::WaterGarden
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

fn all_templates() -> [ModuleTemplate; 11] {
    [
        module_template(ModuleKind::StairRun),
        module_template(ModuleKind::SurfRamp),
        module_template(ModuleKind::MantleStack),
        module_template(ModuleKind::WallRunHall),
        module_template(ModuleKind::LiftChasm),
        module_template(ModuleKind::CrumbleBridge),
        module_template(ModuleKind::WindTunnel),
        module_template(ModuleKind::IceSpine),
        module_template(ModuleKind::WaterGarden),
        module_template(ModuleKind::StairRun),
        module_template(ModuleKind::SurfRamp),
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
            shortcut_eligible: false,
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
            shortcut_eligible: false,
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
            shortcut_eligible: false,
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
            shortcut_eligible: true,
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
            shortcut_eligible: true,
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
            shortcut_eligible: true,
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
            shortcut_eligible: true,
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
            shortcut_eligible: false,
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
            shortcut_eligible: false,
        },
    }
}

fn safe_fallback_kind(difficulty: f32) -> ModuleKind {
    if difficulty > 0.4 {
        ModuleKind::StairRun
    } else {
        ModuleKind::SurfRamp
    }
}

fn spawn_run_world(
    blueprint: &RunBlueprint,
    collected_treasures: &HashSet<u64>,
    unlocked_shortcuts: &HashSet<u64>,
    checkpoint_index: usize,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_cache: &mut WorldAssetCache,
) -> RunSnapshot {
    spawn_sky_backdrop(blueprint, commands, meshes, materials);
    spawn_floating_spheres(blueprint, commands, meshes, materials);
    spawn_macro_spectacle(blueprint, commands, meshes, materials);

    let chunk_order = desired_chunk_window(blueprint, checkpoint_index);
    for chunk in &chunk_order {
        spawn_chunk(
            *chunk,
            blueprint,
            collected_treasures,
            unlocked_shortcuts,
            commands,
            meshes,
            materials,
            asset_cache,
        );
    }

    build_run_snapshot(blueprint, chunk_order.into_iter().collect())
}

fn respawn_active_chunks(
    blueprint: &RunBlueprint,
    collected_treasures: &HashSet<u64>,
    unlocked_shortcuts: &HashSet<u64>,
    checkpoint_index: usize,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_cache: &mut WorldAssetCache,
) -> RunSnapshot {
    let chunk_order = desired_chunk_window(blueprint, checkpoint_index);
    for chunk in &chunk_order {
        spawn_chunk(
            *chunk,
            blueprint,
            collected_treasures,
            unlocked_shortcuts,
            commands,
            meshes,
            materials,
            asset_cache,
        );
    }

    build_run_snapshot(blueprint, chunk_order.into_iter().collect())
}

fn build_run_snapshot(
    blueprint: &RunBlueprint,
    active_chunks: HashSet<WorldChunkKey>,
) -> RunSnapshot {
    RunSnapshot {
        checkpoints: blueprint
            .rooms
            .iter()
            .filter(|room| room.checkpoint_slot.is_some())
            .map(|room| room.top + Vec3::new(0.0, PLAYER_SPAWN_CLEARANCE, 0.0))
            .collect(),
        total_treasures: blueprint
            .branches
            .iter()
            .filter(|branch| branch.treasure_id.is_some())
            .count(),
        active_chunks,
    }
}

#[derive(Clone, Copy)]
struct StreamWindow {
    start_room: usize,
    frontier_room: usize,
}

fn stream_focus_room(
    blueprint: &RunBlueprint,
    checkpoint_index: usize,
    player_position: Vec3,
) -> usize {
    let last_room = blueprint.rooms.len().saturating_sub(1);
    let checkpoint_index = checkpoint_index.min(last_room);
    let search_start = checkpoint_index.saturating_sub(1);
    let search_end = (checkpoint_index + STREAM_AHEAD_ROOMS + 4).min(last_room);
    let mut best_room = checkpoint_index;
    let mut best_score = f32::INFINITY;

    for room in &blueprint.rooms[search_start..=search_end] {
        let offset = room.top - player_position;
        let horizontal = offset.xz().length();
        let vertical = offset.y.abs() * 0.3;
        let progression_bias = (room.index.saturating_sub(checkpoint_index)) as f32 * 1.8;
        let score = horizontal + vertical + progression_bias;
        if score < best_score {
            best_score = score;
            best_room = room.index;
        }
    }

    best_room.max(checkpoint_index)
}

fn stream_window(blueprint: &RunBlueprint, focus_room: usize) -> StreamWindow {
    let last_room = blueprint.rooms.len().saturating_sub(1);
    let focus_room = focus_room.min(last_room);
    let start_room = focus_room.saturating_sub(STREAM_BEHIND_ROOMS);
    let end_room = (focus_room + STREAM_AHEAD_ROOMS).min(last_room);
    let frontier_room = (end_room + 1).min(last_room);

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
            && segment.from <= window.frontier_room
        {
            chunks.push(WorldChunkKey::Segment(segment.index));
        }
    }
    for (branch_index, branch) in blueprint.branches.iter().enumerate() {
        if (window.start_room..=window.frontier_room).contains(&branch.room_index) {
            chunks.push(WorldChunkKey::Branch(branch_index));
        }
    }
    if window.frontier_room + 1 == blueprint.rooms.len() {
        chunks.push(WorldChunkKey::Summit);
    }

    chunks
}

fn spawn_chunk(
    chunk: WorldChunkKey,
    blueprint: &RunBlueprint,
    collected_treasures: &HashSet<u64>,
    unlocked_shortcuts: &HashSet<u64>,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_cache: &mut WorldAssetCache,
) {
    match chunk {
        WorldChunkKey::Room(index) => {
            if let Some(room) = blueprint.rooms.get(index) {
                let layout = build_room_layout(room);
                let _ = spawn_layout(
                    &layout,
                    collected_treasures,
                    unlocked_shortcuts,
                    Some(chunk),
                    commands,
                    meshes,
                    materials,
                    asset_cache,
                );
            }
        }
        WorldChunkKey::Segment(index) => {
            if let Some(segment) = blueprint.segments.get(index) {
                let layout = build_segment_layout(segment, &blueprint.rooms, unlocked_shortcuts);
                let _ = spawn_layout(
                    &layout,
                    collected_treasures,
                    unlocked_shortcuts,
                    Some(chunk),
                    commands,
                    meshes,
                    materials,
                    asset_cache,
                );
            }
        }
        WorldChunkKey::Branch(index) => {
            if let Some(branch) = blueprint.branches.get(index) {
                let layout =
                    build_branch_layout(index, branch, &blueprint.rooms, unlocked_shortcuts);
                let _ = spawn_layout(
                    &layout,
                    collected_treasures,
                    unlocked_shortcuts,
                    Some(chunk),
                    commands,
                    meshes,
                    materials,
                    asset_cache,
                );
            }
        }
        WorldChunkKey::Summit => {
            if let Some(room) = blueprint.rooms.last() {
                let layout = build_summit_layout(room, blueprint.summit);
                let _ = spawn_layout(
                    &layout,
                    collected_treasures,
                    unlocked_shortcuts,
                    Some(chunk),
                    commands,
                    meshes,
                    materials,
                    asset_cache,
                );
            }
        }
    }
}

fn spawn_sky_backdrop(
    blueprint: &RunBlueprint,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let (center, course_radius) = course_visual_center_and_radius(blueprint);
    let mut sky_rng = RunRng::new(blueprint.seed ^ 0xC1A0_DA7A_55AA_9911);
    let moon_heading = if blueprint.rooms.len() > 1 {
        let mut heading = blueprint.rooms[1].top - blueprint.rooms[0].top;
        heading.y = 0.0;
        heading.normalize_or_zero()
    } else {
        Vec3::new(0.0, 0.0, -1.0)
    };
    let moon_right = Vec3::new(-moon_heading.z, 0.0, moon_heading.x).normalize_or_zero();

    let sky_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.08, 0.09, 0.17),
        emissive: LinearRgba::rgb(0.22, 0.2, 0.34),
        unlit: true,
        cull_mode: None,
        ..default()
    });
    let upper_haze_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.24, 0.22, 0.38, 0.1),
        emissive: LinearRgba::rgb(0.18, 0.16, 0.32),
        unlit: true,
        cull_mode: None,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    let horizon_glow_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.32, 0.24, 0.34, 0.16),
        emissive: LinearRgba::rgb(0.52, 0.32, 0.46),
        unlit: true,
        cull_mode: None,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    let nebula_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.22, 0.18, 0.32, 0.1),
        emissive: LinearRgba::rgb(0.34, 0.22, 0.36),
        unlit: true,
        cull_mode: None,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    let aurora_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.18, 0.7, 0.74, 0.05),
        emissive: LinearRgba::rgb(0.18, 0.78, 0.86),
        unlit: true,
        cull_mode: None,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    let aurora_secondary_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.1, 0.34, 0.86, 0.035),
        emissive: LinearRgba::rgb(0.08, 0.24, 0.72),
        unlit: true,
        cull_mode: None,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    let cloud_deck_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.12, 0.1, 0.14, 0.18),
        emissive: LinearRgba::rgb(0.06, 0.05, 0.08),
        unlit: true,
        cull_mode: None,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    let cloud_puff_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.14, 0.12, 0.18, 0.12),
        emissive: LinearRgba::rgb(0.07, 0.06, 0.11),
        unlit: true,
        cull_mode: None,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    let star_mesh = meshes.add(Sphere::new(0.8).mesh().ico(3).unwrap());
    let dome_mesh = meshes.add(Sphere::new(1.0).mesh().ico(6).unwrap());
    let cloud_mesh = meshes.add(Sphere::new(1.0).mesh().ico(4).unwrap());
    let comet_mesh = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let aurora_ring_mesh = meshes.add(Torus {
        minor_radius: 0.075,
        major_radius: 1.0,
    });
    let star_material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        emissive: LinearRgba::rgb(2.8, 3.1, 4.2),
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
        AtmosphereMaterialKind::UpperHaze,
        Name::new("Upper Haze"),
        Mesh3d(dome_mesh),
        MeshMaterial3d(upper_haze_material),
        Transform::from_translation(center + Vec3::Y * 80.0)
            .with_scale(Vec3::splat(SKY_RADIUS * 0.82)),
        NotShadowCaster,
        NotShadowReceiver,
    ));
    commands.spawn((
        GeneratedWorld,
        AtmosphereMaterialKind::HorizonGlow,
        Name::new("Horizon Glow"),
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().ico(5).unwrap())),
        MeshMaterial3d(horizon_glow_material),
        Transform::from_translation(center + Vec3::new(0.0, 42.0, 0.0)).with_scale(Vec3::new(
            SKY_RADIUS * 0.92,
            SKY_RADIUS * 0.28,
            SKY_RADIUS * 0.92,
        )),
        NotShadowCaster,
        NotShadowReceiver,
    ));
    commands.spawn((
        GeneratedWorld,
        AtmosphereMaterialKind::Nebula,
        Name::new("Nebula Veil"),
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().ico(5).unwrap())),
        MeshMaterial3d(nebula_material),
        Transform::from_translation(center + Vec3::new(-140.0, 210.0, 190.0))
            .with_rotation(Quat::from_rotation_z(-0.34) * Quat::from_rotation_x(0.08))
            .with_scale(Vec3::new(
                SKY_RADIUS * 0.78,
                SKY_RADIUS * 0.2,
                SKY_RADIUS * 0.64,
            )),
        NotShadowCaster,
        NotShadowReceiver,
    ));
    commands.spawn((
        GeneratedWorld,
        AtmosphereMaterialKind::Aurora,
        Name::new("Aurora Arc"),
        Mesh3d(aurora_ring_mesh.clone()),
        MeshMaterial3d(aurora_material),
        Transform::from_translation(center + Vec3::new(0.0, 190.0, 0.0))
            .with_rotation(
                Quat::from_rotation_x(1.1)
                    * Quat::from_rotation_y(0.35)
                    * Quat::from_rotation_z(0.12),
            )
            .with_scale(Vec3::new(
                course_radius + 540.0,
                42.0,
                course_radius + 540.0,
            )),
        NotShadowCaster,
        NotShadowReceiver,
        SkyDrift {
            anchor: center + Vec3::new(0.0, 190.0, 0.0),
            primary_axis: Vec3::new(1.0, 0.0, 0.25).normalize_or_zero(),
            secondary_axis: Vec3::new(-0.2, 0.0, 1.0).normalize_or_zero(),
            primary_amplitude: 18.0,
            secondary_amplitude: 10.0,
            vertical_amplitude: 6.0,
            speed: 0.025,
            rotation_speed: 0.004,
            phase: sky_rng.range_f32(0.0, TAU),
            base_rotation: Quat::from_rotation_x(1.1)
                * Quat::from_rotation_y(0.35)
                * Quat::from_rotation_z(0.12),
        },
    ));
    commands.spawn((
        GeneratedWorld,
        AtmosphereMaterialKind::Aurora,
        Name::new("Aurora Echo"),
        Mesh3d(aurora_ring_mesh),
        MeshMaterial3d(aurora_secondary_material),
        Transform::from_translation(center + Vec3::new(120.0, 240.0, -80.0))
            .with_rotation(
                Quat::from_rotation_x(1.24)
                    * Quat::from_rotation_y(-0.42)
                    * Quat::from_rotation_z(-0.08),
            )
            .with_scale(Vec3::new(
                course_radius + 680.0,
                30.0,
                course_radius + 680.0,
            )),
        NotShadowCaster,
        NotShadowReceiver,
        SkyDrift {
            anchor: center + Vec3::new(120.0, 240.0, -80.0),
            primary_axis: Vec3::new(0.8, 0.0, -0.5).normalize_or_zero(),
            secondary_axis: Vec3::new(0.4, 0.0, 0.9).normalize_or_zero(),
            primary_amplitude: 22.0,
            secondary_amplitude: 12.0,
            vertical_amplitude: 8.0,
            speed: 0.02,
            rotation_speed: -0.003,
            phase: sky_rng.range_f32(0.0, TAU),
            base_rotation: Quat::from_rotation_x(1.24)
                * Quat::from_rotation_y(-0.42)
                * Quat::from_rotation_z(-0.08),
        },
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
    commands.spawn((
        GeneratedWorld,
        AtmosphereMaterialKind::CloudDeck,
        Name::new("Deep Fog Sea"),
        Mesh3d(cloud_mesh.clone()),
        MeshMaterial3d(cloud_deck_material.clone()),
        Transform::from_translation(Vec3::new(center.x, blueprint.death_plane - 92.0, center.z))
            .with_scale(Vec3::new(course_radius * 3.1, 52.0, course_radius * 3.1)),
        NotShadowCaster,
        NotShadowReceiver,
    ));

    for cloud_index in 0..CLOUD_PUFF_COUNT {
        let angle =
            TAU * (cloud_index as f32 / CLOUD_PUFF_COUNT as f32) + sky_rng.range_f32(-0.35, 0.35);
        let radial = Vec3::new(angle.cos(), 0.0, angle.sin()).normalize_or_zero();
        let tangent = Vec3::new(-radial.z, 0.0, radial.x);
        let anchor = Vec3::new(
            center.x,
            blueprint.death_plane - sky_rng.range_f32(16.0, 110.0),
            center.z,
        ) + radial * sky_rng.range_f32(course_radius * 0.2, course_radius * 1.8);
        let scale = Vec3::new(
            sky_rng.range_f32(26.0, 72.0),
            sky_rng.range_f32(5.0, 14.0),
            sky_rng.range_f32(20.0, 58.0),
        );
        commands.spawn((
            GeneratedWorld,
            AtmosphereMaterialKind::CloudDeck,
            Name::new("Cloud Puff"),
            Mesh3d(cloud_mesh.clone()),
            MeshMaterial3d(cloud_puff_material.clone()),
            Transform::from_translation(anchor)
                .with_rotation(Quat::from_rotation_y(sky_rng.range_f32(0.0, TAU)))
                .with_scale(scale),
            NotShadowCaster,
            NotShadowReceiver,
            SkyDrift {
                anchor,
                primary_axis: tangent,
                secondary_axis: radial,
                primary_amplitude: sky_rng.range_f32(7.0, 22.0),
                secondary_amplitude: sky_rng.range_f32(3.0, 9.0),
                vertical_amplitude: sky_rng.range_f32(0.8, 2.4),
                speed: sky_rng.range_f32(0.02, 0.06),
                rotation_speed: sky_rng.range_f32(-0.008, 0.008),
                phase: sky_rng.range_f32(0.0, TAU),
                base_rotation: Quat::from_rotation_y(sky_rng.range_f32(0.0, TAU)),
            },
        ));
    }

    let mut starfield = ColoredMeshBuilder::default();
    for star_index in 0..STAR_COUNT {
        let f = star_index as f32 / STAR_COUNT as f32;
        let theta = TAU * f * 21.0;
        let y = 1.0 - 2.0 * (star_index as f32 + 0.5) / STAR_COUNT as f32;
        let r = (1.0 - y * y).sqrt();
        let direction = Vec3::new(r * theta.cos(), y, r * theta.sin());
        let position = center + Vec3::Y * 120.0 + direction * (SKY_RADIUS * 0.9);
        let size = 0.12 + ((star_index * 37 % 17) as f32) * 0.022;
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
            + radial * sky_rng.range_f32(course_radius + 260.0, course_radius + 430.0)
            + Vec3::Y * sky_rng.range_f32(150.0, 280.0);
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
                sky_rng.range_f32(0.28, 0.9),
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
            + radial * sky_rng.range_f32(course_radius + 320.0, course_radius + 520.0)
            + Vec3::Y * sky_rng.range_f32(190.0, 320.0);
        let comet_tint = if comet_index % 2 == 0 {
            Color::linear_rgb(3.8, 4.8, 6.4)
        } else {
            Color::linear_rgb(5.0, 4.0, 5.4)
        };
        let tail_material = materials.add(StandardMaterial {
            base_color: comet_tint.with_alpha(0.08),
            emissive: LinearRgba::from(comet_tint) * 0.5,
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            ..default()
        });
        let core_material = materials.add(StandardMaterial {
            base_color: comet_tint,
            emissive: LinearRgba::from(comet_tint) * 1.0,
            unlit: true,
            ..default()
        });
        let tail_length = sky_rng.range_f32(24.0, 44.0);
        let tail_width = sky_rng.range_f32(1.0, 2.0);
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
        Name::new("Moonlight"),
        AtmosphereLightKind::MoonKey,
        Transform::from_xyz(
            blueprint.summit.x - 80.0,
            blueprint.summit.y + 140.0,
            blueprint.summit.z + 90.0,
        )
        .looking_at(center + Vec3::Y * 10.0, Vec3::Y),
        DirectionalLight {
            shadows_enabled: true,
            illuminance: 22_000.0,
            color: Color::srgb(0.76, 0.82, 1.0),
            ..default()
        },
        CascadeShadowConfigBuilder {
            first_cascade_far_bound: 32.0,
            maximum_distance: 240.0,
            overlap_proportion: 0.22,
            ..default()
        }
        .build(),
    ));
    commands.spawn((
        GeneratedWorld,
        Name::new("Starlight Fill"),
        AtmosphereLightKind::StarlightFill,
        Transform::from_xyz(
            blueprint.spawn.x + 40.0,
            blueprint.spawn.y + 120.0,
            blueprint.spawn.z - 60.0,
        )
        .looking_at(center + Vec3::Y * 20.0, Vec3::Y),
        DirectionalLight {
            shadows_enabled: false,
            illuminance: 8_000.0,
            color: Color::srgb(0.48, 0.6, 0.94),
            ..default()
        },
    ));
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
            center + moon_heading * (course_radius + 260.0) + moon_right * 120.0 + Vec3::Y * 310.0,
        )
        .with_scale(Vec3::splat(64.0)),
        NotShadowCaster,
        NotShadowReceiver,
    ));
    commands.spawn((
        GeneratedWorld,
        AtmosphereMaterialKind::Celestial,
        Name::new("Hero Moon Halo"),
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().ico(5).unwrap())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgba(0.7, 0.82, 1.0, 0.1),
            emissive: LinearRgba::rgb(2.8, 3.4, 5.0),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            ..default()
        })),
        Transform::from_translation(
            center + moon_heading * (course_radius + 260.0) + moon_right * 120.0 + Vec3::Y * 310.0,
        )
        .with_scale(Vec3::splat(82.0)),
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
            center.x - course_radius - 230.0,
            center.y + 260.0,
            center.z + course_radius + 180.0,
        )
        .with_scale(Vec3::splat(34.0)),
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
            center.x + course_radius + 280.0,
            center.y + 170.0,
            center.z - course_radius - 210.0,
        )
        .with_scale(Vec3::splat(18.0)),
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
            center - moon_right * (course_radius + 220.0) + moon_heading * 140.0 + Vec3::Y * 240.0,
        )
        .with_scale(Vec3::splat(22.0)),
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
            center + moon_right * (course_radius + 340.0) - moon_heading * 180.0 + Vec3::Y * 210.0,
        )
        .with_scale(Vec3::splat(14.0)),
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

#[derive(Clone, Copy)]
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
        emissive: LinearRgba::from(tint) * 0.18,
        reflectance: 0.7,
        specular_tint: tint,
        clearcoat: 0.55,
        clearcoat_perceptual_roughness: 0.24,
        metallic: 0.04,
        perceptual_roughness: 0.42,
        ..default()
    });
    let atmosphere_material = materials.add(StandardMaterial {
        base_color: tint.with_alpha(0.14),
        emissive: LinearRgba::from(tint) * 0.52,
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        ..default()
    });
    let core_material = materials.add(StandardMaterial {
        base_color: tint,
        emissive: LinearRgba::from(tint) * 0.34,
        reflectance: 0.92,
        metallic: 0.08,
        perceptual_roughness: 0.18,
        ..default()
    });
    let ring_material = materials.add(StandardMaterial {
        base_color: tint.with_alpha(0.18),
        emissive: LinearRgba::from(tint) * 0.5,
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        ..default()
    });
    let mut entity = commands.spawn((
        GeneratedWorld,
        AtmosphereMaterialKind::Celestial,
        Name::new("Floating Sphere"),
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
        parent.spawn((
            Name::new("Floating Sphere Core"),
            AtmosphereMaterialKind::Celestial,
            Mesh3d(mesh.clone()),
            MeshMaterial3d(core_material),
            Transform::from_scale(shape.halo_scale() * 0.9),
            NotShadowCaster,
            NotShadowReceiver,
        ));
        parent.spawn((
            Name::new("Floating Sphere Atmosphere"),
            AtmosphereMaterialKind::Celestial,
            Mesh3d(mesh.clone()),
            MeshMaterial3d(atmosphere_material),
            Transform::from_scale(shape.halo_scale() + Vec3::splat(radius / 110.0)),
            NotShadowCaster,
            NotShadowReceiver,
        ));

        if ringed {
            parent.spawn((
                Name::new("Floating Sphere Ring"),
                AtmosphereMaterialKind::Celestial,
                Mesh3d(ring_mesh),
                MeshMaterial3d(ring_material),
                Transform::from_rotation(Quat::from_rotation_x(1.2) * Quat::from_rotation_z(0.35))
                    .with_scale(Vec3::new(radius * 1.75, radius * 0.3, radius * 1.75)),
                NotShadowCaster,
                NotShadowReceiver,
            ));
        }

        if glows {
            parent.spawn((
                Name::new("Floating Sphere Light"),
                PointLight {
                    intensity: 100_000.0 + radius * radius * 1_300.0,
                    range: radius * 7.8,
                    color: tint,
                    shadows_enabled: false,
                    ..default()
                },
                Transform::default(),
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

fn build_celestial_body_plans(blueprint: &RunBlueprint) -> Vec<CelestialBodyPlan> {
    let (center, course_radius) = course_visual_center_and_radius(blueprint);
    let mut rng = RunRng::new(blueprint.seed ^ 0x5151_AAAA_9999_7777);
    let major_count = 12;
    let moon_count = 18;
    let decor_count = 28;
    let mut bodies = Vec::with_capacity(major_count + moon_count + decor_count);

    for body_index in 0..major_count {
        let angle = TAU * (body_index as f32 / major_count as f32) + rng.range_f32(-0.12, 0.12);
        let radial = Vec3::new(angle.cos(), 0.0, angle.sin()).normalize_or_zero();
        let tangent = Vec3::new(-radial.z, 0.0, radial.x);
        let radius = rng.range_f32(12.0, 24.0);
        let shape = CelestialShapeKind::Sphere;
        let clearance_radius = radius * shape.clearance_multiplier();
        let anchor = find_safe_celestial_anchor(
            blueprint,
            &mut rng,
            center,
            course_radius,
            radial,
            tangent,
            clearance_radius,
            true,
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
            primary_amplitude: rng.range_f32(6.0, 16.0),
            secondary_amplitude: rng.range_f32(2.5, 7.0),
            vertical_amplitude: 1.6 + radius * 0.1,
            speed: rng.range_f32(0.02, 0.05),
            rotation_speed: rng.range_f32(-0.006, 0.006),
            phase: rng.range_f32(0.0, TAU),
            base_rotation: Quat::from_rotation_y(rng.range_f32(0.0, TAU)),
            tint: floating_sphere_color(body_index),
            glows: true,
            ringed: body_index % 3 == 0 || rng.range_f32(0.0, 1.0) > 0.72,
        });
    }

    for moon_index in 0..moon_count {
        let angle = TAU * (moon_index as f32 / moon_count as f32) + rng.range_f32(-0.16, 0.16);
        let radial = Vec3::new(angle.cos(), 0.0, angle.sin()).normalize_or_zero();
        let tangent = Vec3::new(-radial.z, 0.0, radial.x);
        let radius = rng.range_f32(5.5, 10.5);
        let shape = CelestialShapeKind::Sphere;
        let clearance_radius = radius * shape.clearance_multiplier();
        let anchor = find_safe_celestial_anchor(
            blueprint,
            &mut rng,
            center,
            course_radius,
            radial,
            tangent,
            clearance_radius,
            false,
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
            primary_amplitude: rng.range_f32(4.0, 10.0),
            secondary_amplitude: rng.range_f32(1.8, 4.4),
            vertical_amplitude: 1.0 + radius * 0.12,
            speed: rng.range_f32(0.03, 0.07),
            rotation_speed: rng.range_f32(-0.008, 0.008),
            phase: rng.range_f32(0.0, TAU),
            base_rotation: Quat::from_rotation_y(rng.range_f32(0.0, TAU)),
            tint: Color::srgb(
                rng.range_f32(0.58, 0.84),
                rng.range_f32(0.68, 0.9),
                rng.range_f32(0.88, 1.0),
            ),
            glows: true,
            ringed: false,
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
        let angle = TAU * (decor_index as f32 / decor_count as f32) + rng.range_f32(-0.08, 0.08);
        let radial = Vec3::new(angle.cos(), 0.0, angle.sin()).normalize_or_zero();
        let tangent = Vec3::new(-radial.z, 0.0, radial.x);
        let shape = decor_shapes[decor_index % decor_shapes.len()];
        let radius = rng.range_f32(4.0, 12.0);
        let clearance_radius = radius * shape.clearance_multiplier();
        let mut anchor = find_safe_celestial_anchor(
            blueprint,
            &mut rng,
            center,
            course_radius,
            radial,
            tangent,
            clearance_radius,
            false,
        ) + tangent * rng.range_f32(-42.0, 42.0);
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
            primary_amplitude: rng.range_f32(8.0, 24.0),
            secondary_amplitude: rng.range_f32(4.0, 12.0),
            vertical_amplitude: rng.range_f32(1.8, 6.4),
            speed: rng.range_f32(0.03, 0.08),
            rotation_speed: rng.range_f32(0.12, 0.38)
                * if decor_index % 2 == 0 { 1.0 } else { -1.0 },
            phase: rng.range_f32(0.0, TAU),
            base_rotation: Quat::from_euler(
                EulerRot::YXZ,
                rng.range_f32(0.0, TAU),
                rng.range_f32(0.0, TAU),
                rng.range_f32(0.0, TAU),
            ),
            tint: floating_sphere_color(decor_index + major_count + moon_count),
            glows: decor_index % 3 != 0,
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
    if blueprint.rooms.len() < 2 {
        return;
    }

    let (center, course_radius) = course_visual_center_and_radius(blueprint);
    let mut rng = RunRng::new(blueprint.seed ^ 0xA11C_E5C4_7A51_D00D);
    let entry_heading = direction_from_delta(blueprint.rooms[1].top - blueprint.rooms[0].top);
    let entry_right = Vec3::new(-entry_heading.z, 0.0, entry_heading.x).normalize_or_zero();
    let start_height = blueprint.rooms.first().unwrap().top.y + 180.0;
    let end_height = blueprint.rooms.last().unwrap().top.y - 120.0;

    let mut helix_builder = ColoredMeshBuilder::default();
    let helix_turns = rng.range_f32(2.2, 3.4) * if rng.chance(0.5) { 1.0 } else { -1.0 };
    let helix_phase = rng.range_f32(0.0, TAU);
    let helix_samples = 96;
    let helix_radius = course_radius + 165.0;
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
            let point = center + radial * radius + Vec3::Y * lerp(start_height, end_height, t);
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
                alpha_mode: AlphaMode::Blend,
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
    let mobius_center = center + Vec3::Y * ((start_height + end_height) * 0.5 + 120.0);
    let major_radius = course_radius * 0.82 + 220.0;
    let mobius_samples = 84;
    let mut previous = None;
    for sample in 0..=mobius_samples {
        let t = sample as f32 / mobius_samples as f32;
        let angle = TAU * t;
        let radial = Vec3::new(angle.cos(), 0.0, angle.sin()).normalize_or_zero();
        let point = mobius_center
            + radial * major_radius
            + entry_right * ((angle * 2.0).cos() * major_radius * 0.08)
            + Vec3::Y * ((angle * 2.0).sin() * major_radius * 0.08);
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
                alpha_mode: AlphaMode::Blend,
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
    let frame_center = center
        + entry_right * (course_radius + 220.0)
        + Vec3::Y * (blueprint.rooms[0].top.y + 140.0)
        - entry_heading * 48.0;
    let frame_normal = (center + Vec3::Y * 40.0 - frame_center).normalize_or_zero();
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
                alpha_mode: AlphaMode::Blend,
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
                alpha_mode: AlphaMode::Blend,
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
) -> Vec3 {
    let orbit_min = if major {
        course_radius + 180.0 + radius * 2.8
    } else {
        course_radius + 150.0 + radius * 2.1
    };
    let orbit_max = if major {
        course_radius + 360.0 + radius * 3.2
    } else {
        course_radius + 280.0 + radius * 2.6
    };
    let altitude_min = if major {
        120.0 + radius * 1.4
    } else {
        95.0 + radius
    };
    let altitude_max = if major {
        250.0 + radius * 1.8
    } else {
        190.0 + radius * 1.5
    };
    let tangential_span = if major { 56.0 } else { 40.0 };
    let desired_clearance = if major {
        140.0 + radius * 2.1
    } else {
        120.0 + radius * 1.8
    };

    let mut best = center + radial * orbit_max + Vec3::Y * altitude_min;
    let mut best_clearance = f32::NEG_INFINITY;

    for _ in 0..16 {
        let orbit = rng.range_f32(orbit_min, orbit_max);
        let candidate = center
            + radial * orbit
            + tangent * rng.range_f32(-tangential_span, tangential_span)
            + Vec3::Y * rng.range_f32(altitude_min, altitude_max);
        let clearance = celestial_course_clearance(blueprint, candidate, radius);
        if clearance > best_clearance {
            best = candidate;
            best_clearance = clearance;
        }
        if clearance >= desired_clearance {
            return candidate;
        }
    }

    if best_clearance < desired_clearance {
        let push = desired_clearance - best_clearance + 18.0;
        best += radial * push;
        best.y += push * 0.35;
    }

    best
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

fn spawn_layout(
    layout: &ModuleLayout,
    collected_treasures: &HashSet<u64>,
    unlocked_shortcuts: &HashSet<u64>,
    chunk: Option<WorldChunkKey>,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_cache: &mut WorldAssetCache,
) -> usize {
    let mut treasure_count = 0;
    let mut render_batches = HashMap::<GameMaterialKey, ColoredMeshBuilder>::default();
    for solid in &layout.solids {
        if matches!(solid.extra, ExtraKind::Treasure { .. }) {
            treasure_count += 1;
        }
        if is_batchable_static_render(solid) {
            append_static_render_geometry(&mut render_batches, solid);
            spawn_box_collider_spec(solid, commands, chunk);
        } else {
            spawn_box_spec(
                solid,
                collected_treasures,
                unlocked_shortcuts,
                chunk,
                commands,
                meshes,
                materials,
                asset_cache,
            );
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
        if let Some(chunk) = chunk {
            entity.insert(ChunkMember(chunk));
        }
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
                let mut entity = commands.spawn((
                    GeneratedWorld,
                    Name::new("Wind Zone"),
                    Mesh3d(cached_cuboid_mesh(asset_cache, meshes, *size)),
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

    treasure_count
}

fn spawn_box_spec(
    spec: &SolidSpec,
    collected_treasures: &HashSet<u64>,
    unlocked_shortcuts: &HashSet<u64>,
    chunk: Option<WorldChunkKey>,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_cache: &mut WorldAssetCache,
) {
    if let Err(reason) = validate_solid_spec(spec) {
        eprintln!("Skipping invalid solid '{}': {}", spec.label, reason);
        return;
    }

    if let ExtraKind::Treasure { id } = spec.extra {
        if collected_treasures.contains(&id) {
            return;
        }
    }

    let mesh = match &spec.body {
        SolidBody::StaticSurfWedge { local_points, .. } => meshes.add(build_surf_wedge_mesh(
            local_points,
            paint_base_color(
                spec.paint,
                matches!(&spec.body, SolidBody::ShortcutBridge { active: false, .. }),
            ),
            paint_stripe_color(spec.paint),
        )),
        _ => cached_cuboid_mesh(asset_cache, meshes, spec.size),
    };
    let material = cached_game_material(
        asset_cache,
        materials,
        GameMaterialKey {
            paint: spec.paint,
            ghost: matches!(&spec.body, SolidBody::ShortcutBridge { active: false, .. }),
            surf: matches!(&spec.body, SolidBody::StaticSurfWedge { .. }),
            vertex_colored: matches!(&spec.body, SolidBody::StaticSurfWedge { .. }),
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
        SolidBody::StaticSurfWedge { local_points, .. } => {
            if let Some(collider) = Collider::convex_hull(local_points.clone()) {
                entity.insert((
                    RigidBody::Static,
                    collider,
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
        && matches!(
            &spec.body,
            SolidBody::Static | SolidBody::StaticSurfWedge { .. }
        )
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
        SolidBody::StaticSurfWedge { local_points, .. } => {
            append_surf_wedge_render_geometry(
                builder,
                spec.center,
                local_points,
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

    builder.push_quad(a, b, d, c, ridge_face);
    builder.push_quad(e, g, h, f, underside);
    builder.push_quad(a, c, g, e, outer_face);
    builder.push_quad(b, f, h, d, side_shadow);
    builder.push_quad(a, e, f, b, deepen(base_color, 0.08).with_alpha(0.24));
    builder.push_quad(c, d, h, g, deepen(base_color, 0.14).with_alpha(0.22));

    let front_ridge = a;
    let back_ridge = b;
    let front_outer = c;
    let back_outer = d;
    let mut surface_normal = (back_ridge - front_ridge)
        .cross(front_outer - front_ridge)
        .normalize_or_zero();
    if surface_normal.y < 0.0 {
        surface_normal = -surface_normal;
    }

    let stripe_core = stripe_color.with_alpha(0.94);
    let stripe_glow = dim_linear(stripe_color, 0.42, 0.24);

    for &(t, stripe_width, glow_width, lift) in
        &[(0.18, 0.16, 0.32, 0.022), (0.64, 0.11, 0.22, 0.018)]
    {
        let front_center = front_ridge.lerp(front_outer, t);
        let back_center = back_ridge.lerp(back_outer, t);
        let front_face_dir = (front_outer - front_ridge).normalize_or_zero();
        let back_face_dir = (back_outer - back_ridge).normalize_or_zero();
        let glow_quad = [
            front_center - front_face_dir * (glow_width * 0.5) + surface_normal * lift,
            back_center - back_face_dir * (glow_width * 0.5) + surface_normal * lift,
            back_center + back_face_dir * (glow_width * 0.5) + surface_normal * lift,
            front_center + front_face_dir * (glow_width * 0.5) + surface_normal * lift,
        ];
        let stripe_quad = [
            front_center - front_face_dir * (stripe_width * 0.5) + surface_normal * (lift + 0.012),
            back_center - back_face_dir * (stripe_width * 0.5) + surface_normal * (lift + 0.012),
            back_center + back_face_dir * (stripe_width * 0.5) + surface_normal * (lift + 0.012),
            front_center + front_face_dir * (stripe_width * 0.5) + surface_normal * (lift + 0.012),
        ];
        builder.push_quad(
            glow_quad[0],
            glow_quad[1],
            glow_quad[2],
            glow_quad[3],
            stripe_glow,
        );
        builder.push_quad(
            stripe_quad[0],
            stripe_quad[1],
            stripe_quad[2],
            stripe_quad[3],
            stripe_core,
        );
    }
}

fn spawn_box_collider_spec(
    spec: &SolidSpec,
    commands: &mut Commands,
    chunk: Option<WorldChunkKey>,
) {
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
        SolidBody::StaticSurfWedge { local_points, .. } => {
            if let Some(collider) = Collider::convex_hull(local_points.clone()) {
                entity.insert((
                    RigidBody::Static,
                    collider,
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
        material.cull_mode = None;
        material.perceptual_roughness = 0.04;
        material.reflectance = 0.96;
        material.clearcoat = 1.0;
        material.clearcoat_perceptual_roughness = 0.02;
        material.metallic = 0.0;
        material.specular_tint = paint_stripe_color(key.paint);
        material.emissive = LinearRgba::BLACK;
        material.alpha_mode = AlphaMode::Blend;
        material.specular_transmission = 0.82;
        material.diffuse_transmission = 0.28;
        material.thickness = 0.72;
        material.ior = 1.18;
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

    let needs_collider = !matches!(&spec.body, SolidBody::Decoration);
    if !needs_collider {
        return Ok(());
    }

    let aabb = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match &spec.body {
        SolidBody::StaticSurfWedge { local_points, .. } => {
            Collider::convex_hull(local_points.clone())
                .unwrap()
                .aabb(spec.center, Quat::IDENTITY)
        }
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
        paint: PaintStyle::ThemeFloor(room.theme),
        body: SolidBody::Static,
        friction: None,
        extra: ExtraKind::None,
    });

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
                    paint: PaintStyle::ThemeAccent(room.theme),
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
                paint: PaintStyle::ThemeShadow(room.theme),
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
                paint: PaintStyle::ThemeAccent(room.theme),
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
                        paint: PaintStyle::ThemeAccent(room.theme),
                        body: SolidBody::Static,
                        friction: None,
                        extra: ExtraKind::None,
                    });
                }
            }
        }
    }

    if let Some(index) = room.checkpoint_slot {
        layout.solids.push(SolidSpec {
            owner,
            label: format!("Checkpoint {}", room.index),
            center: top_to_center(room.top + Vec3::Y * 0.18, 0.18),
            size: Vec3::new(2.9, 0.18, 2.9),
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
            append_css_surf_sequence(
                &mut layout,
                owner,
                &mut rng,
                start,
                end,
                forward,
                right,
                to.theme,
                false,
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
                true,
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

fn append_css_surf_sequence(
    layout: &mut ModuleLayout,
    owner: OwnerTag,
    rng: &mut RunRng,
    start: Vec3,
    end: Vec3,
    forward: Vec3,
    right: Vec3,
    theme: Theme,
    intense: bool,
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
    let surf_start = start + forward * entry_margin + Vec3::Y * 0.4;
    let surf_end = end - forward * exit_margin + Vec3::Y * 0.18;
    let total_distance = surf_start.distance(surf_end).max(12.0);
    let segment_count = if intense {
        ((total_distance / 7.5).round() as usize).clamp(34, 96)
    } else {
        ((total_distance / 8.2).round() as usize).clamp(28, 72)
    };
    let curve_cycles = if intense {
        rng.range_f32(1.0, 1.6)
    } else {
        rng.range_f32(0.75, 1.25)
    };
    let curve_phase = rng.range_f32(0.0, TAU);
    let curve_amplitude = if intense {
        rng.range_f32(0.62, 1.15)
    } else {
        rng.range_f32(0.42, 0.88)
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

    let mut centerline = Vec::with_capacity(segment_count + 1);
    for sample in 0..=segment_count {
        let t = sample as f32 / segment_count as f32;
        let envelope = (t * PI).sin().max(0.0).powf(0.85);
        let weave = (t * curve_cycles * TAU + curve_phase).sin();
        let offset = right * weave * curve_amplitude * envelope;
        let lift = Vec3::Y * ridge_lift * envelope;
        centerline.push(surf_start.lerp(surf_end, t) + offset + lift);
    }

    let mut seam_rights = Vec::with_capacity(segment_count + 1);
    for index in 0..=segment_count {
        let prev = if index == 0 {
            centerline[1] - centerline[0]
        } else {
            centerline[index] - centerline[index - 1]
        };
        let next = if index == segment_count {
            centerline[index] - centerline[index - 1]
        } else {
            centerline[index + 1] - centerline[index]
        };
        let tangent = direction_from_delta(prev + next);
        let tangent = if tangent == Vec3::ZERO {
            forward
        } else {
            tangent
        };
        let mut seam_right = Vec3::new(-tangent.z, 0.0, tangent.x).normalize_or_zero();
        if seam_right == Vec3::ZERO {
            seam_right = right;
        } else if seam_right.dot(right) < 0.0 {
            seam_right = -seam_right;
        }
        seam_rights.push(seam_right);
    }

    for index in 0..segment_count {
        let section_start = centerline[index];
        let section_end = centerline[index + 1];
        let section_delta = section_end - section_start;
        let section_forward = direction_from_delta(section_delta);
        if section_forward == Vec3::ZERO {
            continue;
        }
        for side in [-1.0_f32, 1.0] {
            let (center, local_points, bounds) = surf_wedge_from_seams(
                section_start,
                section_end,
                seam_rights[index] * side,
                seam_rights[index + 1] * side,
                ramp_span,
                ramp_drop,
            );
            layout.solids.push(SolidSpec {
                owner,
                label: if intense {
                    format!("Surf Wedge {} {}", index, side)
                } else {
                    format!("Flow Wedge {} {}", index, side)
                },
                center,
                size: bounds,
                paint: if side < 0.0 {
                    PaintStyle::ThemeAccent(theme)
                } else {
                    PaintStyle::ThemeFloor(theme)
                },
                body: SolidBody::StaticSurfWedge {
                    wall_side: side,
                    local_points,
                },
                friction: Some(0.0),
                extra: ExtraKind::None,
            });
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

fn build_branch_layout(
    index: usize,
    branch: &BranchPlan,
    rooms: &[RoomPlan],
    _unlocked_shortcuts: &HashSet<u64>,
) -> ModuleLayout {
    let mut layout = ModuleLayout::default();
    let owner = OwnerTag::Branch(index);
    let room = &rooms[branch.room_index];
    let branch_dir = branch.dir.normalize_or_zero();
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
            emissive: LinearRgba::from(theme_glow_color(theme)) * 0.04,
            reflectance: 0.1,
            perceptual_roughness: 0.94,
            ..default()
        },
        PaintStyle::Prop(theme) => StandardMaterial {
            base_color: theme_prop_color(theme),
            emissive: LinearRgba::from(theme_glow_color(theme)) * 0.07,
            reflectance: 0.46,
            specular_tint: theme_glow_color(theme),
            clearcoat: 0.22,
            clearcoat_perceptual_roughness: 0.34,
            perceptual_roughness: 0.42,
            ..default()
        },
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
        PaintStyle::Treasure => StandardMaterial {
            base_color: Color::srgb(0.86, 0.68, 0.24),
            emissive: LinearRgba::rgb(0.42, 0.26, 0.1),
            reflectance: 0.82,
            clearcoat: 0.84,
            clearcoat_perceptual_roughness: 0.14,
            perceptual_roughness: 0.14,
            metallic: 0.26,
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
                AlphaMode::Blend
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
            alpha_mode: AlphaMode::Blend,
            emissive: LinearRgba::rgb(0.1, 0.2, 0.26),
            reflectance: 0.94,
            clearcoat: 1.0,
            clearcoat_perceptual_roughness: 0.04,
            perceptual_roughness: 0.06,
            ..default()
        },
        PaintStyle::Water => StandardMaterial {
            base_color: Color::srgba(0.02, 0.16, 0.34, 0.5),
            alpha_mode: AlphaMode::Blend,
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
        Theme::Stone => Color::srgb(0.16, 0.18, 0.3),
        Theme::Overgrown => Color::srgb(0.08, 0.2, 0.26),
        Theme::Frost => Color::srgb(0.18, 0.24, 0.38),
        Theme::Ember => Color::srgb(0.28, 0.16, 0.28),
    }
}

fn theme_accent_color(theme: Theme) -> Color {
    match theme {
        Theme::Stone => Color::srgb(0.32, 0.36, 0.66),
        Theme::Overgrown => Color::srgb(0.12, 0.54, 0.62),
        Theme::Frost => Color::srgb(0.28, 0.62, 0.88),
        Theme::Ember => Color::srgb(0.78, 0.32, 0.66),
    }
}

fn theme_shadow_color(theme: Theme) -> Color {
    match theme {
        Theme::Stone => Color::srgb(0.04, 0.05, 0.11),
        Theme::Overgrown => Color::srgb(0.03, 0.07, 0.1),
        Theme::Frost => Color::srgb(0.04, 0.07, 0.14),
        Theme::Ember => Color::srgb(0.08, 0.03, 0.09),
    }
}

fn theme_prop_color(theme: Theme) -> Color {
    match theme {
        Theme::Stone => Color::srgb(0.28, 0.32, 0.46),
        Theme::Overgrown => Color::srgb(0.14, 0.32, 0.36),
        Theme::Frost => Color::srgb(0.26, 0.5, 0.74),
        Theme::Ember => Color::srgb(0.52, 0.26, 0.48),
    }
}

fn theme_glow_color(theme: Theme) -> Color {
    match theme {
        Theme::Stone => Color::srgb(0.56, 0.7, 1.0),
        Theme::Overgrown => Color::srgb(0.28, 0.96, 0.9),
        Theme::Frost => Color::srgb(0.64, 0.92, 1.0),
        Theme::Ember => Color::srgb(1.0, 0.46, 0.84),
    }
}

fn paint_stripe_color(paint: PaintStyle) -> Color {
    match paint {
        PaintStyle::ThemeFloor(theme)
        | PaintStyle::ThemeAccent(theme)
        | PaintStyle::ThemeShadow(theme)
        | PaintStyle::Prop(theme) => {
            let glow = LinearRgba::from(theme_glow_color(theme));
            Color::linear_rgb(glow.red * 1.1, glow.green * 1.1, glow.blue * 1.1)
        }
        PaintStyle::Summit => Color::linear_rgb(1.3, 0.98, 0.34),
        PaintStyle::Checkpoint => Color::linear_rgb(0.42, 1.2, 0.98),
        PaintStyle::Treasure => Color::linear_rgb(1.26, 1.02, 0.4),
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
    match index % 5 {
        0 => Color::srgb(0.34, 0.58, 1.0),
        1 => Color::srgb(0.26, 0.72, 0.62),
        2 => Color::srgb(0.7, 0.48, 0.94),
        3 => Color::srgb(0.94, 0.48, 0.3),
        _ => Color::srgb(0.78, 0.84, 1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_solids_have_valid_collider_bounds() {
        for seed in 0_u64..256 {
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
                let layout = build_segment_layout(segment, &blueprint.rooms, &HashSet::default());
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

            for (index, branch) in blueprint.branches.iter().enumerate() {
                let layout =
                    build_branch_layout(index, branch, &blueprint.rooms, &HashSet::default());
                for solid in &layout.solids {
                    assert!(
                        validate_solid_spec(solid).is_ok(),
                        "seed {seed} branch {} solid '{}' invalid: {}",
                        index,
                        solid.label,
                        validate_solid_spec(solid).unwrap_err()
                    );
                }
            }

            let summit = build_summit_layout(blueprint.rooms.last().unwrap(), blueprint.summit);
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
            true,
        );

        let wedges = layout
            .solids
            .iter()
            .filter_map(|solid| match &solid.body {
                SolidBody::StaticSurfWedge {
                    wall_side,
                    local_points,
                } if *wall_side > 0.0 => Some((solid.center, local_points)),
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

        if window.frontier_room < blueprint.rooms.len() - 1 {
            assert!(
                chunks.contains(&WorldChunkKey::Segment(window.frontier_room)),
                "frontier exit segment {} -> {} should be preloaded",
                window.frontier_room,
                window.frontier_room + 1
            );
        }
    }

    #[test]
    fn stream_focus_advances_with_player_position() {
        let blueprint = build_run_blueprint(0x5eaf_u64);
        let checkpoint = 4.min(blueprint.rooms.len().saturating_sub(3));
        let ahead_room = (checkpoint + 3).min(blueprint.rooms.len() - 1);
        let player_position = blueprint.rooms[ahead_room].top + Vec3::new(0.0, 1.0, 0.0);
        let focus = stream_focus_room(&blueprint, checkpoint, player_position);

        assert!(
            focus >= ahead_room.saturating_sub(1),
            "focus room {} should advance toward the player's actual route position {}",
            focus,
            ahead_room
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

fn surf_wedge_from_seams(
    start_ridge: Vec3,
    end_ridge: Vec3,
    start_outward: Vec3,
    end_outward: Vec3,
    ramp_span: f32,
    ramp_drop: f32,
) -> (Vec3, Vec<Vec3>, Vec3) {
    let start_outer =
        start_ridge + start_outward.normalize_or_zero() * ramp_span - Vec3::Y * ramp_drop;
    let end_outer = end_ridge + end_outward.normalize_or_zero() * ramp_span - Vec3::Y * ramp_drop;
    let world_points = [
        start_ridge,
        end_ridge,
        start_outer,
        end_outer,
        start_ridge - Vec3::Y * SURF_WEDGE_THICKNESS,
        end_ridge - Vec3::Y * SURF_WEDGE_THICKNESS,
        start_outer - Vec3::Y * SURF_WEDGE_THICKNESS,
        end_outer - Vec3::Y * SURF_WEDGE_THICKNESS,
    ];
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut center = Vec3::ZERO;
    for point in world_points {
        min = min.min(point);
        max = max.max(point);
        center += point;
    }
    center /= world_points.len() as f32;
    let local_points = world_points
        .into_iter()
        .map(|point| point - center)
        .collect::<Vec<_>>();
    (center, local_points, max - min)
}

fn build_surf_wedge_mesh(points: &[Vec3], base_color: Color, stripe_color: Color) -> Mesh {
    let mut builder = ColoredMeshBuilder::default();
    append_surf_wedge_render_geometry(&mut builder, Vec3::ZERO, points, base_color, stripe_color);
    builder.build()
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
