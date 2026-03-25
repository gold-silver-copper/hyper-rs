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

use crate::util::{ExampleUtilPlugin, StableGround};

mod util;

const ROOM_HEIGHT: f32 = 1.2;
const CELL_SIZE: f32 = 15.0;
const PLAYER_SPAWN_CLEARANCE: f32 = 2.5;
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
        .add_systems(Startup, (setup_scene, setup_hud).chain())
        .add_systems(PostStartup, tune_player_camera)
        .add_systems(
            Update,
            (
                capture_cursor.run_if(input_just_pressed(MouseButton::Left)),
                release_cursor.run_if(input_just_pressed(KeyCode::Escape)),
                update_hud,
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
    let tower = build_tower(&mut commands, &mut meshes, &mut materials);
    commands.insert_resource(tower);

    commands.spawn((
        Name::new("Spawn Point"),
        SpawnPlayer,
        Transform::from_translation(tower.spawn),
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
            Transform::from_translation(tower.spawn),
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

    commands.spawn((
        Name::new("Sun"),
        Transform::from_xyz(-40.0, tower.summit.y + 36.0, 25.0)
            .looking_at(Vec3::new(0.0, tower.summit.y * 0.45, 0.0), Vec3::Y),
        DirectionalLight {
            shadows_enabled: true,
            illuminance: 28_000.0,
            ..default()
        },
    ));
}

fn setup_hud(mut commands: Commands, tower: Res<TowerMeta>) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: px(12.0),
            left: px(12.0),
            max_width: px(420.0),
            padding: UiRect::all(px(12.0)),
            ..default()
        },
        Text::new(format!(
            "Chronoclimb\nSeed: {:016x}\nGenerating {} floors...",
            tower.seed, tower.floors
        )),
        BackgroundColor(Color::BLACK.with_alpha(0.42)),
        RunHud,
    ));
}

