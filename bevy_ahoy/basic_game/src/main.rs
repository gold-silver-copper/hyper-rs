use std::{
    collections::HashMap,
    f32::consts::{PI, TAU},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use avian3d::prelude::*;
use bevy::{
    asset::RenderAssetUsages,
    camera::Exposure,
    core_pipeline::tonemapping::Tonemapping,
    input::common_conditions::input_just_pressed,
    light::CascadeShadowConfigBuilder,
    math::primitives::{Cuboid, Sphere},
    mesh::{Indices, MeshVertexBufferLayoutRef},
    pbr::{
        ExtendedMaterial, Material, MaterialExtension, MaterialPipeline, MaterialPipelineKey,
        OpaqueRendererMethod,
    },
    prelude::*,
    reflect::TypePath,
    render::render_resource::{
        AsBindGroup, Face, PrimitiveTopology, RenderPipelineDescriptor, ShaderType,
        SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
    window::{CursorGrabMode, CursorOptions, WindowResolution},
};
use bevy_ahoy::{CharacterControllerOutput, CharacterLook, input::AccumulatedInput, prelude::*};
use bevy_ecs::{lifecycle::HookContext, world::DeferredWorld};
use bevy_enhanced_input::prelude::*;
use bevy_time::Stopwatch;

use crate::util::{ControlsOverlay, ExampleUtilPlugin, StableGround};

mod util;

const PLAYER_SPAWN_CLEARANCE: f32 = 3.1;
const SURF_SPAWN_CLEARANCE: f32 = 8.0;
const SURF_SPAWN_FACE_T: f32 = 0.42;
const SPAWN_PLATFORM_EDGE_INSET: f32 = 3.0;
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
const INITIAL_ROOM_COUNT: usize = 16;
const APPEND_TRIGGER_ROOMS: usize = 4;
const APPEND_BATCH_ROOMS: usize = 8;
const MAX_SECTION_TURN_RADIANS: f32 = 18.0_f32.to_radians();
const BHOP_OBJECT_SCALE: f32 = 5.0;
const BHOP_CADENCE_SCALE: f32 = 4.1;
const BHOP_MIN_ROUTE_EDGE_GAP: f32 = 4.0;
const BHOP_MIN_BRANCH_EDGE_GAP: f32 = 3.0;
const BHOP_MIN_ANCHOR_EDGE_CLEARANCE: f32 = 0.75;
const BHOP_VERTICAL_CLEARANCE_BIAS: f32 = 1.0;
const BHOP_SPACING_EPSILON: f32 = 0.05;
const BHOP_MAX_REPAIR_PASSES: usize = 6;
const BHOP_MAX_BUILD_ATTEMPTS: usize = 8;
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
const SURF_SECTION_GAP_MIN: f32 = 128.0;
const SURF_SECTION_GAP_MAX: f32 = 168.0;
const SURF_SECTION_DROP_MIN: f32 = 24.0;
const SURF_SECTION_DROP_MAX: f32 = 36.0;
const BHOP_SECTION_GAP_MIN: f32 = 98.0;
const BHOP_SECTION_GAP_MAX: f32 = 132.0;
const BHOP_SECTION_DROP_MIN: f32 = 16.0;
const BHOP_SECTION_DROP_MAX: f32 = 28.0;
const BHOP_ANCHOR_MARGIN_MIN: f32 = 4.5;
const BHOP_ANCHOR_MARGIN_MAX: f32 = 9.5;
const BHOP_SURF_ALIGNMENT_DROP: f32 = 11.5;
const SURF_ENTRY_MARGIN: f32 = 1.0;
const SURF_EXIT_MARGIN_MIN: f32 = 4.5;
const SURF_EXIT_MARGIN_MAX: f32 = 9.0;
const SKY_DOME_RADIUS: f32 = 1_240.0;
const WORLD_SURFACE_SHADER_ASSET_PATH: &str = "shaders/world_surface_material.wgsl";
const NEBULA_SKY_SHADER_ASSET_PATH: &str = "shaders/nebula_sky.wgsl";

type WorldSurfaceMaterial = ExtendedMaterial<StandardMaterial, WorldSurfaceExtension>;

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct WorldSurfaceExtension {
    #[uniform(100)]
    settings: WorldSurfaceSettings,
}

#[derive(ShaderType, Debug, Clone)]
struct WorldSurfaceSettings {
    accent: Vec4,
    secondary: Vec4,
    emissive: Vec4,
    atmosphere: Vec4,
    params_a: Vec4,
    params_b: Vec4,
    params_c: Vec4,
    params_d: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct NebulaSkyMaterial {
    #[uniform(0)]
    settings: NebulaSkySettings,
}

#[derive(ShaderType, Debug, Clone)]
struct NebulaSkySettings {
    zenith: Vec4,
    horizon: Vec4,
    nebula_a: Vec4,
    nebula_b: Vec4,
    star: Vec4,
    halo: Vec4,
    params_a: Vec4,
    params_b: Vec4,
    params_c: Vec4,
}

impl MaterialExtension for WorldSurfaceExtension {
    fn fragment_shader() -> ShaderRef {
        WORLD_SURFACE_SHADER_ASSET_PATH.into()
    }

    fn enable_prepass() -> bool {
        false
    }
}

impl Material for NebulaSkyMaterial {
    fn fragment_shader() -> ShaderRef {
        NEBULA_SKY_SHADER_ASSET_PATH.into()
    }

    fn opaque_render_method(&self) -> OpaqueRendererMethod {
        OpaqueRendererMethod::Forward
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = Some(Face::Front);
        if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
            depth_stencil.depth_write_enabled = false;
        }
        Ok(())
    }
}

fn color_to_vec4(color: Color) -> Vec4 {
    let [r, g, b, a] = LinearRgba::from(color).to_f32_array();
    Vec4::new(r, g, b, a)
}

fn visual_motion_factor(speed: f32) -> f32 {
    ((speed - 120.0) / 780.0).clamp(0.0, 1.0)
}

fn bhop_world_material() -> WorldSurfaceMaterial {
    WorldSurfaceMaterial {
        base: StandardMaterial {
            base_color: Color::srgb(0.014, 0.01, 0.022),
            reflectance: 0.12,
            clearcoat: 0.02,
            clearcoat_perceptual_roughness: 0.72,
            perceptual_roughness: 0.94,
            opaque_render_method: OpaqueRendererMethod::Forward,
            ..default()
        },
        extension: WorldSurfaceExtension {
            settings: WorldSurfaceSettings {
                accent: color_to_vec4(Color::linear_rgb(1.0, 0.88, 0.96)),
                secondary: color_to_vec4(Color::linear_rgb(0.9, 0.18, 0.78)),
                emissive: color_to_vec4(Color::linear_rgb(0.22, 0.0, 0.16)),
                atmosphere: color_to_vec4(Color::linear_rgb(0.05, 0.01, 0.09)),
                params_a: Vec4::new(0.0, 0.11, 0.18, 0.22),
                params_b: Vec4::new(0.18, 0.0, 3.2, 2.6),
                params_c: Vec4::new(160.0, 1_220.0, 0.03, 0.44),
                params_d: Vec4::new(0.0, 0.12, 0.0, 0.0),
            },
        },
    }
}

fn surf_world_material() -> WorldSurfaceMaterial {
    WorldSurfaceMaterial {
        base: StandardMaterial {
            base_color: Color::srgb(0.13, 0.08, 0.24),
            cull_mode: None,
            reflectance: 0.54,
            clearcoat: 0.92,
            clearcoat_perceptual_roughness: 0.08,
            perceptual_roughness: 0.24,
            opaque_render_method: OpaqueRendererMethod::Forward,
            ..default()
        },
        extension: WorldSurfaceExtension {
            settings: WorldSurfaceSettings {
                accent: color_to_vec4(Color::linear_rgb(1.0, 0.95, 0.985)),
                secondary: color_to_vec4(Color::linear_rgb(0.78, 0.2, 0.98)),
                emissive: color_to_vec4(Color::linear_rgb(0.34, 0.04, 0.48)),
                atmosphere: color_to_vec4(Color::linear_rgb(0.08, 0.02, 0.16)),
                params_a: Vec4::new(1.0, 0.12, 0.0, 0.28),
                params_b: Vec4::new(0.68, 2.8, 24.0, 7.2),
                params_c: Vec4::new(180.0, 1_260.0, 0.22, 0.18),
                params_d: Vec4::new(0.0, 0.42, 0.0, 0.18),
            },
        },
    }
}

fn nebula_sky_material() -> NebulaSkyMaterial {
    NebulaSkyMaterial {
        settings: NebulaSkySettings {
            zenith: color_to_vec4(Color::linear_rgb(0.005, 0.001, 0.02)),
            horizon: color_to_vec4(Color::linear_rgb(0.035, 0.006, 0.07)),
            nebula_a: color_to_vec4(Color::linear_rgb(0.35, 0.04, 0.52)),
            nebula_b: color_to_vec4(Color::linear_rgb(0.98, 0.18, 0.58)),
            star: color_to_vec4(Color::linear_rgba(1.0, 0.95, 0.98, 1.2)),
            halo: color_to_vec4(Color::linear_rgba(0.88, 0.28, 1.0, 0.28)),
            params_a: Vec4::new(2.1, 1.3, 360.0, 28.0),
            params_b: Vec4::new(0.0024, -0.0016, 0.34, 0.28),
            params_c: Vec4::new(-0.46, 0.22, 0.86, 7.5),
        },
    }
}

struct BasicGamePlugin;

impl Plugin for BasicGamePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(Color::srgb(0.008, 0.002, 0.02)))
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
                PostUpdate,
                (sync_sky_dome_to_camera, update_render_dynamics).chain(),
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
                    extend_course_ahead,
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
            MaterialPlugin::<WorldSurfaceMaterial>::default(),
            MaterialPlugin::<NebulaSkyMaterial>::default(),
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
    mut materials: ResMut<Assets<WorldSurfaceMaterial>>,
    mut sky_materials: ResMut<Assets<NebulaSkyMaterial>>,
    mut asset_cache: ResMut<WorldAssetCache>,
) {
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.08, 0.03, 0.12),
        brightness: 30.0,
        affects_lightmapped_meshes: true,
    });

    let blueprint = build_run_blueprint(current_run_seed());
    let initial_look = spawn_look_for_blueprint(&blueprint);
    spawn_nebula_sky(
        &mut commands,
        &mut meshes,
        &mut sky_materials,
        blueprint.spawn,
    );
    spawn_world(
        &blueprint,
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut asset_cache,
    );
    commands.insert_resource(RunState::new(blueprint.clone()));

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
        Tonemapping::AcesFitted,
        Exposure { ev100: 9.0 },
        DistanceFog {
            color: Color::srgba(0.035, 0.008, 0.07, 0.42),
            directional_light_color: Color::srgba(0.86, 0.42, 1.0, 0.12),
            directional_light_exponent: 14.0,
            falloff: FogFalloff::Linear {
                start: 250.0,
                end: 1_420.0,
            },
        },
    ));
}

fn spawn_world(
    blueprint: &RunBlueprint,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<WorldSurfaceMaterial>,
    asset_cache: &mut WorldAssetCache,
) {
    spawn_basic_lighting(commands);
    spawn_world_range(blueprint, 0, 0, commands, meshes, materials, asset_cache);
}

fn spawn_world_range(
    blueprint: &RunBlueprint,
    room_start: usize,
    segment_start: usize,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<WorldSurfaceMaterial>,
    asset_cache: &mut WorldAssetCache,
) {
    for room in &blueprint.rooms[room_start.min(blueprint.rooms.len())..] {
        let layout = build_room_layout(room);
        spawn_layout(&layout, commands, meshes, materials, asset_cache);
    }
    for segment in &blueprint.segments[segment_start.min(blueprint.segments.len())..] {
        let layout = build_segment_layout(segment, &blueprint.rooms);
        spawn_layout(&layout, commands, meshes, materials, asset_cache);
    }
}

