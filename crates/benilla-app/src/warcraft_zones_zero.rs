//! A small offline third-person party-combat slice layered over the faithful 1.12.1 client.
//! Stock world assets, creature attachment and animation remain owned by benilla.

use benilla_protocol::{messages::ObjectFields, EntityKind};
use benilla_world::{
    collision::WorldCollision, schedule::WorldStage, view::WorldCamera, world_point::WorldPoint,
};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy::{input::mouse::AccumulatedMouseMotion, math::Dir3};

use crate::{
    creature_anim::{move_flags, MovementState},
    net::{ActiveMover, Guid, NetEntity, ObjectStore},
    player::{Player, PlayerCapsule, CAPSULE_HEIGHT},
    run_mode::CaptureMode,
};

const SCARLET_MONASTERY: u32 = 189;
// Scarlet Monastery Cathedral, map 189 (WoW coordinates, converted below to Bevy axes).
const START_WOW: [f32; 3] = [855.11, 1320.76, 18.67];

pub(crate) fn active() -> bool {
    std::env::var_os("WOW_WARCRAFT_ZONES_ZERO").is_some()
}

pub(crate) struct WarcraftZonesZeroPlugin;

impl Plugin for WarcraftZonesZeroPlugin {
    fn build(&self, app: &mut App) {
        // Disable stock glue/UI/controller input; our game layer owns the in-world session.
        app.insert_resource(CaptureMode)
            .init_resource::<ActionCamPointer>()
            .init_resource::<Party>()
            .add_systems(
                OnEnter(crate::char_select::ClientState::InWorld),
                (spawn_run, capture_pointer).chain(),
            )
            .add_systems(
                Update,
                (
                    update_pointer_mode,
                    control_party,
                    combat,
                    reap_defeated,
                    follow_camera,
                )
                    .chain()
                    .in_set(WorldStage::Input),
            );
        info!("Warcraft Zones Zero: WASD move, mouse look, Space jump, LMB light, RMB heavy, Q/E/R specials, 1-3 swap");
    }
}

#[derive(Component)]
struct PartyMember(usize);

#[derive(Component)]
struct Enemy;

/// The prototype has no world-pointer interactions; the modifier only releases the OS pointer.
#[derive(Resource)]
pub(crate) struct ActionCamPointer {
    pub(crate) captured: bool,
}

impl Default for ActionCamPointer {
    fn default() -> Self {
        Self { captured: true }
    }
}

#[derive(Component)]
struct Health(f32);

#[derive(Resource)]
struct Party {
    active: usize,
    facing: f32,
    camera_yaw: f32,
    camera_pitch: f32,
    light_cooldown: f32,
    heavy_cooldown: f32,
    combo: u8,
    combo_timer: f32,
    special_cooldowns: [f32; 3],
    jump_velocity: f32,
    grounded: bool,
}

impl Default for Party {
    fn default() -> Self {
        Self {
            active: 0,
            facing: 0.0,
            camera_yaw: 0.0,
            camera_pitch: 0.42,
            light_cooldown: 0.0,
            heavy_cooldown: 0.0,
            combo: 0,
            combo_timer: 0.0,
            special_cooldowns: [0.0; 3],
            jump_velocity: 0.0,
            grounded: true,
        }
    }
}

fn wow_to_bevy(p: [f32; 3]) -> Vec3 {
    Vec3::new(-p[1], p[2], -p[0])
}

fn spawn_unit(
    commands: &mut Commands,
    serial: u64,
    display: u32,
    pos: Vec3,
    faction: u32,
) -> Entity {
    const HIGH_CREATURE: u64 = 0xF130 << 48;
    let guid = HIGH_CREATURE | serial;
    let fields = ObjectFields::from_pairs(&[
        (0, guid as u32),
        (1, (guid >> 32) as u32),
        (2, 0x9),
        (4, 1.0f32.to_bits()),
        (22, 100),
        (28, 100),
        (34, 30),
        (35, faction),
        // Human male, warrior, physical power; scarlet enemies use the same renderable human rig.
        (36, 1 | (1 << 8)),
    ]);
    commands
        .spawn((
            Guid(guid),
            NetEntity {
                kind: EntityKind::Unit,
                display_id: Some(display),
                scale: 1.0,
            },
            ObjectStore(fields),
            MovementState::default(),
            Transform::from_translation(pos),
            Visibility::default(),
        ))
        .id()
}