fn update_hud(
    tower: Res<TowerMeta>,
    players: Query<&Transform, With<Player>>,
    mut hud: Single<&mut Text, With<RunHud>>,
) {
    let Ok(player) = players.single() else {
        return;
    };

    let current_height = player.translation.y.max(0.0);
    let summit_height = tower.summit.y.max(1.0);
    let progress = (current_height / summit_height).clamp(0.0, 1.0);
    let horizontal_delta = (player.translation - tower.summit).xz().length();
    let reached_summit = current_height >= tower.summit.y - 1.2 && horizontal_delta < 7.5;
    let objective = if reached_summit {
        "Summit reached. Restart to get a fresh tower."
    } else {
        "Goal: reach the beacon at the top."
    };

    hud.0 = format!(
        "Chronoclimb\n\
         Seed: {seed:016x}\n\
         Floors: {floors}\n\
         Summit: {summit_height:.1}m\n\
         Height: {current_height:.1}m ({progress:.0}%)\n\
         {objective}\n\
         Controls: WASD move | hold Space bhop/jump | Ctrl crouch/climbdown\n\
         RMB pull/drop props | LMB throw | reach the brightest platform",
        seed = tower.seed,
        floors = tower.floors,
        summit_height = summit_height,
        current_height = current_height,
        progress = progress * 100.0,
        objective = objective,
    );
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

#[derive(Resource, Clone, Copy)]
struct TowerMeta {
    seed: u64,
    floors: usize,
    spawn: Vec3,
    summit: Vec3,
}

#[derive(Clone, Copy)]
struct TowerNode {
    top: Vec3,
    footprint: Vec2,
    theme: Theme,
    motif: TraversalMotif,
    approach: IVec2,
    seed: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TraversalMotif {
    Stair,
    Mantle,
    WallRun,
    MovingPlatform,
    CrateVault,
    IceBridge,
    WaterGarden,
}

#[derive(Clone, Copy)]
enum Theme {
    Stone,
    Overgrown,
    Frost,
    Ember,
}

#[derive(Component)]
struct RunHud;

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

fn build_tower(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> TowerMeta {
    let seed = current_run_seed();
    let mut rng = RunRng::new(seed);
    let theme_offset = (rng.next_u64() % 4) as usize;
    let nodes = generate_nodes(&mut rng, theme_offset);

    spawn_static_box(
        commands,
        meshes,
        materials,
        "Abyss Floor",
        Vec3::new(200.0, 2.0, 200.0),
        Vec3::new(0.0, -2.0, 0.0),
        tailwind::SLATE_950.into(),
    );

    for (index, node) in nodes.iter().enumerate() {
        spawn_room(commands, meshes, materials, index, *node, nodes.len() - 1);

        if index > 0 {
            spawn_connection(commands, meshes, materials, nodes[index - 1], *node);
        }

        let exit_hint = nodes
            .get(index + 1)
            .map(|next| direction_from_delta(next.top - node.top))
            .unwrap_or_else(|| cardinal_to_vec3(node.approach));
        spawn_side_branch(
            commands,
            meshes,
            materials,
            *node,
            exit_hint,
            index,
            nodes.len(),
        );
    }

    let summit = *nodes.last().unwrap();
    spawn_summit(commands, meshes, materials, summit);

    TowerMeta {
        seed,
        floors: nodes.len(),
        spawn: nodes[0].top + Vec3::new(0.0, PLAYER_SPAWN_CLEARANCE, nodes[0].footprint.y * 0.18),
        summit: summit.top + Vec3::new(0.0, 1.0, 0.0),
    }
}

fn generate_nodes(rng: &mut RunRng, theme_offset: usize) -> Vec<TowerNode> {
    let floors = rng.range_usize(9, 13);
    let mut nodes = Vec::with_capacity(floors);
    let mut visited = HashSet::new();
    let mut cell = IVec2::ZERO;
    let mut heading = IVec2::NEG_Y;
    let mut height = 1.4;
    let mut previous_motif = TraversalMotif::Stair;

    visited.insert(cell);
    nodes.push(TowerNode {
        top: Vec3::new(0.0, height, 0.0),
        footprint: Vec2::splat(12.5),
        theme: theme_for(0, floors, theme_offset),
        motif: TraversalMotif::Stair,
        approach: IVec2::ZERO,
        seed: rng.next_u64(),
    });

    for index in 1..floors {
        let dir = choose_next_dir(rng, cell, heading, &visited);
        heading = dir;
        cell += dir;
        visited.insert(cell);

        height += rng.range_f32(4.4, 6.4) + index as f32 * 0.18;
        let jitter = Vec3::new(rng.range_f32(-1.2, 1.2), 0.0, rng.range_f32(-1.2, 1.2));
        let top = Vec3::new(cell.x as f32 * CELL_SIZE, height, cell.y as f32 * CELL_SIZE) + jitter;
        let footprint = if index == floors - 1 {
            Vec2::splat(14.0)
        } else {
            Vec2::new(rng.range_f32(8.4, 11.8), rng.range_f32(8.4, 11.8))
        };
        let motif = choose_motif(rng, previous_motif, index, floors);
        previous_motif = motif;

        nodes.push(TowerNode {
            top,
            footprint,
            theme: theme_for(index, floors, theme_offset),
            motif,
            approach: dir,
            seed: rng.next_u64(),
        });
    }

    nodes
}

fn choose_next_dir(
    rng: &mut RunRng,
    cell: IVec2,
    heading: IVec2,
    visited: &HashSet<IVec2>,
) -> IVec2 {
    let dirs = [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y];
    let mut weighted = Vec::with_capacity(dirs.len());

    for dir in dirs {
        if dir == -heading {
            continue;
        }

        let next = cell + dir;
        let radius = next.x.abs().max(next.y.abs());
        let mut weight: u32 = 1;
        if dir == heading {
            weight += 4;
        }
        if !visited.contains(&next) {
            weight += 5;
        }
        if radius <= 4 {
            weight += 2;
        }
        if radius >= 6 {
            weight = weight.saturating_sub(1);
        }
        weighted.push((dir, weight as u32));
    }

    if weighted.is_empty() {
        return heading;
    }

    rng.weighted_choice(&weighted)
}

fn choose_motif(
    rng: &mut RunRng,
    previous: TraversalMotif,
    index: usize,
    total: usize,
) -> TraversalMotif {
    let progress = index as f32 / total as f32;
    let mut weighted = vec![
        (TraversalMotif::Stair, if progress < 0.3 { 7 } else { 3 }),
        (TraversalMotif::Mantle, 5),
        (TraversalMotif::WallRun, if progress > 0.2 { 4 } else { 1 }),
        (
            TraversalMotif::MovingPlatform,
            if progress > 0.25 { 4 } else { 1 },
        ),
        (TraversalMotif::CrateVault, 4),
        (
            TraversalMotif::IceBridge,
            if progress > 0.45 { 4 } else { 2 },
        ),
        (
            TraversalMotif::WaterGarden,
            if progress < 0.65 { 3 } else { 1 },
        ),
    ];

    for (_, weight) in &mut weighted {
        if *weight > 1 {
            *weight -= 1;
        }
    }

    for (motif, weight) in &mut weighted {
        if *motif == previous {
            *weight = 1;
        }
    }

    rng.weighted_choice(&weighted)
}

fn theme_for(index: usize, total: usize, offset: usize) -> Theme {
    let bands = [Theme::Stone, Theme::Overgrown, Theme::Frost, Theme::Ember];
    let band = (index * bands.len() / total + offset) % bands.len();
    bands[band]
}

fn spawn_room(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    index: usize,
    node: TowerNode,
    summit_index: usize,
) {
    let room_color = if index == summit_index {
        tailwind::YELLOW_200.into()
    } else {
        theme_floor_color(node.theme)
    };

    spawn_static_box_top(
        commands,
        meshes,
        materials,
        &format!("Room {}", index),
        Vec3::new(node.footprint.x, ROOM_HEIGHT, node.footprint.y),
        node.top,
        room_color,
    );

    if node.top.y > 4.0 {
        let support_height = node.top.y + 2.0;
        spawn_static_box(
            commands,
            meshes,
            materials,
            &format!("Support {}", index),
            Vec3::new(2.4, support_height, 2.4),
            Vec3::new(node.top.x, node.top.y - support_height * 0.5, node.top.z),
            theme_shadow_color(node.theme),
        );
    }

    let mut local_rng = RunRng::new(node.seed ^ 0x5DEECE66D);
    if index > 0 && index < summit_index && local_rng.chance(0.45) {
        let prop_count = if local_rng.chance(0.4) { 2 } else { 1 };
        for prop_index in 0..prop_count {
            let offset = Vec3::new(
                local_rng.range_f32(-node.footprint.x * 0.2, node.footprint.x * 0.2),
                0.0,
                local_rng.range_f32(-node.footprint.y * 0.2, node.footprint.y * 0.2),
            );
            spawn_dynamic_box_top(
                commands,
                meshes,
                materials,
                &format!("Room {} Cache {}", index, prop_index),
                Vec3::splat(1.3),
                node.top + offset + Vec3::Y * 0.02,
                theme_prop_color(node.theme),
            );
        }
    }
}

fn spawn_connection(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    from: TowerNode,
    to: TowerNode,
) {
    let mut rng = RunRng::new(to.seed ^ 0x9E3779B97F4A7C15);
    let forward = direction_from_delta(to.top - from.top);
    let along_x = forward.x.abs() > 0.5;
    let right = Vec3::new(-forward.z, 0.0, forward.x);
    let start = room_edge(from, forward);
    let end = room_edge(to, -forward);

    match to.motif {
        TraversalMotif::Stair => {
            let steps = rng.range_usize(4, 7);
            for step in 0..steps {
                let t = (step + 1) as f32 / (steps + 1) as f32;
                let mut top = start.lerp(end, t);
                top.y = from.top.y + (to.top.y - from.top.y) * t - 0.15;
                top += right * rng.range_f32(-0.6, 0.6);

                spawn_static_box_top(
                    commands,
                    meshes,
                    materials,
                    "Stair Step",
                    axis_box_size(along_x, 3.0, 0.8 + t * 0.25, 3.4),
                    top,
                    theme_accent_color(to.theme),
                );
            }
        }
        TraversalMotif::Mantle => {
            let ledges = [
                (0.28, from.top.y + 1.4),
                (0.56, from.top.y + (to.top.y - from.top.y) * 0.58),
                (0.8, to.top.y - 0.4),
            ];
            for (index, (t, top_y)) in ledges.into_iter().enumerate() {
                let mut top = start.lerp(end, t);
                top.y = top_y.min(to.top.y - 0.2);
                top += right * (index as f32 - 1.0) * 0.9;
                spawn_static_box_top(
                    commands,
                    meshes,
                    materials,
                    "Mantle Ledge",
                    axis_box_size(along_x, 3.2, 1.2 + index as f32 * 0.5, 3.6),
                    top,
                    theme_accent_color(to.theme),
                );
            }

            let wall_mid = start.lerp(end, 0.68) + right * 2.2;
            let wall_height = (to.top.y - from.top.y).abs() + 4.5;
            spawn_static_box(
                commands,
                meshes,
                materials,
                "Mantle Wall",
                axis_box_size(along_x, 5.6, wall_height, 1.0),
                Vec3::new(wall_mid.x, from.top.y + wall_height * 0.5 - 0.6, wall_mid.z),
                theme_shadow_color(to.theme),
            );
        }
        TraversalMotif::WallRun => {
            let wall_side = if rng.chance(0.5) { 1.0 } else { -1.0 };
            let length = (end - start).xz().length() + 5.0;
            let wall_height = (to.top.y - from.top.y).abs() + 7.5;
            let wall_mid = start.lerp(end, 0.5) + right * wall_side * 3.8;
            spawn_static_box(
                commands,
                meshes,
                materials,
                "Wall Run Wall",
                axis_box_size(along_x, length, wall_height, 1.15),
                Vec3::new(wall_mid.x, from.top.y + wall_height * 0.5 - 1.0, wall_mid.z),
                theme_shadow_color(to.theme),
            );

            spawn_static_box_top(
                commands,
                meshes,
                materials,
                "Kick Pad",
                axis_box_size(along_x, 2.6, 0.8, 2.6),
                start + right * wall_side * 1.15 + Vec3::Y * 0.55,
                theme_accent_color(to.theme),
            );

            spawn_static_box_top(
                commands,
                meshes,
                materials,
                "Wall Run Rest",
                axis_box_size(along_x, 3.0, 0.9, 3.0),
                start.lerp(end, 0.56) - right * wall_side * 2.4 + Vec3::Y * 1.0,
                theme_floor_color(to.theme),
            );
        }
        TraversalMotif::MovingPlatform => {
            let mover_size = Vec3::splat(3.3).with_y(0.65);
            let mut mover_top_start = start.lerp(end, 0.28);
            mover_top_start.y = from.top.y + (to.top.y - from.top.y) * 0.35 + 1.0;
            let mut mover_top_end = start.lerp(end, 0.74);
            mover_top_end.y = from.top.y + (to.top.y - from.top.y) * 0.72 + 1.2;

            spawn_mover(
                commands,
                meshes,
                materials,
                "Sky Lift",
                mover_size,
                top_to_center(mover_top_start, mover_size.y),
                top_to_center(mover_top_end, mover_size.y),
                3.4,
                theme_accent_color(to.theme),
            );

            for anchor in [0.28, 0.74] {
                let top = start.lerp(end, anchor) - right * 3.4;
                let support_height = top.y + 1.5;
                spawn_static_box(
                    commands,
                    meshes,
                    materials,
                    "Lift Support",
                    Vec3::new(1.6, support_height, 1.6),
                    Vec3::new(top.x, top.y - support_height * 0.5, top.z),
                    theme_shadow_color(to.theme),
                );
            }
        }
        TraversalMotif::CrateVault => {
            for perch in [0.3, 0.58, 0.82] {
                let mut top = start.lerp(end, perch);
                top.y = from.top.y + (to.top.y - from.top.y) * perch - 0.2;
                spawn_static_box_top(
                    commands,
                    meshes,
                    materials,
                    "Vault Perch",
                    axis_box_size(along_x, 3.8, 0.9, 3.8),
                    top,
                    theme_floor_color(to.theme),
                );

                let box_offset = right * rng.range_f32(-0.8, 0.8) + Vec3::Y * 0.08;
                spawn_dynamic_box_top(
                    commands,
                    meshes,
                    materials,
                    "Vault Crate",
                    Vec3::splat(1.35),
                    top + box_offset,
                    theme_prop_color(to.theme),
                );
            }
        }
        TraversalMotif::IceBridge => {
            for span in [0.22, 0.5, 0.78] {
                let mut top = start.lerp(end, span);
                top.y = from.top.y + (to.top.y - from.top.y) * span - 0.18;
                let mut segment = spawn_static_box_top(
                    commands,
                    meshes,
                    materials,
                    "Ice Span",
                    axis_box_size(along_x, 3.6, 0.65, 2.1),
                    top,
                    tailwind::SKY_300.into(),
                );
                segment.insert(Friction::new(0.02));
            }

            spawn_static_box_top(
                commands,
                meshes,
                materials,
                "Ice Rest",
                Vec3::new(3.2, 0.7, 3.2),
                start.lerp(end, 0.5) + Vec3::Y * 0.5,
                tailwind::SLATE_200.into(),
            );
        }
        TraversalMotif::WaterGarden => {
            let basin_top = from.top.y.min(to.top.y) - 2.4;
            let basin_mid = start.lerp(end, 0.5);
            let basin_size = axis_box_size(along_x, (end - start).xz().length() + 8.0, 0.8, 10.0);

            spawn_static_box_top(
                commands,
                meshes,
                materials,
                "Water Basin Floor",
                basin_size,
                Vec3::new(basin_mid.x, basin_top, basin_mid.z),
                tailwind::SLATE_700.into(),
            );

            spawn_water_box(
                commands,
                meshes,
                materials,
                axis_box_size(
                    along_x,
                    basin_size.x.max(basin_size.z),
                    2.8,
                    basin_size.z.min(basin_size.x),
                ),
                Vec3::new(basin_mid.x, basin_top - 1.4, basin_mid.z),
            );

            for pillar in [0.2, 0.5, 0.8] {
                let mut top = start.lerp(end, pillar);
                top.y = from.top.y + (to.top.y - from.top.y) * pillar - 0.8;
                let pillar_height = (top.y - basin_top + 0.8).max(1.5);
                spawn_static_box_top(
                    commands,
                    meshes,
                    materials,
                    "Water Pillar",
                    Vec3::new(2.5, pillar_height, 2.5),
                    top,
                    theme_floor_color(to.theme),
                );
            }
        }
    }
}

fn spawn_side_branch(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    node: TowerNode,
    main_forward: Vec3,
    index: usize,
    total: usize,
) {
    if index == 0 || index + 1 >= total {
        return;
    }

    let mut rng = RunRng::new(node.seed ^ 0xC6BC279692B5C323);
    if !rng.chance(0.42) {
        return;
    }

    let mut branch_dir = Vec3::new(-main_forward.z, 0.0, main_forward.x);
    if branch_dir == Vec3::ZERO {
        branch_dir = Vec3::X;
    }
    if rng.chance(0.5) {
        branch_dir = -branch_dir;
    }

    let branch_distance = node.footprint.max_element() * 0.5 + rng.range_f32(6.0, 8.5);
    let bridge_top = node.top + branch_dir * (branch_distance * 0.5);
    let branch_top = node.top + branch_dir * branch_distance + Vec3::Y * rng.range_f32(-0.3, 1.8);
    let branch_size = Vec3::new(rng.range_f32(4.8, 6.8), 0.9, rng.range_f32(4.8, 6.8));

    let mut bridge = spawn_static_box_top(
        commands,
        meshes,
        materials,
        "Side Bridge",
        if branch_dir.x.abs() > 0.5 {
            Vec3::new(branch_distance - 1.6, 0.6, 2.2)
        } else {
            Vec3::new(2.2, 0.6, branch_distance - 1.6)
        },
        Vec3::new(bridge_top.x, node.top.y + 0.2, bridge_top.z),
        tailwind::STONE_500.into(),
    );

    if rng.chance(0.35) {
        bridge.insert(Friction::new(0.05));
    }

    spawn_static_box_top(
        commands,
        meshes,
        materials,
        "Side Platform",
        branch_size,
        branch_top,
        theme_floor_color(node.theme),
    );

    for reward in 0..=1 {
        if reward == 1 && !rng.chance(0.5) {
            continue;
        }
        let offset = Vec3::new(
            rng.range_f32(-branch_size.x * 0.18, branch_size.x * 0.18),
            0.0,
            rng.range_f32(-branch_size.z * 0.18, branch_size.z * 0.18),
        );
        spawn_dynamic_box_top(
            commands,
            meshes,
            materials,
            "Branch Cache",
            Vec3::splat(1.25),
            branch_top + offset + Vec3::Y * 0.06,
            theme_prop_color(node.theme),
        );
    }
}

fn spawn_summit(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    node: TowerNode,
) {
    spawn_static_box_top(
        commands,
        meshes,
        materials,
        "Summit Dais",
        Vec3::new(node.footprint.x + 3.0, ROOM_HEIGHT, node.footprint.y + 3.0),
        node.top + Vec3::Y * 0.75,
        tailwind::YELLOW_100.into(),
    );

    spawn_static_box(
        commands,
        meshes,
        materials,
        "Summit Beacon Column",
        Vec3::new(1.8, 8.0, 1.8),
        Vec3::new(node.top.x, node.top.y + 4.0, node.top.z),
        tailwind::AMBER_400.into(),
    );
    spawn_static_box(
        commands,
        meshes,
        materials,
        "Summit Beacon",
        Vec3::new(4.8, 1.4, 4.8),
        Vec3::new(node.top.x, node.top.y + 8.6, node.top.z),
        tailwind::YELLOW_300.into(),
    );
    commands.spawn((
        Name::new("Summit Light"),
        PointLight {
            intensity: 500_000.0,
            range: 120.0,
            color: tailwind::AMBER_200.into(),
            shadows_enabled: true,
            ..default()
        },
        Transform::from_translation(node.top + Vec3::Y * 10.0),
    ));
}

fn room_edge(node: TowerNode, forward: Vec3) -> Vec3 {
    let extent = if forward.x.abs() > 0.5 {
        node.footprint.x
    } else {
        node.footprint.y
    };
    node.top + forward * (extent * 0.5 - 1.1)
}

fn direction_from_delta(delta: Vec3) -> Vec3 {
    if delta.x.abs() >= delta.z.abs() {
        Vec3::new(delta.x.signum(), 0.0, 0.0)
    } else {
        Vec3::new(0.0, 0.0, delta.z.signum())
    }
}

fn cardinal_to_vec3(dir: IVec2) -> Vec3 {
    Vec3::new(dir.x as f32, 0.0, dir.y as f32).normalize_or_zero()
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

fn spawn_static_box_top<'a>(
    commands: &'a mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    name: &str,
    size: Vec3,
    top: Vec3,
    color: Color,
) -> EntityCommands<'a> {
    spawn_static_box(
        commands,
        meshes,
        materials,
        name,
        size,
        top_to_center(top, size.y),
        color,
    )
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

fn spawn_dynamic_box_top(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    name: &str,
    size: Vec3,
    top: Vec3,
    color: Color,
) {
    spawn_dynamic_box(
        commands,
        meshes,
        materials,
        name,
        size,
        top_to_center(top, size.y),
        color,
    );
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
            base_color: Color::srgba(0.15, 0.45, 0.95, 0.42),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.15,
            reflectance: 0.5,
            ..default()
        })),
        Transform::from_translation(translation),
        RigidBody::Static,
        Collider::cuboid(size.x, size.y, size.z),
        CollisionLayers::new(CollisionLayer::Default, LayerMask::ALL),
        Water { speed: 0.72 },
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