fn spawn_basic_lighting(commands: &mut Commands) {
    commands.spawn((
        GeneratedWorld,
        Name::new("Sun"),
        Transform::from_xyz(170.0, 260.0, -120.0).looking_at(Vec3::new(0.0, 40.0, 0.0), Vec3::Y),
        DirectionalLight {
            shadows_enabled: true,
            illuminance: 21_000.0,
            color: Color::srgb(0.98, 0.9, 1.0),
            shadow_depth_bias: 0.12,
            shadow_normal_bias: 0.52,
            ..default()
        },
        CascadeShadowConfigBuilder {
            first_cascade_far_bound: 72.0,
            maximum_distance: 760.0,
            overlap_proportion: 0.18,
            ..default()
        }
        .build(),
    ));

    commands.spawn((
        GeneratedWorld,
        Name::new("Fill Light"),
        Transform::from_xyz(-150.0, 220.0, 150.0).looking_at(Vec3::new(0.0, 30.0, 0.0), Vec3::Y),
        DirectionalLight {
            shadows_enabled: false,
            illuminance: 7_500.0,
            color: Color::srgb(0.8, 0.42, 0.96),
            ..default()
        },
    ));
}

fn spawn_nebula_sky(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<NebulaSkyMaterial>,
    center: Vec3,
) {
    commands.spawn((
        Name::new("Nebula Sky"),
        SkyDome,
        Mesh3d(meshes.add(Sphere::new(SKY_DOME_RADIUS).mesh().uv(64, 32))),
        MeshMaterial3d(materials.add(nebula_sky_material())),
        Transform::from_translation(center),
    ));
}

fn sync_sky_dome_to_camera(
    camera: Query<&Transform, (With<Camera3d>, Without<SkyDome>)>,
    mut sky: Query<&mut Transform, With<SkyDome>>,
) {
    let Ok(camera_transform) = camera.single() else {
        return;
    };

    for mut sky_transform in &mut sky {
        sky_transform.translation = camera_transform.translation;
    }
}

fn update_render_dynamics(
    players: Query<&LinearVelocity, With<Player>>,
    asset_cache: Res<WorldAssetCache>,
    mut materials: ResMut<Assets<WorldSurfaceMaterial>>,
) {
    let speed = players
        .single()
        .map(|velocity| velocity.length())
        .unwrap_or(0.0);
    let motion = visual_motion_factor(speed);

    for handle in asset_cache.materials.values() {
        if let Some(material) = materials.get_mut(handle) {
            material.extension.settings.params_d.x = motion;
        }
    }
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
R: new seed\n\
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
         Sections: {rooms}\n\
         Height: {height:.1} | Speed: {speed:.1}\n\
         Time: {time:.1}s\n\
         Mode: Endless",
        seed = run.blueprint.seed,
        rooms = run.blueprint.rooms.len(),
        height = player.translation.y,
        speed = velocity.length(),
        time = run.timer.elapsed_secs(),
    );
}

fn tick_run_timer(time: Res<Time>, mut run: ResMut<RunState>) {
    run.timer.tick(time.delta());
}

fn queue_run_controls(keys: Res<ButtonInput<KeyCode>>, mut director: ResMut<RunDirector>) {
    if director.pending.is_some() {
        return;
    }

    if keys.just_pressed(KeyCode::KeyR) {
        director.pending = Some(RunRequest {
            kind: RunRequestKind::RestartNewSeed,
            seed: current_run_seed(),
        });
    }
}

fn extend_course_ahead(
    mut commands: Commands,
    director: Res<RunDirector>,
    mut run: ResMut<RunState>,
    players: Query<&Transform, With<Player>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<WorldSurfaceMaterial>>,
    mut asset_cache: ResMut<WorldAssetCache>,
) {
    if director.pending.is_some() {
        return;
    }

    let Ok(player) = players.single() else {
        return;
    };

    let focus_room = nearest_room_index(&run.blueprint, player.translation);
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
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut asset_cache,
    );
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
        ),
        With<Player>,
    >,
    mut camera: Query<&mut Transform, (With<Camera3d>, Without<Player>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<WorldSurfaceMaterial>>,
    mut asset_cache: ResMut<WorldAssetCache>,
) {
    let Some(request) = director.pending.take() else {
        return;
    };

    let reset_player = match request.kind {
        RunRequestKind::RestartNewSeed => {
            for entity in &generated {
                commands.entity(entity).despawn();
            }
            let blueprint = build_run_blueprint(request.seed);
            spawn_world(
                &blueprint,
                &mut commands,
                &mut meshes,
                &mut materials,
                &mut asset_cache,
            );
            run.apply_restart_blueprint(blueprint);
            true
        }
    };

    let spawn = run.blueprint.spawn;
    let look = spawn_look_for_blueprint(&run.blueprint);

    if reset_player
        && let Ok((mut position, mut transform, mut velocity, mut character_look)) =
            players.single_mut()
    {
        position.0 = spawn;
        transform.translation = spawn;
        velocity.0 = Vec3::ZERO;
        *character_look = look.clone();
    }

    if let Ok(mut camera_transform) = camera.single_mut() {
        camera_transform.rotation = look.to_quat();
    }
}

fn spawn_look_for_blueprint(blueprint: &RunBlueprint) -> CharacterLook {
    let facing = spawn_facing_for_blueprint(blueprint);
    if facing == Vec3::ZERO {
        return CharacterLook::default();
    }

    CharacterLook {
        yaw: (-facing.x).atan2(-facing.z),
        pitch: 0.0,
    }
}

fn spawn_facing_for_blueprint(blueprint: &RunBlueprint) -> Vec3 {
    if let Some(segment) = blueprint.first_segment() {
        let layout = build_segment_layout(segment, &blueprint.rooms);
        if let Some(facing) = first_segment_facing(&layout, segment.kind) {
            return facing;
        }
    }

    let next_room = 1.min(blueprint.rooms.len().saturating_sub(1));
    let mut facing = blueprint.rooms[next_room].top - blueprint.rooms[0].top;
    facing.y = 0.0;
    facing.normalize_or_zero()
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

#[derive(Component)]
struct Player;

#[derive(Component)]
struct RunHud;

#[derive(Component)]
struct GeneratedWorld;

#[derive(Component)]
struct SurfRampSurface;

#[derive(Component)]
struct SkyDome;

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
    RestartNewSeed,
}

#[derive(Resource)]
struct RunState {
    blueprint: RunBlueprint,
    timer: Stopwatch,
}

impl RunState {
    fn new(blueprint: RunBlueprint) -> Self {
        Self {
            blueprint,
            timer: Stopwatch::new(),
        }
    }

    fn apply_restart_blueprint(&mut self, blueprint: RunBlueprint) {
        self.blueprint = blueprint;
        self.timer = Stopwatch::new();
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
    generator: RunRng,
}

impl RunBlueprint {
    fn first_segment(&self) -> Option<&SegmentPlan> {
        self.segments.first()
    }
}

#[derive(Clone)]
struct RoomPlan {
    index: usize,
    top: Vec3,
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

fn build_run_blueprint(seed: u64) -> RunBlueprint {
    let spawn_room = RoomPlan {
        index: 0,
        top: Vec3::new(0.0, 160.0, 0.0),
    };
    let mut blueprint = RunBlueprint {
        seed,
        rooms: vec![spawn_room],
        segments: Vec::with_capacity(INITIAL_ROOM_COUNT.saturating_sub(1)),
        spawn: Vec3::ZERO,
        tail_forward: Vec3::X,
        next_segment_kind: SegmentKind::SquareBhop,
        generator: RunRng::new(seed),
    };
    append_run_blueprint(&mut blueprint, INITIAL_ROOM_COUNT.saturating_sub(1));
    blueprint.spawn = spawn_position_for_first_segment(&blueprint);
    blueprint
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
            SegmentKind::SurfRamp => blueprint
                .generator
                .range_f32(SURF_SECTION_GAP_MIN, SURF_SECTION_GAP_MAX),
            SegmentKind::SquareBhop => blueprint
                .generator
                .range_f32(BHOP_SECTION_GAP_MIN, BHOP_SECTION_GAP_MAX),
        };
        let drop = match kind {
            SegmentKind::SurfRamp => blueprint
                .generator
                .range_f32(SURF_SECTION_DROP_MIN, SURF_SECTION_DROP_MAX),
            SegmentKind::SquareBhop => blueprint
                .generator
                .range_f32(BHOP_SECTION_DROP_MIN, BHOP_SECTION_DROP_MAX),
        };
        let lateral_jitter = right * blueprint.generator.range_f32(-4.0, 4.0);
        let next_top = from_top + forward * gap + lateral_jitter - Vec3::Y * drop;
        let next_index = blueprint.rooms.len();

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
        });
        blueprint.tail_forward = forward;
        blueprint.next_segment_kind = next_segment_kind(kind);
    }
}

fn next_segment_kind(kind: SegmentKind) -> SegmentKind {
    match kind {
        SegmentKind::SurfRamp => SegmentKind::SquareBhop,
        SegmentKind::SquareBhop => SegmentKind::SurfRamp,
    }
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
    SurfRamp,
}

#[derive(Resource, Default)]
struct WorldAssetCache {
    cuboid_meshes: HashMap<MeshSizeKey, Handle<Mesh>>,
    materials: HashMap<MaterialKey, Handle<WorldSurfaceMaterial>>,
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

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum PaintStyle {
    BhopPlatform,
    SurfRamp,
}

#[derive(Default, Clone)]
struct ModuleLayout {
    solids: Vec<SolidSpec>,
}

fn build_room_layout(_room: &RoomPlan) -> ModuleLayout {
    ModuleLayout::default()
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
        SegmentKind::SquareBhop => append_square_bhop_sequence(
            &mut layout,
            &mut rng,
            start,
            end,
            forward,
            right,
            segment.index,
        ),
    }

    layout
}

fn spawn_position_for_first_segment(blueprint: &RunBlueprint) -> Vec3 {
    let Some(segment) = blueprint.first_segment() else {
        return blueprint.rooms[0].top + Vec3::new(0.0, PLAYER_SPAWN_CLEARANCE, 0.0);
    };
    let layout = build_segment_layout(segment, &blueprint.rooms);
    match segment.kind {
        SegmentKind::SquareBhop => square_bhop_spawn_position(&layout)
            .unwrap_or(blueprint.rooms[0].top + Vec3::new(0.0, PLAYER_SPAWN_CLEARANCE, 0.0)),
        SegmentKind::SurfRamp => surf_spawn_position(&layout)
            .unwrap_or(blueprint.rooms[0].top + Vec3::new(0.0, SURF_SPAWN_CLEARANCE, 0.0)),
    }
}

fn square_bhop_spawn_position(layout: &ModuleLayout) -> Option<Vec3> {
    let platform = layout
        .solids
        .iter()
        .find(|solid| matches!(solid.body, SolidBody::Static))?;
    let top = platform.center + Vec3::Y * (platform.size.y * 0.5);
    let forward = first_segment_facing(layout, SegmentKind::SquareBhop).unwrap_or(Vec3::X);
    let edge_offset = (platform.size.x * 0.5 - SPAWN_PLATFORM_EDGE_INSET).max(0.0);
    let spawn_top = top + forward * edge_offset;
    Some(spawn_top + Vec3::new(0.0, PLAYER_SPAWN_CLEARANCE, 0.0))
}

fn first_segment_facing(layout: &ModuleLayout, kind: SegmentKind) -> Option<Vec3> {
    match kind {
        SegmentKind::SquareBhop => {
            let mut platforms = layout
                .solids
                .iter()
                .filter(|solid| matches!(solid.body, SolidBody::Static));
            let first = platforms.next()?;
            let second = platforms.next()?;
            Some(direction_from_delta(second.center - first.center))
        }
        SegmentKind::SurfRamp => layout.solids.iter().find_map(|solid| match &solid.body {
            SolidBody::StaticSurfWedge {
                wall_side,
                render_points,
            } if *wall_side > 0.0 && render_points.len() >= 2 => Some(direction_from_delta(
                (solid.center + render_points[1]) - (solid.center + render_points[0]),
            )),
            _ => None,
        }),
    }
}

fn surf_spawn_position(layout: &ModuleLayout) -> Option<Vec3> {
    let wedge = layout.solids.iter().find_map(|solid| match &solid.body {
        SolidBody::StaticSurfWedge {
            wall_side,
            render_points,
        } if *wall_side > 0.0 && render_points.len() >= 4 => Some((solid.center, render_points)),
        _ => None,
    })?;

    let (center, render_points) = wedge;
    let (ridge, outer) = (center + render_points[0], center + render_points[2]);
    let face_point = ridge.lerp(outer, SURF_SPAWN_FACE_T);
    Some(face_point + Vec3::Y * SURF_SPAWN_CLEARANCE)
}