fn spawn_run(mut commands: Commands, map: Option<ResMut<benilla_world::world_map::CurrentMap>>) {
    if let Some(mut map) = map {
        map.0 = SCARLET_MONASTERY;
    }
    let origin = wow_to_bevy(START_WOW);
    // Party looks use familiar 1.12 human display models; enemies use a Scarlet Crusader look.
    for (i, display) in [49, 50, 51].into_iter().enumerate() {
        let pos = origin + Vec3::new((i as f32 - 1.0) * 1.8, 0.0, 2.0 + (i as f32 * 0.4));
        let entity = spawn_unit(&mut commands, 0xD311_000 + i as u64, display, pos, 35);
        commands
            .entity(entity)
            .insert((PartyMember(i), Health(100.0), ActiveMover));
    }
    for i in 0..4 {
        let pos = origin + Vec3::new((i as f32 - 1.5) * 2.2, 0.0, -8.0 - (i as f32 % 2.0) * 2.5);
        let entity = spawn_unit(&mut commands, 0xD311_100 + i as u64, 1563, pos, 14);
        commands.entity(entity).insert((Enemy, Health(80.0)));
    }
    info!("Scarlet Monastery loaded. Mouse look, WASD move, Space jump, LMB/RMB attacks, Q/E/R specials.");
}

fn capture_pointer(
    mut pointers: Query<&mut CursorOptions, With<PrimaryWindow>>,
    mut action_cam: ResMut<ActionCamPointer>,
) {
    action_cam.captured = true;
    if let Ok(mut pointer) = pointers.single_mut() {
        pointer.grab_mode = CursorGrabMode::Locked;
        pointer.visible = false;
    }
}

/// Holding the platform's window modifier releases the cursor and pauses mouse-look.
#[cfg(target_os = "macos")]
fn pointer_release_held(keys: &ButtonInput<KeyCode>) -> bool {
    keys.any_pressed([KeyCode::SuperLeft, KeyCode::SuperRight])
}

#[cfg(target_os = "macos")]
fn pointer_release_just_released(keys: &ButtonInput<KeyCode>) -> bool {
    keys.just_released(KeyCode::SuperLeft) || keys.just_released(KeyCode::SuperRight)
}

/// Holding Alt releases the cursor and pauses mouse-look on Windows and Linux.
#[cfg(not(target_os = "macos"))]
fn pointer_release_held(keys: &ButtonInput<KeyCode>) -> bool {
    keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight])
}

#[cfg(not(target_os = "macos"))]
fn pointer_release_just_released(keys: &ButtonInput<KeyCode>) -> bool {
    keys.just_released(KeyCode::AltLeft) || keys.just_released(KeyCode::AltRight)
}

fn update_pointer_mode(
    keys: Res<ButtonInput<KeyCode>>,
    mut action_cam: ResMut<ActionCamPointer>,
    mut windows: Query<(&mut Window, &mut CursorOptions), With<PrimaryWindow>>,
) {
    let release = pointer_release_held(&keys);
    action_cam.captured = !release;
    if let Ok((mut window, mut pointer)) = windows.single_mut() {
        pointer.grab_mode = if release {
            CursorGrabMode::None
        } else {
            CursorGrabMode::Locked
        };
        pointer.visible = release;
        if !release && pointer_release_just_released(&keys) {
            let center = window.size() * 0.5;
            window.set_cursor_position(Some(center));
        }
    }
}

