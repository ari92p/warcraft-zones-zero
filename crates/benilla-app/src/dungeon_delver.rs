//! A small offline third-person party-combat slice layered over the faithful 1.12.1 client.
//! Stock world assets, creature attachment and animation remain owned by benilla.

use benilla_protocol::{messages::ObjectFields, EntityKind};
use benilla_world::{schedule::WorldStage, view::WorldCamera, world_point::WorldPoint};
use bevy::prelude::*;

use crate::{
    creature_anim::{move_flags, MovementState},
    net::{Guid, NetEntity, ObjectStore, ScriptedFacing},
    player::Player,
    run_mode::CaptureMode,
};

const SCARLET_MONASTERY: u32 = 189;
// Scarlet Monastery Cathedral, map 189 (WoW coordinates, converted below to Bevy axes).
const START_WOW: [f32; 3] = [855.11, 1320.76, 18.67];

pub(crate) fn active() -> bool {
    std::env::var_os("WOW_DUNGEON_DELVER").is_some()
}

pub(crate) struct DungeonDelverPlugin;

impl Plugin for DungeonDelverPlugin {
    fn build(&self, app: &mut App) {
        // Disable stock glue/UI/controller input; our game layer owns the in-world session.
        app.insert_resource(CaptureMode)
            .init_resource::<Party>()
            .add_systems(OnEnter(crate::char_select::ClientState::InWorld), spawn_run)
            .add_systems(
                Update,
                (control_party, combat, reap_defeated, follow_camera)
                    .chain()
                    .in_set(WorldStage::Input),
            );
        info!("Dungeon Delver: Scarlet Monastery prototype; WASD move, Q/E turn, 1-3 swap, Space attack, Shift special");
    }
}

#[derive(Component)]
struct PartyMember(usize);

#[derive(Component)]
struct Enemy;

#[derive(Component)]
struct Health(f32);

#[derive(Resource, Default)]
struct Party {
    active: usize,
    facing: f32,
    attack_cooldown: f32,
    combo: u8,
    combo_timer: f32,
    special_cooldown: f32,
}

fn wow_to_bevy(p: [f32; 3]) -> Vec3 {
    Vec3::new(-p[1], p[2], -p[0])
}

fn spawn_unit(commands: &mut Commands, serial: u64, display: u32, pos: Vec3, faction: u32) -> Entity {
    const HIGH_CREATURE: u64 = 0xF130 << 48;
    let guid = HIGH_CREATURE | serial;
    let fields = ObjectFields::from_pairs(&[
        (0, guid as u32), (1, (guid >> 32) as u32), (2, 0x9),
        (4, 1.0f32.to_bits()), (22, 100), (28, 100), (34, 30), (35, faction),
        // Human male, warrior, physical power; scarlet enemies use the same renderable human rig.
        (36, 1 | (1 << 8)),
    ]);
    commands.spawn((
        Guid(guid),
        NetEntity { kind: EntityKind::Unit, display_id: Some(display), scale: 1.0 },
        ObjectStore(fields), MovementState::default(), ScriptedFacing,
        Transform::from_translation(pos), Visibility::default(),
    )).id()
}

fn spawn_run(mut commands: Commands, mut map: Option<ResMut<benilla_world::world_map::CurrentMap>>) {
    if let Some(mut map) = map {
        map.0 = SCARLET_MONASTERY;
    }
    let origin = wow_to_bevy(START_WOW);
    // Party looks use familiar 1.12 human display models; enemies use a Scarlet Crusader look.
    for (i, display) in [49, 50, 51].into_iter().enumerate() {
        let pos = origin + Vec3::new((i as f32 - 1.0) * 1.8, 0.0, 2.0 + (i as f32 * 0.4));
        let entity = spawn_unit(&mut commands, 0xD311_000 + i as u64, display, pos, 35);
        commands.entity(entity).insert((PartyMember(i), Health(100.0)));
    }
    for i in 0..4 {
        let pos = origin + Vec3::new((i as f32 - 1.5) * 2.2, 0.0, -8.0 - (i as f32 % 2.0) * 2.5);
        let entity = spawn_unit(&mut commands, 0xD311_100 + i as u64, 1563, pos, 14);
        commands.entity(entity).insert((Enemy, Health(80.0)));
    }
    info!("Scarlet Monastery loaded. Prototype controls: WASD, Q/E, 1-3, Space, Shift.");
}