fn spawn_layout(
    layout: &ModuleLayout,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<WorldSurfaceMaterial>,
    asset_cache: &mut WorldAssetCache,
) {
    for solid in &layout.solids {
        spawn_solid(solid, commands, meshes, materials, asset_cache);
    }
}

fn spawn_solid(
    spec: &SolidSpec,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<WorldSurfaceMaterial>,
    asset_cache: &mut WorldAssetCache,
) {
    if let Err(reason) = validate_solid_spec(spec) {
        eprintln!("Skipping invalid solid '{}': {}", spec.label, reason);
        return;
    }

    match &spec.body {
        SolidBody::Static => {
            let mut entity = commands.spawn((
                GeneratedWorld,
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
                GeneratedWorld,
                Name::new(spec.label.clone()),
                Mesh3d(meshes.add(build_surf_wedge_mesh(
                    render_points,
                    paint_base_color(spec.paint),
                    paint_stripe_color(spec.paint),
                ))),
                MeshMaterial3d(cached_material(
                    asset_cache,
                    materials,
                    MaterialKey::SurfRamp,
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
                GeneratedWorld,
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
    materials: &mut Assets<WorldSurfaceMaterial>,
    key: MaterialKey,
) -> Handle<WorldSurfaceMaterial> {
    if let Some(handle) = cache.materials.get(&key) {
        return handle.clone();
    }

    let material = match key {
        MaterialKey::BhopPlatform => bhop_world_material(),
        MaterialKey::SurfRamp => surf_world_material(),
    };
    let handle = materials.add(material);
    cache.materials.insert(key, handle.clone());
    handle
}

fn material_key_for_paint(paint: PaintStyle) -> MaterialKey {
    match paint {
        PaintStyle::BhopPlatform => MaterialKey::BhopPlatform,
        PaintStyle::SurfRamp => MaterialKey::SurfRamp,
    }
}

fn paint_base_color(paint: PaintStyle) -> Color {
    match paint {
        PaintStyle::BhopPlatform => Color::srgb(0.07, 0.05, 0.12),
        PaintStyle::SurfRamp => Color::srgb(0.18, 0.12, 0.34),
    }
}

fn paint_stripe_color(paint: PaintStyle) -> Color {
    match paint {
        PaintStyle::BhopPlatform => Color::linear_rgb(1.0, 0.82, 0.95),
        PaintStyle::SurfRamp => Color::linear_rgb(1.0, 0.9, 0.97),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PathLateralStyle {
    Straight,
    Serpentine,
    Switchback,
    Arc,
    OneSidedArc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum BhopPattern {
    Rhythm,
    LaneSwitch,
    DropStrafe,
    SplitMerge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum BhopDifficultyBand {
    Intro,
    Mid,
    Advanced,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct BhopProfile {
    band: BhopDifficultyBand,
    pattern: BhopPattern,
    pad_count: usize,
    min_pad_count: usize,
    path_style: PathLateralStyle,
    weave_cycles: f32,
    phase: f32,
    lateral_amplitude: f32,
    vertical_wave: f32,
    lane_offset: f32,
    drop_depth: f32,
    catch_interval: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct BhopPadSpec {
    step: usize,
    route_mask: u8,
    top: Vec3,
    side: f32,
    height: f32,
    catch: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct BhopPlacementPlan {
    usable_length: f32,
    step_samples: Vec<f32>,
    pads: Vec<BhopPadSpec>,
}

#[derive(Clone, Copy, Debug)]
struct BhopSpacingRules {
    min_route_edge_gap: f32,
    min_branch_edge_gap: f32,
    epsilon: f32,
}

impl Default for BhopSpacingRules {
    fn default() -> Self {
        Self {
            min_route_edge_gap: BHOP_MIN_ROUTE_EDGE_GAP,
            min_branch_edge_gap: BHOP_MIN_BRANCH_EDGE_GAP,
            epsilon: BHOP_SPACING_EPSILON,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BhopSpacingIssueKind {
    RouteGap,
    BranchGap,
    Overlap,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct BhopSpacingIssue {
    kind: BhopSpacingIssueKind,
    left: usize,
    right: usize,
    deficit: f32,
    earlier_step: usize,
    later_step: usize,
}

fn append_square_bhop_sequence(
    layout: &mut ModuleLayout,
    rng: &mut RunRng,
    start: Vec3,
    end: Vec3,
    forward: Vec3,
    right: Vec3,
    segment_index: usize,
) {
    let (_, pads) = build_square_bhop_pads(segment_index, rng, start, end, forward, right);
    for pad in pads {
        layout.solids.push(SolidSpec {
            label: if pad.catch {
                format!("Square Bhop Catch {}", pad.step)
            } else {
                format!("Square Bhop Pad {}", pad.step)
            },
            center: top_to_center(pad.top, pad.height),
            size: Vec3::new(pad.side, pad.height, pad.side),
            paint: PaintStyle::BhopPlatform,
            body: SolidBody::Static,
            friction: None,
        });
    }
}

fn build_square_bhop_pads(
    segment_index: usize,
    rng: &mut RunRng,
    start: Vec3,
    end: Vec3,
    forward: Vec3,
    right: Vec3,
) -> (BhopProfile, Vec<BhopPadSpec>) {
    let distance = start.distance(end).max(18.0);
    let rules = BhopSpacingRules::default();
    let path_forward = direction_from_delta(end - start);
    let path_forward = if path_forward == Vec3::ZERO {
        forward
    } else {
        path_forward
    };
    let start_margin = bhop_path_margin(distance, BHOP_ANCHOR_MARGIN_MIN, BHOP_ANCHOR_MARGIN_MAX);
    let end_margin = bhop_path_margin(distance, BHOP_ANCHOR_MARGIN_MIN, BHOP_ANCHOR_MARGIN_MAX);
    let path_start = start + path_forward * start_margin
        - Vec3::Y * (BHOP_SURF_ALIGNMENT_DROP + BHOP_VERTICAL_CLEARANCE_BIAS);
    let path_end = end
        - path_forward * end_margin
        - Vec3::Y * (BHOP_SURF_ALIGNMENT_DROP + BHOP_VERTICAL_CLEARANCE_BIAS);
    let usable_length = path_start.distance(path_end).max(1.0);
    let base_rng = rng.clone();
    let mut profile_rng = base_rng.fork(0xB105_F11E);
    let base_profile = choose_bhop_profile(segment_index, usable_length, &mut profile_rng);

    for retry in 0..BHOP_MAX_BUILD_ATTEMPTS {
        let profile = retry_bhop_profile(base_profile, retry);
        let mut attempt_rng = base_rng.fork(0xA11E_0000_u64.wrapping_add(retry as u64));
        let Some(mut plan) = build_bhop_placement_plan(
            &profile,
            &mut attempt_rng,
            path_start,
            path_end,
            path_forward,
            right,
            &rules,
        ) else {
            continue;
        };
        if repair_bhop_spacing(&profile, &mut plan, path_forward, right, &rules)
            && validate_bhop_spacing(&profile, &plan.pads, &rules).is_empty()
        {
            return (profile, plan.pads);
        }
    }

    let mut fallback_profile = base_profile;
    fallback_profile.pattern = BhopPattern::LaneSwitch;
    fallback_profile.path_style = match fallback_profile.path_style {
        PathLateralStyle::Straight
        | PathLateralStyle::Serpentine
        | PathLateralStyle::Switchback => fallback_profile.path_style,
        _ => PathLateralStyle::Serpentine,
    };
    fallback_profile.pad_count = fallback_profile.min_pad_count;
    let mut fallback_plan = None;
    while fallback_profile.pad_count >= 2 && fallback_plan.is_none() {
        let mut fallback_rng =
            base_rng.fork(0xFA11_BACC_u64.wrapping_add(fallback_profile.pad_count as u64));
        fallback_plan = build_bhop_placement_plan(
            &fallback_profile,
            &mut fallback_rng,
            path_start,
            path_end,
            path_forward,
            right,
            &rules,
        );
        if fallback_plan.is_none() && fallback_profile.pad_count > 2 {
            fallback_profile.pad_count -= 1;
        } else {
            break;
        }
    }
    let mut fallback_plan = fallback_plan.expect("fallback lane-switch bhop placement should fit");
    let _ = repair_bhop_spacing(
        &fallback_profile,
        &mut fallback_plan,
        path_forward,
        right,
        &rules,
    );
    (fallback_profile, fallback_plan.pads)
}

fn choose_bhop_profile(segment_index: usize, usable_length: f32, rng: &mut RunRng) -> BhopProfile {
    let ordinal = bhop_segment_ordinal(segment_index);
    let band = bhop_difficulty_band(ordinal);
    let pattern = choose_bhop_pattern(rng, band);
    let path_style = choose_bhop_path_style(rng, band, pattern);
    let (cadence_min, cadence_max, min_count, max_count, min_spacing, catch_interval) = match band {
        BhopDifficultyBand::Intro => (5.4, 6.8, 6, 11, scaled_bhop_size(4.6), 3),
        BhopDifficultyBand::Mid => (4.8, 6.2, 7, 13, scaled_bhop_size(4.0), 4),
        BhopDifficultyBand::Advanced => (4.2, 5.6, 8, 16, scaled_bhop_size(3.6), 7),
    };
    let requested_count = ((usable_length / scaled_bhop_cadence(cadence_min, cadence_max, rng))
        .round() as usize)
        .clamp(min_count, max_count);
    let mut pad_count =
        clamp_platform_count_for_spacing(usable_length, requested_count, min_spacing, min_count);
    if pattern == BhopPattern::SplitMerge {
        pad_count = pad_count.max(8);
    }

    let weave_cycles = bhop_weave_cycles(path_style, band, rng);
    let lateral_amplitude = match band {
        BhopDifficultyBand::Intro => scaled_bhop_size(rng.range_f32(0.35, 0.82)),
        BhopDifficultyBand::Mid => scaled_bhop_size(rng.range_f32(0.7, 1.22)),
        BhopDifficultyBand::Advanced => scaled_bhop_size(rng.range_f32(1.0, 1.7)),
    };
    let vertical_wave = match band {
        BhopDifficultyBand::Intro => scaled_bhop_size(rng.range_f32(0.04, 0.12)),
        BhopDifficultyBand::Mid => scaled_bhop_size(rng.range_f32(0.08, 0.18)),
        BhopDifficultyBand::Advanced => scaled_bhop_size(rng.range_f32(0.1, 0.24)),
    };
    let lane_offset = match band {
        BhopDifficultyBand::Intro => 0.0,
        BhopDifficultyBand::Mid => scaled_bhop_size(rng.range_f32(0.8, 1.15)),
        BhopDifficultyBand::Advanced => scaled_bhop_size(rng.range_f32(1.1, 1.45)),
    };
    let drop_depth = match band {
        BhopDifficultyBand::Intro => 0.0,
        BhopDifficultyBand::Mid => scaled_bhop_size(rng.range_f32(0.45, 0.85)),
        BhopDifficultyBand::Advanced => scaled_bhop_size(rng.range_f32(0.85, 1.35)),
    };

    BhopProfile {
        band,
        pattern,
        pad_count,
        min_pad_count: min_count,
        path_style,
        weave_cycles,
        phase: rng.range_f32(0.0, TAU),
        lateral_amplitude,
        vertical_wave,
        lane_offset,
        drop_depth,
        catch_interval,
    }
}

fn retry_bhop_profile(base: BhopProfile, retry: usize) -> BhopProfile {
    let mut profile = base;
    let max_reduction = base.pad_count.saturating_sub(base.min_pad_count);
    let reduction = retry.saturating_sub(1).min(max_reduction);
    profile.pad_count = base
        .pad_count
        .saturating_sub(reduction)
        .max(base.min_pad_count);

    if retry >= 5 && base.pattern == BhopPattern::SplitMerge {
        profile.pattern = BhopPattern::LaneSwitch;
        profile.path_style = match base.path_style {
            PathLateralStyle::Straight
            | PathLateralStyle::Serpentine
            | PathLateralStyle::Switchback => base.path_style,
            _ => PathLateralStyle::Serpentine,
        };
    }

    profile
}

fn bhop_segment_ordinal(segment_index: usize) -> usize {
    segment_index / 2
}

fn bhop_difficulty_band(ordinal: usize) -> BhopDifficultyBand {
    match ordinal {
        0..=2 => BhopDifficultyBand::Intro,
        3..=5 => BhopDifficultyBand::Mid,
        _ => BhopDifficultyBand::Advanced,
    }
}

fn choose_bhop_pattern(rng: &mut RunRng, band: BhopDifficultyBand) -> BhopPattern {
    match band {
        BhopDifficultyBand::Intro => BhopPattern::Rhythm,
        BhopDifficultyBand::Mid => rng.weighted_choice(&[
            (BhopPattern::Rhythm, 4),
            (BhopPattern::LaneSwitch, 3),
            (BhopPattern::DropStrafe, 2),
        ]),
        BhopDifficultyBand::Advanced => rng.weighted_choice(&[
            (BhopPattern::Rhythm, 2),
            (BhopPattern::LaneSwitch, 3),
            (BhopPattern::DropStrafe, 2),
            (BhopPattern::SplitMerge, 3),
        ]),
    }
}

fn choose_bhop_path_style(
    rng: &mut RunRng,
    band: BhopDifficultyBand,
    pattern: BhopPattern,
) -> PathLateralStyle {
    match pattern {
        BhopPattern::Rhythm => match band {
            BhopDifficultyBand::Intro => {
                rng.weighted_choice(&[(PathLateralStyle::Straight, 6), (PathLateralStyle::Arc, 3)])
            }
            BhopDifficultyBand::Mid => rng.weighted_choice(&[
                (PathLateralStyle::Straight, 2),
                (PathLateralStyle::Serpentine, 4),
                (PathLateralStyle::Arc, 3),
            ]),
            BhopDifficultyBand::Advanced => rng.weighted_choice(&[
                (PathLateralStyle::Serpentine, 4),
                (PathLateralStyle::Switchback, 3),
                (PathLateralStyle::Arc, 3),
            ]),
        },
        BhopPattern::LaneSwitch => rng.weighted_choice(&[
            (PathLateralStyle::Straight, 2),
            (PathLateralStyle::Serpentine, 4),
            (PathLateralStyle::Switchback, 4),
        ]),
        BhopPattern::DropStrafe => rng.weighted_choice(&[
            (PathLateralStyle::Straight, 3),
            (PathLateralStyle::Arc, 4),
            (PathLateralStyle::Serpentine, 2),
        ]),
        BhopPattern::SplitMerge => rng.weighted_choice(&[
            (PathLateralStyle::Straight, 4),
            (PathLateralStyle::Arc, 4),
            (PathLateralStyle::Serpentine, 1),
        ]),
    }
}

fn bhop_weave_cycles(style: PathLateralStyle, band: BhopDifficultyBand, rng: &mut RunRng) -> f32 {
    match style {
        PathLateralStyle::Straight => match band {
            BhopDifficultyBand::Intro => rng.range_f32(0.25, 0.55),
            BhopDifficultyBand::Mid => rng.range_f32(0.35, 0.8),
            BhopDifficultyBand::Advanced => rng.range_f32(0.45, 1.0),
        },
        PathLateralStyle::Arc => match band {
            BhopDifficultyBand::Intro => rng.range_f32(0.35, 0.8),
            BhopDifficultyBand::Mid => rng.range_f32(0.6, 1.1),
            BhopDifficultyBand::Advanced => rng.range_f32(0.8, 1.25),
        },
        PathLateralStyle::Serpentine => match band {
            BhopDifficultyBand::Intro => rng.range_f32(0.8, 1.3),
            BhopDifficultyBand::Mid => rng.range_f32(1.0, 1.9),
            BhopDifficultyBand::Advanced => rng.range_f32(1.35, 2.5),
        },
        PathLateralStyle::Switchback => match band {
            BhopDifficultyBand::Intro => rng.range_f32(0.8, 1.2),
            BhopDifficultyBand::Mid => rng.range_f32(1.2, 2.0),
            BhopDifficultyBand::Advanced => rng.range_f32(1.6, 2.8),
        },
        PathLateralStyle::OneSidedArc => rng.range_f32(0.2, 0.5),
    }
}

fn build_bhop_placement_plan(
    profile: &BhopProfile,
    rng: &mut RunRng,
    path_start: Vec3,
    path_end: Vec3,
    forward: Vec3,
    right: Vec3,
    rules: &BhopSpacingRules,
) -> Option<BhopPlacementPlan> {
    let mut pads = build_bhop_pad_specs(profile, rng);
    if !fit_bhop_pad_sizes_to_length(profile, &mut pads, path_start.distance(path_end), rules) {
        return None;
    }
    let gap_weights = build_bhop_gap_weights(profile, rng);
    let step_samples = place_bhop_step_samples(
        profile,
        &pads,
        path_start.distance(path_end),
        &gap_weights,
        rules,
    )?;
    let mut plan = BhopPlacementPlan {
        usable_length: path_start.distance(path_end).max(1.0),
        step_samples,
        pads,
    };
    materialize_bhop_placement_plan(
        profile, &mut plan, path_start, path_end, forward, right, rules,
    );
    Some(plan)
}

fn build_bhop_pad_specs(profile: &BhopProfile, rng: &mut RunRng) -> Vec<BhopPadSpec> {
    match profile.pattern {
        BhopPattern::Rhythm => build_rhythm_bhop_pads(profile, rng),
        BhopPattern::LaneSwitch => build_lane_switch_bhop_pads(profile, rng),
        BhopPattern::DropStrafe => build_drop_strafe_bhop_pads(profile, rng),
        BhopPattern::SplitMerge => build_split_merge_bhop_pads(profile, rng),
    }
}

fn build_rhythm_bhop_pads(profile: &BhopProfile, rng: &mut RunRng) -> Vec<BhopPadSpec> {
    (0..profile.pad_count)
        .map(|step| {
            let catch =
                bhop_is_catch_step(profile, step) || step == 0 || step + 1 == profile.pad_count;
            let beat_scale = match step % 4 {
                0 => 1.08,
                1 => 0.94,
                2 => 1.0,
                _ => 0.88,
            };
            make_bhop_pad(profile, rng, step, 0b01, catch, beat_scale, 1.0)
        })
        .collect()
}

fn build_lane_switch_bhop_pads(profile: &BhopProfile, rng: &mut RunRng) -> Vec<BhopPadSpec> {
    (0..profile.pad_count)
        .map(|step| {
            let catch =
                bhop_is_catch_step(profile, step) || step == 0 || step + 1 == profile.pad_count;
            make_bhop_pad(profile, rng, step, 0b01, catch, 1.0, 1.0)
        })
        .collect()
}

fn build_drop_strafe_bhop_pads(profile: &BhopProfile, rng: &mut RunRng) -> Vec<BhopPadSpec> {
    let last = profile.pad_count.saturating_sub(1).max(1);
    (0..profile.pad_count)
        .map(|step| {
            let t = step as f32 / last as f32;
            let basin = (1.0 - ((t - 0.5).abs() / 0.34).clamp(0.0, 1.0)).powf(1.7);
            let catch =
                bhop_is_catch_step(profile, step) || step <= 1 || step + 2 >= profile.pad_count;
            let size_scale = lerp(1.04, 0.82, basin);
            make_bhop_pad(profile, rng, step, 0b01, catch, size_scale, 1.0)
        })
        .collect()
}

fn build_split_merge_bhop_pads(profile: &BhopProfile, rng: &mut RunRng) -> Vec<BhopPadSpec> {
    let split_start = (profile.pad_count / 3).max(2);
    let merge_start = profile
        .pad_count
        .saturating_sub((profile.pad_count / 3).max(2));
    let mut pads = Vec::with_capacity(profile.pad_count + merge_start.saturating_sub(split_start));

    for step in 0..profile.pad_count {
        let shared = step < split_start || step >= merge_start;
        if shared {
            let catch = step == 0
                || step + 1 == profile.pad_count
                || step + 1 == split_start
                || step == merge_start;
            pads.push(make_bhop_pad(profile, rng, step, 0b11, catch, 1.02, 1.0));
            continue;
        }

        pads.push(make_bhop_pad(profile, rng, step, 0b01, false, 1.18, 1.04));
        pads.push(make_bhop_pad(profile, rng, step, 0b10, false, 0.82, 0.94));
    }

    pads
}

fn build_bhop_gap_weights(profile: &BhopProfile, rng: &mut RunRng) -> Vec<f32> {
    if profile.pad_count <= 1 {
        return Vec::new();
    }

    match profile.pattern {
        BhopPattern::Rhythm => rhythm_gap_weights(profile.pad_count, rng),
        BhopPattern::DropStrafe => (0..profile.pad_count - 1)
            .map(|gap| {
                let t = (gap + 1) as f32 / profile.pad_count as f32;
                1.0 + (1.0 - ((t - 0.5).abs() / 0.45).clamp(0.0, 1.0)) * 0.35
            })
            .collect(),
        _ => vec![1.0; profile.pad_count - 1],
    }
}

fn fit_bhop_pad_sizes_to_length(
    profile: &BhopProfile,
    pads: &mut [BhopPadSpec],
    usable_length: f32,
    rules: &BhopSpacingRules,
) -> bool {
    for _ in 0..BHOP_MAX_REPAIR_PASSES {
        let required_total = bhop_required_path_length(profile, pads, rules);
        if required_total <= usable_length {
            return true;
        }

        let overflow = required_total - usable_length;
        let shrinkable = pads
            .iter()
            .enumerate()
            .filter_map(|(index, pad)| {
                (!pad.catch && pad.side > bhop_min_side_for_repair(profile.band) + rules.epsilon)
                    .then_some(index)
            })
            .collect::<Vec<_>>();
        if shrinkable.is_empty() {
            return false;
        }

        let per_pad = overflow / shrinkable.len() as f32 + rules.epsilon;
        for index in shrinkable {
            let min_side = bhop_min_side_for_repair(profile.band);
            pads[index].side = (pads[index].side - per_pad).max(min_side);
        }
    }

    bhop_required_path_length(profile, pads, rules) <= usable_length
}

fn place_bhop_step_samples(
    profile: &BhopProfile,
    pads: &[BhopPadSpec],
    usable_length: f32,
    gap_weights: &[f32],
    rules: &BhopSpacingRules,
) -> Option<Vec<f32>> {
    if profile.pad_count == 0 {
        return Some(Vec::new());
    }
    if profile.pad_count == 1 {
        return Some(vec![0.5]);
    }

    let usable_length = usable_length.max(1.0);
    let (start_clearance, end_clearance, required_gaps) = bhop_path_budget(profile, pads, rules)?;
    let total_required = start_clearance + end_clearance + required_gaps.iter().sum::<f32>();
    if total_required > usable_length {
        return None;
    }

    let slack = (usable_length - total_required).max(0.0);
    let weight_sum = gap_weights.iter().sum::<f32>().max(0.001);
    let mut samples = Vec::with_capacity(profile.pad_count);
    let mut traveled = start_clearance;
    samples.push((traveled / usable_length).clamp(0.0, 1.0));
    for (index, required_gap) in required_gaps.into_iter().enumerate() {
        let extra = slack * gap_weights.get(index).copied().unwrap_or(1.0) / weight_sum;
        traveled += required_gap + extra;
        samples.push((traveled / usable_length).clamp(0.0, 1.0));
    }
    Some(samples)
}

fn bhop_path_budget(
    profile: &BhopProfile,
    pads: &[BhopPadSpec],
    rules: &BhopSpacingRules,
) -> Option<(f32, f32, Vec<f32>)> {
    let route_masks = bhop_route_masks(profile.pattern);
    let start_clearance = bhop_step_half_extent(pads, 0) + BHOP_MIN_ANCHOR_EDGE_CLEARANCE;
    let end_clearance =
        bhop_step_half_extent(pads, profile.pad_count - 1) + BHOP_MIN_ANCHOR_EDGE_CLEARANCE;
    let mut required_gaps = Vec::with_capacity(profile.pad_count - 1);
    for step in 1..profile.pad_count {
        let mut required: f32 = 0.0;
        for route_mask in route_masks {
            let prev = route_pad_for_step(pads, step - 1, *route_mask)?;
            let curr = route_pad_for_step(pads, step, *route_mask)?;
            required = required.max(bhop_required_center_gap(
                prev.side,
                curr.side,
                rules.min_route_edge_gap,
            ));
        }
        required_gaps.push(required);
    }
    Some((start_clearance, end_clearance, required_gaps))
}

fn bhop_required_path_length(
    profile: &BhopProfile,
    pads: &[BhopPadSpec],
    rules: &BhopSpacingRules,
) -> f32 {
    bhop_path_budget(profile, pads, rules)
        .map(|(start_clearance, end_clearance, required_gaps)| {
            start_clearance + end_clearance + required_gaps.iter().sum::<f32>()
        })
        .unwrap_or(f32::INFINITY)
}

fn materialize_bhop_placement_plan(
    profile: &BhopProfile,
    plan: &mut BhopPlacementPlan,
    path_start: Vec3,
    path_end: Vec3,
    forward: Vec3,
    right: Vec3,
    rules: &BhopSpacingRules,
) {
    let base_tops = sample_descending_platform_tops_with_samples(
        path_start,
        path_end,
        right,
        &plan.step_samples,
        profile.path_style,
        profile.weave_cycles,
        profile.phase,
        profile.lateral_amplitude,
        profile.vertical_wave,
    );

    for pad in &mut plan.pads {
        pad.top = base_tops
            .get(pad.step)
            .copied()
            .unwrap_or_else(|| path_start.lerp(path_end, 0.5));
    }

    match profile.pattern {
        BhopPattern::Rhythm => {}
        BhopPattern::LaneSwitch => materialize_lane_switch_bhop_pads(profile, plan, right),
        BhopPattern::DropStrafe => materialize_drop_strafe_bhop_pads(profile, plan),
        BhopPattern::SplitMerge => {
            materialize_split_merge_bhop_pads(profile, plan, forward, right, rules)
        }
    }
}

fn materialize_lane_switch_bhop_pads(
    profile: &BhopProfile,
    plan: &mut BhopPlacementPlan,
    right: Vec3,
) {
    let lane_cycles = match profile.band {
        BhopDifficultyBand::Intro => 0.0,
        BhopDifficultyBand::Mid => 1.7,
        BhopDifficultyBand::Advanced => 2.6,
    };

    for pad in &mut plan.pads {
        let t = plan.step_samples.get(pad.step).copied().unwrap_or(0.5);
        let envelope = (t * PI).sin().max(0.0).powf(0.76);
        let lane_shift =
            (t * lane_cycles * TAU + profile.phase * 0.9).sin() * profile.lane_offset * envelope;
        pad.top += right * lane_shift;
    }
}

fn materialize_drop_strafe_bhop_pads(profile: &BhopProfile, plan: &mut BhopPlacementPlan) {
    let last = profile.pad_count.saturating_sub(1).max(1);
    for pad in &mut plan.pads {
        let t = pad.step as f32 / last as f32;
        let basin = (1.0 - ((t - 0.5).abs() / 0.34).clamp(0.0, 1.0)).powf(1.7);
        pad.top -= Vec3::Y * profile.drop_depth * basin;
    }
}

fn materialize_split_merge_bhop_pads(
    profile: &BhopProfile,
    plan: &mut BhopPlacementPlan,
    _forward: Vec3,
    right: Vec3,
    rules: &BhopSpacingRules,
) {
    for step in 0..profile.pad_count {
        let indices = plan
            .pads
            .iter()
            .enumerate()
            .filter_map(|(index, pad)| (pad.step == step).then_some(index))
            .collect::<Vec<_>>();
        if indices.len() < 2 {
            continue;
        }

        let safe_index = indices
            .iter()
            .copied()
            .find(|index| plan.pads[*index].route_mask == 0b01);
        let risk_index = indices
            .iter()
            .copied()
            .find(|index| plan.pads[*index].route_mask == 0b10);
        let (Some(safe_index), Some(risk_index)) = (safe_index, risk_index) else {
            continue;
        };
        let separation = solve_split_merge_lane_offset(
            plan.pads[safe_index].side,
            plan.pads[risk_index].side,
            profile.lane_offset,
            rules,
        );
        let safe_offset = separation * 0.44;
        let risk_offset = separation * 0.56;
        let base = plan.pads[safe_index]
            .top
            .lerp(plan.pads[risk_index].top, 0.5);
        plan.pads[safe_index].top = base - right * safe_offset;
        plan.pads[risk_index].top = base + right * risk_offset;
    }
}

fn validate_bhop_spacing(
    profile: &BhopProfile,
    pads: &[BhopPadSpec],
    rules: &BhopSpacingRules,
) -> Vec<BhopSpacingIssue> {
    let mut issues = Vec::new();

    for route_mask in bhop_route_masks(profile.pattern) {
        for step in 1..profile.pad_count {
            let Some(prev_index) = route_pad_index_for_step(pads, step - 1, *route_mask) else {
                continue;
            };
            let Some(curr_index) = route_pad_index_for_step(pads, step, *route_mask) else {
                continue;
            };
            let gap = bhop_edge_gap(&pads[prev_index], &pads[curr_index]);
            if gap + rules.epsilon < rules.min_route_edge_gap {
                issues.push(BhopSpacingIssue {
                    kind: BhopSpacingIssueKind::RouteGap,
                    left: prev_index,
                    right: curr_index,
                    deficit: rules.min_route_edge_gap - gap,
                    earlier_step: step - 1,
                    later_step: step,
                });
            }
        }
    }

    for step in 0..profile.pad_count {
        let step_indices = pads
            .iter()
            .enumerate()
            .filter_map(|(index, pad)| (pad.step == step).then_some(index))
            .collect::<Vec<_>>();
        for left_offset in 0..step_indices.len() {
            for right_offset in left_offset + 1..step_indices.len() {
                let left = step_indices[left_offset];
                let right = step_indices[right_offset];
                let gap = bhop_edge_gap(&pads[left], &pads[right]);
                if gap + rules.epsilon < rules.min_branch_edge_gap {
                    issues.push(BhopSpacingIssue {
                        kind: BhopSpacingIssueKind::BranchGap,
                        left,
                        right,
                        deficit: rules.min_branch_edge_gap - gap,
                        earlier_step: step,
                        later_step: step,
                    });
                }
            }
        }
    }

    for left in 0..pads.len() {
        for right in left + 1..pads.len() {
            let left_pad = &pads[left];
            let right_pad = &pads[right];
            let adjacent_same_route = left_pad.step.abs_diff(right_pad.step) == 1
                && left_pad.route_mask & right_pad.route_mask != 0;
            let branch_pair = left_pad.step == right_pad.step;
            if adjacent_same_route || branch_pair {
                continue;
            }

            let gap = bhop_edge_gap(left_pad, right_pad);
            if gap + rules.epsilon < 0.0 {
                issues.push(BhopSpacingIssue {
                    kind: BhopSpacingIssueKind::Overlap,
                    left,
                    right,
                    deficit: -gap,
                    earlier_step: left_pad.step.min(right_pad.step),
                    later_step: left_pad.step.max(right_pad.step),
                });
            }
        }
    }

    issues.sort_by(|left, right| {
        left.later_step
            .cmp(&right.later_step)
            .then(left.earlier_step.cmp(&right.earlier_step))
            .then_with(|| right.deficit.total_cmp(&left.deficit))
    });
    issues
}

fn repair_bhop_spacing(
    profile: &BhopProfile,
    plan: &mut BhopPlacementPlan,
    path_forward: Vec3,
    right: Vec3,
    rules: &BhopSpacingRules,
) -> bool {
    for _ in 0..BHOP_MAX_REPAIR_PASSES {
        let issues = validate_bhop_spacing(profile, &plan.pads, rules);
        if issues.is_empty() {
            return true;
        }

        let separated = separate_bhop_spacing_issues(plan, path_forward, right, &issues, rules);
        let issues = validate_bhop_spacing(profile, &plan.pads, rules);
        if issues.is_empty() {
            return true;
        }

        let shrunk = shrink_bhop_spacing_issues(profile, &mut plan.pads, &issues, rules);
        if !separated && !shrunk {
            break;
        }
    }

    validate_bhop_spacing(profile, &plan.pads, rules).is_empty()
}

fn separate_bhop_spacing_issues(
    plan: &mut BhopPlacementPlan,
    path_forward: Vec3,
    right: Vec3,
    issues: &[BhopSpacingIssue],
    rules: &BhopSpacingRules,
) -> bool {
    let max_step = plan.pads.iter().map(|pad| pad.step).max().unwrap_or(0);
    let mut route_shift_starts = vec![0.0_f32; max_step + 1];
    let mut branch_shift_by_step = vec![0.0_f32; max_step + 1];
    let mut changed = false;

    for issue in issues {
        match issue.kind {
            BhopSpacingIssueKind::BranchGap => {
                branch_shift_by_step[issue.later_step] =
                    branch_shift_by_step[issue.later_step].max(issue.deficit * 0.5 + rules.epsilon);
            }
            BhopSpacingIssueKind::RouteGap | BhopSpacingIssueKind::Overlap => {
                if issue.later_step <= max_step {
                    route_shift_starts[issue.later_step] += issue.deficit + rules.epsilon;
                }
            }
        }
    }

    let mut cumulative_route_shift = 0.0;
    for step in 0..=max_step {
        cumulative_route_shift += route_shift_starts[step];
        if cumulative_route_shift > 0.0 {
            changed = true;
        }

        let branch_shift = branch_shift_by_step[step];
        if branch_shift > 0.0 {
            changed = true;
        }

        for pad in plan.pads.iter_mut().filter(|pad| pad.step == step) {
            if cumulative_route_shift > 0.0 {
                pad.top += path_forward * cumulative_route_shift;
            }
            if branch_shift > 0.0 {
                match pad.route_mask {
                    0b01 => pad.top -= right * branch_shift,
                    0b10 => pad.top += right * branch_shift,
                    _ => {}
                }
            }
        }
    }

    changed
}

fn shrink_bhop_spacing_issues(
    profile: &BhopProfile,
    pads: &mut [BhopPadSpec],
    issues: &[BhopSpacingIssue],
    rules: &BhopSpacingRules,
) -> bool {
    let mut shrink_requests = vec![0.0; pads.len()];
    for issue in issues {
        let mut shrinkable = Vec::new();
        if !pads[issue.left].catch && pads[issue.left].side > bhop_min_side_for_repair(profile.band)
        {
            shrinkable.push(issue.left);
        }
        if !pads[issue.right].catch
            && pads[issue.right].side > bhop_min_side_for_repair(profile.band)
        {
            shrinkable.push(issue.right);
        }
        if shrinkable.is_empty() {
            continue;
        }

        let per_pad = ((issue.deficit * 2.0) / shrinkable.len() as f32) + rules.epsilon;
        for index in shrinkable {
            shrink_requests[index] += per_pad;
        }
    }

    let mut changed = false;
    for (index, requested) in shrink_requests.into_iter().enumerate() {
        if requested <= 0.0 {
            continue;
        }
        let min_side = bhop_min_side_for_repair(profile.band);
        let new_side = (pads[index].side - requested).max(min_side);
        if new_side + rules.epsilon < pads[index].side {
            pads[index].side = new_side;
            changed = true;
        }
    }

    changed
}

fn bhop_route_masks(pattern: BhopPattern) -> &'static [u8] {
    match pattern {
        BhopPattern::SplitMerge => &[0b01, 0b10],
        _ => &[0b01],
    }
}

fn route_pad_for_step<'a>(
    pads: &'a [BhopPadSpec],
    step: usize,
    route_mask: u8,
) -> Option<&'a BhopPadSpec> {
    route_pad_index_for_step(pads, step, route_mask).map(|index| &pads[index])
}

fn route_pad_index_for_step(pads: &[BhopPadSpec], step: usize, route_mask: u8) -> Option<usize> {
    pads.iter()
        .position(|pad| pad.step == step && pad.route_mask & route_mask != 0)
}

fn bhop_required_center_gap(left_side: f32, right_side: f32, min_edge_gap: f32) -> f32 {
    left_side * 0.5 + right_side * 0.5 + min_edge_gap
}

fn bhop_step_half_extent(pads: &[BhopPadSpec], step: usize) -> f32 {
    pads.iter()
        .filter(|pad| pad.step == step)
        .map(|pad| pad.side * 0.5)
        .fold(0.0, f32::max)
}

fn bhop_edge_gap(left: &BhopPadSpec, right: &BhopPadSpec) -> f32 {
    left.top.xz().distance(right.top.xz()) - (left.side + right.side) * 0.5
}

fn solve_split_merge_lane_offset(
    safe_side: f32,
    risk_side: f32,
    desired_lane_offset: f32,
    rules: &BhopSpacingRules,
) -> f32 {
    let required_center_gap =
        bhop_required_center_gap(safe_side, risk_side, rules.min_branch_edge_gap);
    (desired_lane_offset * 2.0).max(required_center_gap)
}

fn make_bhop_pad(
    profile: &BhopProfile,
    rng: &mut RunRng,
    step: usize,
    route_mask: u8,
    catch: bool,
    side_scale: f32,
    height_scale: f32,
) -> BhopPadSpec {
    let accent = !catch && step % 3 == 0;
    let side = bhop_pad_side(profile.band, catch, accent, rng) * side_scale;
    let height = bhop_pad_height(profile.band, catch, rng) * height_scale;
    BhopPadSpec {
        step,
        route_mask,
        top: Vec3::ZERO,
        side,
        height,
        catch,
    }
}

fn bhop_pad_side(band: BhopDifficultyBand, catch: bool, accent: bool, rng: &mut RunRng) -> f32 {
    let side = match (band, catch, accent) {
        (BhopDifficultyBand::Intro, true, _) => rng.range_f32(4.6, 5.8),
        (BhopDifficultyBand::Intro, false, true) => rng.range_f32(3.6, 4.4),
        (BhopDifficultyBand::Intro, false, false) => rng.range_f32(3.0, 3.8),
        (BhopDifficultyBand::Mid, true, _) => rng.range_f32(4.2, 5.4),
        (BhopDifficultyBand::Mid, false, true) => rng.range_f32(3.1, 3.9),
        (BhopDifficultyBand::Mid, false, false) => rng.range_f32(2.3, 3.2),
        (BhopDifficultyBand::Advanced, true, _) => rng.range_f32(3.9, 5.0),
        (BhopDifficultyBand::Advanced, false, true) => rng.range_f32(2.6, 3.4),
        (BhopDifficultyBand::Advanced, false, false) => rng.range_f32(1.8, 2.7),
    };
    scaled_bhop_size(side).max(scaled_bhop_size(1.6))
}

fn bhop_pad_height(band: BhopDifficultyBand, catch: bool, rng: &mut RunRng) -> f32 {
    let height = match (band, catch) {
        (BhopDifficultyBand::Intro, true) => rng.range_f32(0.94, 1.12),
        (BhopDifficultyBand::Intro, false) => rng.range_f32(0.78, 0.96),
        (BhopDifficultyBand::Mid, true) => rng.range_f32(0.9, 1.08),
        (BhopDifficultyBand::Mid, false) => rng.range_f32(0.74, 0.9),
        (BhopDifficultyBand::Advanced, true) => rng.range_f32(0.86, 1.04),
        (BhopDifficultyBand::Advanced, false) => rng.range_f32(0.68, 0.86),
    };
    scaled_bhop_size(height) * 0.45
}

fn bhop_is_catch_step(profile: &BhopProfile, step: usize) -> bool {
    step == 0 || step + 1 == profile.pad_count || step % profile.catch_interval == 0
}

fn rhythm_gap_weights(count: usize, rng: &mut RunRng) -> Vec<f32> {
    if count <= 1 {
        return Vec::new();
    }
    let short = rng.range_f32(0.7, 0.86);
    let long = rng.range_f32(1.08, 1.28);
    let settle = rng.range_f32(0.9, 1.06);
    let mut weights = Vec::with_capacity(count - 1);
    for gap in 0..count - 1 {
        weights.push(match gap % 4 {
            0 => 1.0,
            1 => short,
            2 => long,
            _ => settle,
        });
    }
    weights
}

fn bhop_min_side_for_repair(band: BhopDifficultyBand) -> f32 {
    match band {
        BhopDifficultyBand::Intro => scaled_bhop_size(2.8),
        BhopDifficultyBand::Mid => scaled_bhop_size(2.0),
        BhopDifficultyBand::Advanced => scaled_bhop_size(1.6),
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
    let entry_margin = SURF_ENTRY_MARGIN;
    let exit_margin = if intense {
        (direct_distance * 0.07).clamp(SURF_EXIT_MARGIN_MIN, SURF_EXIT_MARGIN_MAX)
    } else {
        (direct_distance * 0.06).clamp(SURF_EXIT_MARGIN_MIN, SURF_EXIT_MARGIN_MAX - 1.0)
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

fn sample_descending_platform_tops_with_samples(
    start: Vec3,
    end: Vec3,
    right: Vec3,
    samples: &[f32],
    style: PathLateralStyle,
    weave_cycles: f32,
    phase: f32,
    lateral_amplitude: f32,
    vertical_wave: f32,
) -> Vec<Vec3> {
    if samples.is_empty() {
        return Vec::new();
    }

    let last = samples.len().saturating_sub(1);
    let mut points = Vec::with_capacity(samples.len());
    for (step, sample) in samples.iter().copied().enumerate() {
        let t = sample.clamp(0.0, 1.0);
        let envelope = (t * PI).sin().max(0.0).powf(0.72);
        let endpoint_factor = match step.min(last.saturating_sub(step)) {
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
    uvs: Vec<[f32; 2]>,
}

impl ColoredMeshBuilder {
    fn push_triangle_uv_outward(
        &mut self,
        hull_center: Vec3,
        a: Vec3,
        b: Vec3,
        c: Vec3,
        uvs: [Vec2; 3],
        color: Color,
    ) {
        let mut points = [(a, uvs[0]), (b, uvs[1]), (c, uvs[2])];
        let mut normal = outward_triangle_normal(a, b, c, hull_center);
        let triangle_center = (a + b + c) / 3.0;
        if normal.dot(triangle_center - hull_center) < 0.0 {
            points.swap(1, 2);
            normal = outward_triangle_normal(points[0].0, points[1].0, points[2].0, hull_center);
        }

        let color = LinearRgba::from(color).to_f32_array();
        for (point, uv) in points {
            self.positions.push([point.x, point.y, point.z]);
            self.normals.push([normal.x, normal.y, normal.z]);
            self.colors.push(color);
            self.uvs.push([uv.x, uv.y]);
        }
    }

    fn push_quad_outward(
        &mut self,
        hull_center: Vec3,
        a: Vec3,
        b: Vec3,
        c: Vec3,
        d: Vec3,
        color: Color,
    ) {
        self.push_quad_uv_outward(
            hull_center,
            a,
            b,
            c,
            d,
            [
                Vec2::new(0.0, 0.0),
                Vec2::new(1.0, 0.0),
                Vec2::new(1.0, 1.0),
                Vec2::new(0.0, 1.0),
            ],
            color,
        );
    }

    fn push_quad_uv_outward(
        &mut self,
        hull_center: Vec3,
        a: Vec3,
        b: Vec3,
        c: Vec3,
        d: Vec3,
        uvs: [Vec2; 4],
        color: Color,
    ) {
        self.push_triangle_uv_outward(hull_center, a, b, c, [uvs[0], uvs[1], uvs[2]], color);
        self.push_triangle_uv_outward(hull_center, a, c, d, [uvs[0], uvs[2], uvs[3]], color);
    }

    fn build(self) -> Mesh {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colors)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs)
    }
}

fn outward_triangle_normal(a: Vec3, b: Vec3, c: Vec3, hull_center: Vec3) -> Vec3 {
    let normal = (b - a).cross(c - a).normalize_or_zero();
    if normal != Vec3::ZERO {
        return normal;
    }

    let fallback = ((a + b + c) / 3.0 - hull_center).normalize_or_zero();
    if fallback == Vec3::ZERO {
        Vec3::Y
    } else {
        fallback
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

    builder.push_quad_outward(center, e, g, h, f, underside);
    builder.push_quad_outward(center, a, c, g, e, outer_face);
    builder.push_quad_outward(center, b, f, h, d, side_shadow);
    builder.push_quad_outward(center, a, e, f, b, deepen(base_color, 0.08));
    builder.push_quad_outward(center, c, d, h, g, deepen(base_color, 0.14));

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
        builder.push_quad_uv_outward(
            center,
            front_ridge.lerp(front_outer, start_t),
            back_ridge.lerp(back_outer, start_t),
            back_ridge.lerp(back_outer, end_t),
            front_ridge.lerp(front_outer, end_t),
            [
                Vec2::new(0.0, start_t),
                Vec2::new(1.0, start_t),
                Vec2::new(1.0, end_t),
                Vec2::new(0.0, end_t),
            ],
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

    fn fork(&self, salt: u64) -> Self {
        Self {
            state: self.state ^ salt.wrapping_mul(0xD6E8_FD50_1A7F_6D4B),
        }
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
    use std::collections::HashSet;

    fn test_room(index: usize, top: Vec3) -> RoomPlan {
        RoomPlan { index, top }
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

    fn test_bhop_rooms() -> Vec<RoomPlan> {
        vec![
            test_room(0, Vec3::new(0.0, 120.0, 0.0)),
            test_room(1, Vec3::new(118.0, 92.0, 22.0)),
        ]
    }

    fn bhop_path_frame(rooms: &[RoomPlan]) -> (Vec3, Vec3, Vec3, Vec3) {
        let start = rooms[0].top;
        let end = rooms[1].top;
        let forward = direction_from_delta(end - start);
        let right = Vec3::new(-forward.z, 0.0, forward.x).normalize_or_zero();
        let distance = start.distance(end).max(18.0);
        let start_margin =
            bhop_path_margin(distance, BHOP_ANCHOR_MARGIN_MIN, BHOP_ANCHOR_MARGIN_MAX);
        let end_margin = bhop_path_margin(distance, BHOP_ANCHOR_MARGIN_MIN, BHOP_ANCHOR_MARGIN_MAX);
        let path_start = start + forward * start_margin
            - Vec3::Y * (BHOP_SURF_ALIGNMENT_DROP + BHOP_VERTICAL_CLEARANCE_BIAS);
        let path_end = end
            - forward * end_margin
            - Vec3::Y * (BHOP_SURF_ALIGNMENT_DROP + BHOP_VERTICAL_CLEARANCE_BIAS);
        (path_start, path_end, forward, right)
    }

    fn build_test_bhop_section(
        segment_index: usize,
        seed: u64,
    ) -> (BhopProfile, Vec<BhopPadSpec>, Vec<RoomPlan>) {
        let rooms = test_bhop_rooms();
        let (profile, pads) = build_test_bhop_section_in_rooms(segment_index, seed, &rooms);
        (profile, pads, rooms)
    }

    fn build_test_bhop_section_in_rooms(
        segment_index: usize,
        seed: u64,
        rooms: &[RoomPlan],
    ) -> (BhopProfile, Vec<BhopPadSpec>) {
        let from = &rooms[0];
        let to = &rooms[1];
        let forward = direction_from_delta(to.top - from.top);
        let right = Vec3::new(-forward.z, 0.0, forward.x).normalize_or_zero();
        let mut rng = RunRng::new(seed);
        build_square_bhop_pads(segment_index, &mut rng, from.top, to.top, forward, right)
    }

    fn forced_bhop_profile(pattern: BhopPattern, band: BhopDifficultyBand) -> BhopProfile {
        BhopProfile {
            band,
            pattern,
            pad_count: if pattern == BhopPattern::SplitMerge {
                8
            } else {
                6
            },
            min_pad_count: if pattern == BhopPattern::SplitMerge {
                8
            } else {
                5
            },
            path_style: match pattern {
                BhopPattern::Rhythm => PathLateralStyle::Straight,
                BhopPattern::LaneSwitch => PathLateralStyle::Serpentine,
                BhopPattern::DropStrafe => PathLateralStyle::Arc,
                BhopPattern::SplitMerge => PathLateralStyle::Straight,
            },
            weave_cycles: 1.6,
            phase: 0.8,
            lateral_amplitude: match band {
                BhopDifficultyBand::Intro => scaled_bhop_size(0.5),
                BhopDifficultyBand::Mid => scaled_bhop_size(0.95),
                BhopDifficultyBand::Advanced => scaled_bhop_size(1.25),
            },
            vertical_wave: scaled_bhop_size(0.14),
            lane_offset: match band {
                BhopDifficultyBand::Intro => 0.0,
                BhopDifficultyBand::Mid => scaled_bhop_size(0.95),
                BhopDifficultyBand::Advanced => scaled_bhop_size(1.25),
            },
            drop_depth: match band {
                BhopDifficultyBand::Intro => 0.0,
                BhopDifficultyBand::Mid => scaled_bhop_size(0.7),
                BhopDifficultyBand::Advanced => scaled_bhop_size(1.1),
            },
            catch_interval: match band {
                BhopDifficultyBand::Intro => 3,
                BhopDifficultyBand::Mid => 4,
                BhopDifficultyBand::Advanced => 7,
            },
        }
    }

    fn build_forced_bhop_section(
        profile: BhopProfile,
        seed: u64,
    ) -> (BhopProfile, Vec<BhopPadSpec>, Vec<RoomPlan>) {
        let rooms = test_bhop_rooms();
        let (profile, pads) = build_forced_bhop_section_in_rooms(profile, seed, &rooms);
        (profile, pads, rooms)
    }

    fn build_forced_bhop_section_in_rooms(
        profile: BhopProfile,
        seed: u64,
        rooms: &[RoomPlan],
    ) -> (BhopProfile, Vec<BhopPadSpec>) {
        let (path_start, path_end, forward, right) = bhop_path_frame(rooms);
        let rules = BhopSpacingRules::default();
        let mut profile = profile;
        let mut plan = None;
        while profile.pad_count >= 2 && plan.is_none() {
            let mut rng = RunRng::new(seed ^ profile.pad_count as u64);
            plan = build_bhop_placement_plan(
                &profile, &mut rng, path_start, path_end, forward, right, &rules,
            );
            if plan.is_none() && profile.pad_count > 2 {
                profile.pad_count -= 1;
            } else {
                break;
            }
        }
        let mut plan = plan.expect("forced bhop section should fit");
        assert!(repair_bhop_spacing(
            &profile, &mut plan, forward, right, &rules
        ));
        (profile, plan.pads)
    }

    fn route_covers_all_steps(pads: &[BhopPadSpec], route_mask: u8, step_count: usize) -> bool {
        (0..step_count).all(|step| {
            pads.iter()
                .any(|pad| pad.step == step && pad.route_mask & route_mask != 0)
        })
    }

    fn assert_has_complete_route(profile: &BhopProfile, pads: &[BhopPadSpec]) {
        assert!(
            route_covers_all_steps(pads, 0b01, profile.pad_count)
                || route_covers_all_steps(pads, 0b10, profile.pad_count),
            "expected at least one start-to-finish route for {:?}",
            profile.pattern
        );
    }

    fn assert_boundary_catches(profile: &BhopProfile, pads: &[BhopPadSpec]) {
        assert!(pads.iter().any(|pad| pad.step == 0 && pad.catch));
        assert!(
            pads.iter()
                .any(|pad| pad.step + 1 == profile.pad_count && pad.catch)
        );
    }

    fn assert_anchor_clearance(profile: &BhopProfile, pads: &[BhopPadSpec], rooms: &[RoomPlan]) {
        let start_pad = pads
            .iter()
            .find(|pad| pad.step == 0)
            .expect("expected start pad");
        let end_pad = pads
            .iter()
            .rev()
            .find(|pad| pad.step + 1 == profile.pad_count)
            .expect("expected end pad");
        let forward = direction_from_delta(rooms[1].top - rooms[0].top);
        let start_clearance = (start_pad.top - rooms[0].top).dot(forward) - start_pad.side * 0.5;
        let end_clearance = (rooms[1].top - end_pad.top).dot(forward) - end_pad.side * 0.5;

        assert!(
            start_clearance > 0.5,
            "start clearance too small: {start_clearance}"
        );
        assert!(
            end_clearance > 0.5,
            "end clearance too small: {end_clearance}"
        );
    }

    fn average_pad_side(pads: &[BhopPadSpec]) -> f32 {
        pads.iter().map(|pad| pad.side).sum::<f32>() / pads.len().max(1) as f32
    }

    fn catch_pad_count(pads: &[BhopPadSpec]) -> usize {
        pads.iter().filter(|pad| pad.catch).count()
    }

    fn catch_pad_rate(pads: &[BhopPadSpec]) -> f32 {
        catch_pad_count(pads) as f32 / pads.len().max(1) as f32
    }

    fn spacing_issues(profile: &BhopProfile, pads: &[BhopPadSpec]) -> Vec<BhopSpacingIssue> {
        validate_bhop_spacing(profile, pads, &BhopSpacingRules::default())
    }

    fn route_gaps_for_mask(
        profile: &BhopProfile,
        pads: &[BhopPadSpec],
        route_mask: u8,
    ) -> Vec<f32> {
        let mut gaps = Vec::new();
        for step in 1..profile.pad_count {
            let prev = route_pad_for_step(pads, step - 1, route_mask).expect("expected route pad");
            let curr = route_pad_for_step(pads, step, route_mask).expect("expected route pad");
            gaps.push(bhop_edge_gap(prev, curr));
        }
        gaps
    }

    fn min_route_gap(profile: &BhopProfile, pads: &[BhopPadSpec]) -> f32 {
        bhop_route_masks(profile.pattern)
            .iter()
            .flat_map(|route_mask| route_gaps_for_mask(profile, pads, *route_mask))
            .fold(f32::INFINITY, f32::min)
    }

    fn min_branch_gap(profile: &BhopProfile, pads: &[BhopPadSpec]) -> Option<f32> {
        let mut min_gap: Option<f32> = None;
        for step in 0..profile.pad_count {
            let step_pads = pads
                .iter()
                .filter(|pad| pad.step == step)
                .collect::<Vec<_>>();
            for left in 0..step_pads.len() {
                for right in left + 1..step_pads.len() {
                    let gap = bhop_edge_gap(step_pads[left], step_pads[right]);
                    min_gap = Some(min_gap.map_or(gap, |current| current.min(gap)));
                }
            }
        }
        min_gap
    }

    fn surf_end_ridge(layout: &ModuleLayout) -> Option<Vec3> {
        layout
            .solids
            .iter()
            .filter_map(|solid| match &solid.body {
                SolidBody::StaticSurfWedge {
                    wall_side,
                    render_points,
                } if *wall_side > 0.0 && render_points.len() >= 2 => {
                    Some(solid.center + render_points[1])
                }
                _ => None,
            })
            .last()
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
        let expected_spawn =
            square_bhop_spawn_position(&layout).expect("first bhop segment should emit platforms");

        assert_eq!(blueprint.spawn, expected_spawn);
    }

    #[test]
    fn initial_spawn_look_faces_into_the_course() {
        let blueprint = build_run_blueprint(0xA11C_E123);
        let look = spawn_look_for_blueprint(&blueprint);
        let facing = spawn_facing_for_blueprint(&blueprint);
        let forward = look.to_quat() * Vec3::NEG_Z;

        assert!(
            forward.dot(facing) > 0.98,
            "{forward:?} should face {facing:?}"
        );
    }

    #[test]
    fn surf_first_blueprint_spawns_above_surf_face() {
        let rooms = vec![
            test_room(0, Vec3::new(0.0, 120.0, 0.0)),
            test_room(1, Vec3::new(140.0, 88.0, 24.0)),
        ];
        let segment = test_segment(0, SegmentKind::SurfRamp, 0xC001_CAFE);
        let layout = build_segment_layout(&segment, &rooms);
        let expected =
            surf_spawn_position(&layout).expect("surf segment should provide a spawn point");

        assert!(expected.y > rooms[0].top.y);
    }

    #[test]
    fn build_room_layout_is_empty() {
        let room = test_room(3, Vec3::new(0.0, 32.0, 0.0));
        let layout = build_room_layout(&room);

        assert!(layout.solids.is_empty());
    }

    #[test]
    fn append_run_blueprint_keeps_only_surf_and_square_bhop_sections() {
        let blueprint = build_run_blueprint(0xC001_CAFE);
        let mut blueprint = blueprint.clone();
        append_run_blueprint(&mut blueprint, 6);

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
    fn square_bhop_layout_emits_only_square_static_platforms() {
        let rooms = vec![
            test_room(0, Vec3::new(0.0, 120.0, 0.0)),
            test_room(1, Vec3::new(92.0, 92.0, 18.0)),
        ];
        let layout = build_segment_layout(
            &test_segment(0, SegmentKind::SquareBhop, 0xBEEF_CAFE),
            &rooms,
        );

        assert!(!layout.solids.is_empty());
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
    fn square_bhop_layout_keeps_platforms_clear_of_room_anchors() {
        let rooms = vec![
            test_room(0, Vec3::new(0.0, 120.0, 0.0)),
            test_room(1, Vec3::new(118.0, 92.0, 22.0)),
        ];
        let layout = build_segment_layout(
            &test_segment(0, SegmentKind::SquareBhop, 0xBEEF_CAFE),
            &rooms,
        );
        let platforms = layout
            .solids
            .iter()
            .filter(|solid| matches!(solid.body, SolidBody::Static))
            .collect::<Vec<_>>();
        let first = platforms.first().expect("expected first platform");
        let last = platforms.last().expect("expected last platform");
        let forward = direction_from_delta(rooms[1].top - rooms[0].top);

        let start_clearance = (first.center - rooms[0].top).dot(forward) - first.size.x * 0.5;
        let end_clearance = (rooms[1].top - last.center).dot(forward) - last.size.x * 0.5;

        assert!(
            start_clearance > 0.5,
            "start clearance too small: {start_clearance}"
        );
        assert!(
            end_clearance > 0.5,
            "end clearance too small: {end_clearance}"
        );
    }

    #[test]
    fn square_bhop_layout_sits_below_room_anchor_height() {
        let rooms = vec![
            test_room(0, Vec3::new(0.0, 120.0, 0.0)),
            test_room(1, Vec3::new(118.0, 92.0, 22.0)),
        ];
        let layout = build_segment_layout(
            &test_segment(0, SegmentKind::SquareBhop, 0xBEEF_CAFE),
            &rooms,
        );
        let platforms = layout
            .solids
            .iter()
            .filter(|solid| matches!(solid.body, SolidBody::Static))
            .collect::<Vec<_>>();
        let first = platforms.first().expect("expected first platform");
        let last = platforms.last().expect("expected last platform");

        let first_top = first.center.y + first.size.y * 0.5;
        let last_top = last.center.y + last.size.y * 0.5;

        assert!(
            rooms[0].top.y - first_top > 8.0,
            "first bhop top should sit noticeably below the room anchor"
        );
        assert!(
            rooms[1].top.y - last_top > 8.0,
            "last bhop top should sit noticeably below the room anchor"
        );
    }

    #[test]
    fn surf_to_bhop_transition_stays_close_to_shared_anchor() {
        let rooms = vec![
            test_room(0, Vec3::new(0.0, 132.0, 0.0)),
            test_room(1, Vec3::new(138.0, 102.0, 24.0)),
            test_room(2, Vec3::new(244.0, 78.0, 54.0)),
        ];
        let surf_segment = SegmentPlan {
            index: 1,
            from: 0,
            to: 1,
            kind: SegmentKind::SurfRamp,
            seed: 0x5EED_CAFE,
        };
        let bhop_segment = SegmentPlan {
            index: 2,
            from: 1,
            to: 2,
            kind: SegmentKind::SquareBhop,
            seed: 0xBEEF_CAFE,
        };
        let surf_layout = build_segment_layout(&surf_segment, &rooms);
        let bhop_layout = build_segment_layout(&bhop_segment, &rooms);

        let surf_end = surf_end_ridge(&surf_layout).expect("surf layout should have an end ridge");
        let bhop_pad = bhop_layout
            .solids
            .iter()
            .find(|solid| matches!(solid.body, SolidBody::Static))
            .expect("bhop layout should have a first platform");
        let bhop_forward = direction_from_delta(rooms[2].top - rooms[1].top);
        let bhop_room_facing_edge = bhop_pad.center - bhop_forward * (bhop_pad.size.x * 0.5);

        assert!(
            surf_end.xz().distance(rooms[1].top.xz()) <= 10.5,
            "surf exit should stay near the shared room anchor"
        );
        assert!(
            bhop_room_facing_edge.xz().distance(rooms[1].top.xz()) <= 11.0,
            "bhop entry should stay near the shared room anchor"
        );
        assert!(
            surf_end.xz().distance(bhop_room_facing_edge.xz()) <= 18.0,
            "surf-to-bhop handoff should not leave a large dead zone"
        );
    }

    #[test]
    fn bhop_profile_selection_is_deterministic_for_seed_and_ordinal() {
        let (profile_a, pads_a, _) = build_test_bhop_section(12, 0xD15C_A11E);
        let (profile_b, pads_b, _) = build_test_bhop_section(12, 0xD15C_A11E);

        assert_eq!(profile_a, profile_b);
        assert_eq!(pads_a, pads_b);
    }

    #[test]
    fn bhop_patterns_emit_complete_routes_boundary_catches_and_anchor_clearance() {
        for (pattern, band) in [
            (BhopPattern::Rhythm, BhopDifficultyBand::Intro),
            (BhopPattern::LaneSwitch, BhopDifficultyBand::Mid),
            (BhopPattern::DropStrafe, BhopDifficultyBand::Mid),
            (BhopPattern::SplitMerge, BhopDifficultyBand::Advanced),
        ] {
            let profile = forced_bhop_profile(pattern, band);
            let (profile, pads, rooms) = build_forced_bhop_section(profile, 0xABCD_EF01);

            assert_has_complete_route(&profile, &pads);
            assert_boundary_catches(&profile, &pads);
            assert_anchor_clearance(&profile, &pads, &rooms);
            assert!(pads.iter().all(|pad| pad.side > 0.0 && pad.height > 0.0));
        }
    }

    #[test]
    fn bhop_difficulty_ramp_reduces_average_pad_size_and_catch_frequency() {
        let mut intro_side = 0.0;
        let mut advanced_side = 0.0;
        let mut intro_catch_rate = 0.0;
        let mut advanced_catch_rate = 0.0;

        for seed in 0_u64..24 {
            let (_, intro_pads, _) = build_test_bhop_section(0, seed ^ 0x1000);
            let (_, advanced_pads, _) = build_test_bhop_section(14, seed ^ 0x2000);
            intro_side += average_pad_side(&intro_pads);
            advanced_side += average_pad_side(&advanced_pads);
            intro_catch_rate += catch_pad_rate(&intro_pads);
            advanced_catch_rate += catch_pad_rate(&advanced_pads);
        }

        assert!(intro_side > advanced_side);
        assert!(intro_catch_rate > advanced_catch_rate);
    }

    #[test]
    fn split_merge_pattern_only_appears_in_advanced_bands() {
        let intro_mid_has_split_merge = [0_usize, 6_usize].into_iter().any(|segment_index| {
            (0_u64..128).any(|seed| {
                build_test_bhop_section(segment_index, seed).0.pattern == BhopPattern::SplitMerge
            })
        });
        let long_rooms = vec![
            test_room(0, Vec3::new(0.0, 120.0, 0.0)),
            test_room(1, Vec3::new(176.0, 80.0, 28.0)),
        ];
        let advanced_patterns = (0_u64..128)
            .map(|seed| {
                build_test_bhop_section_in_rooms(14, seed, &long_rooms)
                    .0
                    .pattern
            })
            .collect::<HashSet<_>>();

        assert!(!intro_mid_has_split_merge);
        assert!(advanced_patterns.contains(&BhopPattern::SplitMerge));
    }

    #[test]
    fn bhop_spacing_regression_fixtures_remain_valid() {
        let fixtures = [
            (0_usize, 0xBEEF_CAFE_u64),
            (12_usize, 0xD15C_A11E_u64),
            (14_usize, 0xABCD_EF01_u64),
        ];

        for (segment_index, seed) in fixtures {
            let (profile, pads, _) = build_test_bhop_section(segment_index, seed);
            let issues = spacing_issues(&profile, &pads);
            assert!(
                issues.is_empty(),
                "segment {segment_index} seed {seed:#x} had spacing issues: {issues:?}"
            );
        }
    }

    #[test]
    fn rhythm_spacing_preserves_variation_without_breaking_min_gaps() {
        let profile = forced_bhop_profile(BhopPattern::Rhythm, BhopDifficultyBand::Intro);
        let long_rooms = vec![
            test_room(0, Vec3::new(0.0, 120.0, 0.0)),
            test_room(1, Vec3::new(182.0, 82.0, 34.0)),
        ];
        let (profile, pads) = build_forced_bhop_section_in_rooms(profile, 0x1A2B_3C4D, &long_rooms);
        let gaps = route_gaps_for_mask(&profile, &pads, 0b01);
        let min_gap = gaps.iter().copied().fold(f32::INFINITY, f32::min);
        let max_gap = gaps.iter().copied().fold(f32::NEG_INFINITY, f32::max);

        assert!(spacing_issues(&profile, &pads).is_empty());
        assert!(min_gap >= BHOP_MIN_ROUTE_EDGE_GAP);
        assert!(max_gap - min_gap > 0.25, "rhythm gaps should still vary");
    }

    #[test]
    fn bhop_spacing_is_hard_guaranteed_across_seed_sweep() {
        let mut min_observed_route_gap = f32::INFINITY;
        let mut min_observed_branch_gap = f32::INFINITY;

        for segment_index in 0_usize..=20 {
            for seed in 0_u64..=2047 {
                let (profile, pads, _) = build_test_bhop_section(segment_index, seed);
                let issues = spacing_issues(&profile, &pads);
                assert!(
                    issues.is_empty(),
                    "segment {segment_index} seed {seed:#x} had spacing issues: {issues:?}"
                );
                min_observed_route_gap = min_observed_route_gap.min(min_route_gap(&profile, &pads));
                if let Some(branch_gap) = min_branch_gap(&profile, &pads) {
                    min_observed_branch_gap = min_observed_branch_gap.min(branch_gap);
                }
            }
        }

        assert!(min_observed_route_gap + BHOP_SPACING_EPSILON >= BHOP_MIN_ROUTE_EDGE_GAP);
        assert!(min_observed_branch_gap + BHOP_SPACING_EPSILON >= BHOP_MIN_BRANCH_EDGE_GAP);
    }

    #[test]
    fn surf_layout_emits_only_wedges_and_collider_strips() {
        let rooms = vec![
            test_room(0, Vec3::new(0.0, 120.0, 0.0)),
            test_room(1, Vec3::new(110.0, 94.0, 24.0)),
        ];
        let layout =
            build_segment_layout(&test_segment(0, SegmentKind::SurfRamp, 0x1234_5678), &rooms);

        assert!(!layout.solids.is_empty());
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
        let original_tail_y = blueprint.rooms.last().unwrap().top.y;

        append_run_blueprint(&mut blueprint, 6);

        assert_eq!(blueprint.rooms.len(), original_room_count + 6);
        assert_eq!(blueprint.segments.len(), original_segment_count + 6);
        assert_eq!(blueprint.segments.len(), blueprint.rooms.len() - 1);
        assert!(blueprint.rooms.last().unwrap().top.y < original_tail_y);
        assert_eq!(
            blueprint.segments[original_segment_count].from,
            original_room_count - 1
        );
    }

    #[test]
    fn surf_material_uses_forward_glossy_profile() {
        let mut cache = WorldAssetCache::default();
        let mut materials = Assets::<WorldSurfaceMaterial>::default();
        let handle = cached_material(&mut cache, &mut materials, MaterialKey::SurfRamp);
        let material = materials
            .get(&handle)
            .expect("cached surf material should exist");

        assert!(matches!(
            material.base.opaque_render_method,
            OpaqueRendererMethod::Forward
        ));
        assert!(matches!(material.base.alpha_mode, AlphaMode::Opaque));
        assert!(material.base.clearcoat >= 0.8);
        assert!(material.base.cull_mode.is_none());
        assert!(material.extension.settings.params_a.x > 0.5);
        assert!(material.extension.settings.params_c.x < material.extension.settings.params_c.y);
        assert!(material.extension.settings.atmosphere.w > 0.0);
    }

    #[test]
    fn bhop_material_uses_shader_detail_profile() {
        let mut cache = WorldAssetCache::default();
        let mut materials = Assets::<WorldSurfaceMaterial>::default();
        let handle = cached_material(&mut cache, &mut materials, MaterialKey::BhopPlatform);
        let material = materials
            .get(&handle)
            .expect("cached bhop material should exist");

        assert!(matches!(
            material.base.opaque_render_method,
            OpaqueRendererMethod::Forward
        ));
        assert!(material.base.clearcoat <= 0.12);
        assert!(material.extension.settings.params_a.x < 0.5);
        assert!(material.extension.settings.params_a.y > 0.1);
        assert!(material.extension.settings.params_b.z <= 5.0);
        assert!(material.extension.settings.params_c.x < material.extension.settings.params_c.y);
    }

    #[test]
    fn nebula_sky_material_uses_forward_unshadowed_pipeline() {
        let material = nebula_sky_material();

        assert!(matches!(
            material.opaque_render_method(),
            OpaqueRendererMethod::Forward
        ));
        assert!(!<NebulaSkyMaterial as Material>::enable_prepass());
        assert!(!<NebulaSkyMaterial as Material>::enable_shadows());
        assert!(material.settings.params_a.z > 100.0);
        assert!(material.settings.halo.w > 0.0);
    }

    #[test]
    fn visual_motion_factor_is_bounded() {
        assert_eq!(visual_motion_factor(0.0), 0.0);
        assert_eq!(visual_motion_factor(120.0), 0.0);
        assert!(visual_motion_factor(450.0) > 0.0);
        assert_eq!(visual_motion_factor(10_000.0), 1.0);
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
        assert_eq!(builder.uvs.len(), builder.positions.len());
        assert!(
            builder
                .uvs
                .iter()
                .all(|uv| uv[0].is_finite() && uv[1].is_finite())
        );
    }

    #[test]
    fn surf_wedge_triangles_face_outward_for_mirrored_walls() {
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

        for render_points in [
            points.clone(),
            points
                .iter()
                .map(|point| Vec3::new(-point.x, point.y, point.z))
                .collect::<Vec<_>>(),
        ] {
            let mut builder = ColoredMeshBuilder::default();
            append_surf_wedge_render_geometry(
                &mut builder,
                Vec3::ZERO,
                &render_points,
                Color::srgb(0.2, 0.3, 0.5),
                Color::srgb(0.9, 0.9, 1.0),
            );

            for triangle in builder
                .positions
                .chunks_exact(3)
                .zip(builder.normals.chunks_exact(3))
            {
                let (positions, normals) = triangle;
                let center = positions
                    .iter()
                    .map(|position| Vec3::from_array(*position))
                    .sum::<Vec3>()
                    / 3.0;
                let normal = Vec3::from_array(normals[0]);
                assert!(
                    normal.dot(center) >= -0.001,
                    "triangle normal should face away from wedge center"
                );
            }
        }
    }

    #[test]
    fn generated_render_meshes_include_shader_uv_inputs() {
        let mut cache = WorldAssetCache::default();
        let mut meshes = Assets::<Mesh>::default();
        let handle = cached_cuboid_mesh(&mut cache, &mut meshes, Vec3::new(4.0, 1.0, 4.0));
        let mesh = meshes
            .get(&handle)
            .expect("cached cuboid mesh should exist");

        assert!(mesh.attribute(Mesh::ATTRIBUTE_UV_0).is_some());
    }

    #[test]
    fn surf_collider_strip_extends_past_render_seams() {
        let rooms = vec![
            test_room(0, Vec3::new(0.0, 40.0, 0.0)),
            test_room(1, Vec3::new(120.0, 18.0, 26.0)),
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