fn control_party(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse_motion: Res<AccumulatedMouseMotion>,
    world: WorldPoint,
    collision: WorldCollision,
    capsule: Res<PlayerCapsule>,
    mut party: ResMut<Party>,
    mut player: ResMut<Player>,
    mut members: Query<(&PartyMember, &mut Transform, &mut MovementState)>,
) {
    let dt = time.delta_secs();
    party.light_cooldown = (party.light_cooldown - dt).max(0.0);
    party.heavy_cooldown = (party.heavy_cooldown - dt).max(0.0);
    for cooldown in &mut party.special_cooldowns {
        *cooldown = (*cooldown - dt).max(0.0);
    }
    party.combo_timer = (party.combo_timer - dt).max(0.0);
    if party.combo_timer == 0.0 {
        party.combo = 0;
    }
    if !pointer_release_held(&keys) {
        const MOUSE_SENSITIVITY: f32 = 0.003;
        party.camera_yaw -= mouse_motion.delta.x * MOUSE_SENSITIVITY;
        party.camera_pitch =
            (party.camera_pitch + mouse_motion.delta.y * MOUSE_SENSITIVITY).clamp(-0.15, 1.2);
    }
    if keys.just_pressed(KeyCode::Digit1) {
        party.active = 0;
    }
    if keys.just_pressed(KeyCode::Digit2) {
        party.active = 1;
    }
    if keys.just_pressed(KeyCode::Digit3) {
        party.active = 2;
    }
    if keys.just_pressed(KeyCode::Tab) {
        party.active = (party.active + 1) % 3;
    }

    let forward = Vec3::new(-party.camera_yaw.sin(), 0.0, -party.camera_yaw.cos());
    let right = Vec3::new(forward.z, 0.0, -forward.x);
    let mut dir = Vec3::ZERO;
    if keys.pressed(KeyCode::KeyW) {
        dir += forward;
    }
    if keys.pressed(KeyCode::KeyS) {
        dir -= forward;
    }
    if keys.pressed(KeyCode::KeyD) {
        dir += right;
    }
    if keys.pressed(KeyCode::KeyA) {
        dir -= right;
    }
    let travel = if dir == Vec3::ZERO {
        Vec3::ZERO
    } else {
        dir.normalize() * 6.0 * dt
    };
    if keys.just_pressed(KeyCode::Space) && party.grounded {
        party.jump_velocity = 7.955_547;
        party.grounded = false;
    }
    if !party.grounded {
        party.jump_velocity -= 19.291_105 * dt;
    }
    let vertical_travel = if party.grounded {
        0.0
    } else {
        party.jump_velocity * dt
    };
    party.facing = if travel == Vec3::ZERO {
        party.camera_yaw
    } else {
        (-travel.x).atan2(-travel.z)
    };

    let mut active_pos = None;
    let mut landed = false;
    for (member, mut transform, mut movement) in &mut members {
        let selected = member.0 == party.active;
        let capsule_center = transform.translation + Vec3::Y * (CAPSULE_HEIGHT * 0.5);
        let mut safe_travel = collision
            .cast_mover(&capsule.0, capsule_center, travel, 0.02)
            .map_or(travel, |hit| {
                travel.normalize_or_zero() * hit.distance.min(travel.length())
            });
        let next_feet = transform.translation + safe_travel;
        if party.grounded
            && safe_travel != Vec3::ZERO
            && collision
                .ray_body(next_feet + Vec3::Y * 0.2, Dir3::NEG_Y, 0.7)
                .is_none()
        {
            safe_travel = Vec3::ZERO;
        }
        let previous_feet = transform.translation;
        transform.translation += safe_travel + Vec3::Y * vertical_travel;
        movement.speed = if safe_travel == Vec3::ZERO { 0.0 } else { 6.0 };
        movement.vertical_speed = party.jump_velocity;
        movement.flags = if safe_travel == Vec3::ZERO {
            0
        } else {
            move_flags::FORWARD
        };
        if !party.grounded {
            movement.flags |= move_flags::FALLING;
        }
        transform.rotation = Quat::from_rotation_y(party.facing);
        let floor_y = world
            .terrain_height_under(transform.translation)
            .or_else(|| {
                let origin = previous_feet + Vec3::Y * 0.1;
                collision
                    .ray_body(origin, Dir3::NEG_Y, (-vertical_travel).max(0.0) + 0.2)
                    .map(|hit| origin.y - hit.distance)
            });
        if let Some(y) = floor_y {
            if transform.translation.y <= y {
                transform.translation.y = y;
                if selected {
                    landed = true;
                }
            }
        }
        if selected {
            active_pos = Some(transform.translation);
        }
    }
    if let Some(pos) = active_pos {
        player.active = true;
        player.detached = false;
        player.settling = false;
        player.world_stale = false;
        player.pos = pos;
    }
    if landed {
        party.grounded = true;
        party.jump_velocity = 0.0;
    }
}