fn control_party(
    time: Res<Time>, keys: Res<ButtonInput<KeyCode>>, world: WorldPoint,
    mut party: ResMut<Party>, mut player: ResMut<Player>,
    mut members: Query<(&PartyMember, &mut Transform, &mut MovementState)>,
) {
    let dt = time.delta_secs();
    party.attack_cooldown = (party.attack_cooldown - dt).max(0.0);
    party.special_cooldown = (party.special_cooldown - dt).max(0.0);
    party.combo_timer = (party.combo_timer - dt).max(0.0);
    if party.combo_timer == 0.0 { party.combo = 0; }
    if keys.just_pressed(KeyCode::Digit1) { party.active = 0; }
    if keys.just_pressed(KeyCode::Digit2) { party.active = 1; }
    if keys.just_pressed(KeyCode::Digit3) { party.active = 2; }
    if keys.just_pressed(KeyCode::Tab) { party.active = (party.active + 1) % 3; }

    let turn = keys.pressed(KeyCode::KeyE) as i8 as f32 - keys.pressed(KeyCode::KeyQ) as i8 as f32;
    party.facing += turn * dt * 2.2;
    let forward = Vec3::new(-party.facing.sin(), 0.0, -party.facing.cos());
    let right = Vec3::new(forward.z, 0.0, -forward.x);
    let mut dir = Vec3::ZERO;
    if keys.pressed(KeyCode::KeyW) { dir += forward; }
    if keys.pressed(KeyCode::KeyS) { dir -= forward; }
    if keys.pressed(KeyCode::KeyD) { dir += right; }
    if keys.pressed(KeyCode::KeyA) { dir -= right; }
    let travel = if dir == Vec3::ZERO { Vec3::ZERO } else { dir.normalize() * 6.0 * dt };

    let mut active_pos = None;
    for (member, mut transform, mut movement) in &mut members {
        let selected = member.0 == party.active;
        if travel != Vec3::ZERO {
            transform.translation += travel;
        }
        if selected && travel != Vec3::ZERO {
            party.facing = (-travel.x).atan2(-travel.z);
            movement.speed = 6.0;
            movement.flags = move_flags::FORWARD;
        } else {
            movement.speed = 0.0;
            movement.flags = 0;
        }
        transform.rotation = Quat::from_rotation_y(party.facing);
        if let Some(y) = world.terrain_height_under(transform.translation) {
            transform.translation.y = y;
        }
        if selected { active_pos = Some(transform.translation); }
    }
    if let Some(pos) = active_pos {
        player.active = true;
        player.detached = false;
        player.settling = false;
        player.world_stale = false;
        player.pos = pos;
    }
}

fn combat(
    keys: Res<ButtonInput<KeyCode>>, mouse: Res<ButtonInput<MouseButton>>,
    mut party: ResMut<Party>, members: Query<(&PartyMember, &Transform)>,
    mut enemies: Query<(&mut Health, &Transform), With<Enemy>>,
) {
    let Ok((_, hero)) = members.iter().find(|(m, _)| m.0 == party.active) else { return; };
    let special = keys.just_pressed(KeyCode::ShiftLeft) || keys.just_pressed(KeyCode::ShiftRight);
    let attack = keys.just_pressed(KeyCode::Space) || mouse.just_pressed(MouseButton::Left);
    if special && party.special_cooldown == 0.0 {
        party.special_cooldown = 8.0;
        for (mut health, target) in &mut enemies {
            if hero.translation.distance(target.translation) < 5.0 { health.0 -= 45.0; }
        }
    } else if attack && party.attack_cooldown == 0.0 {
        party.attack_cooldown = 0.48;
        party.combo = if party.combo_timer > 0.0 { party.combo % 3 + 1 } else { 1 };
        party.combo_timer = 1.1;
        let forward = hero.rotation * Vec3::NEG_Z;
        for (mut health, target) in &mut enemies {
            if hero.translation.distance(target.translation) < 3.2
                && forward.dot((target.translation - hero.translation).normalize_or_zero()) > 0.15
            {
                health.0 -= 18.0 + 4.0 * f32::from(party.combo);
            }
        }
    }
}

fn reap_defeated(mut commands: Commands, defeated: Query<Entity, (With<Enemy>, With<Health>)>, health: Query<&Health>) {
    for entity in &defeated {
        if health.get(entity).is_ok_and(|h| h.0 <= 0.0) {
            commands.entity(entity).despawn();
        }
    }
}

fn follow_camera(
    party: Res<Party>, members: Query<(&PartyMember, &Transform), Without<WorldCamera>>,
    mut cameras: Query<&mut Transform, With<WorldCamera>>,
) {
    let Some((_, hero)) = members.iter().find(|(m, _)| m.0 == party.active) else { return; };
    let focus = hero.translation + Vec3::Y * 1.4;
    let eye = focus - hero.rotation * Vec3::NEG_Z * 7.0 + Vec3::Y * 3.2;
    let transform = Transform::from_translation(eye).looking_at(focus, Vec3::Y);
    for mut camera in &mut cameras { *camera = transform; }
}