fn combat(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut party: ResMut<Party>,
    members: Query<(&PartyMember, &Transform)>,
    mut enemies: Query<(&mut Health, &Transform), With<Enemy>>,
) {
    if pointer_release_held(&keys) {
        return;
    }
    let Some((_, hero)) = members.iter().find(|(m, _)| m.0 == party.active) else {
        return;
    };
    let special_slot = if keys.just_pressed(KeyCode::KeyQ) {
        Some(0)
    } else if keys.just_pressed(KeyCode::KeyE) {
        Some(1)
    } else if keys.just_pressed(KeyCode::KeyR) {
        Some(2)
    } else {
        None
    };
    if let Some(slot) = special_slot.filter(|&slot| party.special_cooldowns[slot] == 0.0) {
        party.special_cooldowns[slot] = [7.0, 9.0, 12.0][slot];
        let range = [5.2, 4.0, 6.0][slot];
        let damage = [38.0, 52.0, 65.0][slot];
        for (mut health, target) in &mut enemies {
            if hero.translation.distance(target.translation) < range {
                health.0 -= damage;
            }
        }
    }

    let light_held = mouse.pressed(MouseButton::Left);
    let heavy_held = mouse.pressed(MouseButton::Right);
    if light_held && party.light_cooldown == 0.0 {
        party.light_cooldown = 0.42;
        party.combo = if party.combo_timer > 0.0 {
            party.combo % 3 + 1
        } else {
            1
        };
        party.combo_timer = 1.1;
        let forward = hero.rotation * Vec3::NEG_Z;
        for (mut health, target) in &mut enemies {
            if hero.translation.distance(target.translation) < 3.0
                && forward.dot((target.translation - hero.translation).normalize_or_zero()) > 0.15
            {
                health.0 -= 16.0 + 4.0 * f32::from(party.combo);
            }
        }
    }
    if heavy_held && party.heavy_cooldown == 0.0 {
        party.heavy_cooldown = 1.05;
        let forward = hero.rotation * Vec3::NEG_Z;
        for (mut health, target) in &mut enemies {
            if hero.translation.distance(target.translation) < 3.8
                && forward.dot((target.translation - hero.translation).normalize_or_zero()) > -0.1
            {
                health.0 -= 36.0;
            }
        }
    }
}

fn reap_defeated(
    mut commands: Commands,
    defeated: Query<Entity, (With<Enemy>, With<Health>)>,
    health: Query<&Health>,
) {
    for entity in &defeated {
        if health.get(entity).is_ok_and(|h| h.0 <= 0.0) {
            commands.entity(entity).despawn();
        }
    }
}

fn follow_camera(
    party: Res<Party>,
    members: Query<(&PartyMember, &Transform), Without<WorldCamera>>,
    mut cameras: Query<&mut Transform, With<WorldCamera>>,
    collision: WorldCollision,
) {
    let Some((_, hero)) = members.iter().find(|(m, _)| m.0 == party.active) else {
        return;
    };
    let focus = hero.translation + Vec3::Y * 1.4;
    let pitch = party.camera_pitch;
    let offset = Vec3::new(
        party.camera_yaw.sin() * pitch.cos(),
        pitch.sin(),
        party.camera_yaw.cos() * pitch.cos(),
    ) * 7.0;
    let eye = focus + offset;
    let head = hero.translation + Vec3::Y * 1.8;
    let boom = eye - head;
    let safe_distance = collision
        .cast_camera(head, boom, false)
        .map_or(boom.length(), |distance| (distance - 0.15).max(0.0));
    let eye = head + boom.normalize_or_zero() * safe_distance;
    let transform = Transform::from_translation(eye).looking_at(focus, Vec3::Y);
    for mut camera in &mut cameras {
        *camera = transform;
    }
}
