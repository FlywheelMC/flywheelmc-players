use crate::MaxViewDistance;
use crate::conn::Connection;
use crate::conn::packet::{ PacketReadEvent, Packet };
use crate::conn::play::ConnStatePlay;
use flywheelmc_common::prelude::*;
use protocol::packet::c2s::config::{
    C2SConfigPackets,
    ClientInformationC2SConfigPacket
};
use protocol::packet::c2s::play::{
    C2SPlayPackets,
    ClientInformationC2SPlayPacket
};
use protocol::packet::s2c::play::{
    SetChunkCacheCenterS2CPlayPacket,
    SetChunkCacheRadiusS2CPlayPacket,
    LevelChunkWithLightS2CPlayPacket
};
use protocol::value::{ Identifier, BlockState, DimType, Nbt };
use protocol::registry::RegEntry;


mod chunk;
pub use chunk::*;

mod chunk_section;
pub use chunk_section::*;

mod setbatch;
use setbatch::*;

mod action;
pub use action::*;


const BLOCK_AIR : RegEntry<BlockState> = unsafe { RegEntry::new_unchecked(0) };


#[derive(Component)]
pub struct ChunkCentre(pub(crate) Dirty<Vec2<i32>>);
pub type ChunkCenter = ChunkCentre;

#[derive(Component)]
pub struct ViewDistance(pub(crate) Ordered<NonZeroU8>);


#[derive(Component)]
pub struct World {
    #[expect(dead_code)]
    pub(crate) dim_id       : Identifier,
    pub(crate) dim_type     : DimType,
    pub(crate) chunks       : BTreeMap<Vec2<i32>, Chunk>,
    pub(crate) newly_loaded : Vec<Vec2<i32>>
}

#[derive(Component)]
pub struct PlayerInWorld;


pub(crate) fn read_settings_updates(
    mut q_conns     : Query<(&mut ViewDistance,),>,
    mut er_packet   : EventReader<PacketReadEvent>,
        r_view_dist : Res<MaxViewDistance>
) {
    for PacketReadEvent { entity, packet, index } in er_packet.read() {
        if let Ok((mut view_dist,)) = q_conns.get_mut(*entity)
            && let    Packet::Config(C2SConfigPackets::ClientInformation(ClientInformationC2SConfigPacket { info }))
                    | Packet::Play(C2SPlayPackets::ClientInformation(ClientInformationC2SPlayPacket { info })) = packet
            && let Some(value) = NonZeroU8::new(info.view_distance)
        { Ordered::set(&mut view_dist.0, value.min(r_view_dist.0), *index); }
    }
}

pub(crate) fn update_chunk_view(
    mut q_conns : Query<(&mut Connection, &mut ChunkCentre, &mut ViewDistance,), (With<ConnStatePlay>,)>
) {
    for (mut conn, mut chunk_centre, mut view_dist,) in &mut q_conns {
        if (Dirty::take_dirty(&mut chunk_centre.0)) {
            trace!("Updating chunk centre of peer {} to <{}, {}>", conn.peer_addr(), chunk_centre.0.x, chunk_centre.0.y);
            let _ = conn.send_packet_play(SetChunkCacheCenterS2CPlayPacket {
                chunk_x : chunk_centre.0.x.into(),
                chunk_z : chunk_centre.0.y.into(),
            });
        }
        if (Ordered::take_dirty(&mut view_dist.0)) {
            trace!("Updating chunk radius of peer {} to {}", conn.peer_addr(), *view_dist.0);
            let _ = conn.send_packet_play(SetChunkCacheRadiusS2CPlayPacket {
                view_dist : (view_dist.0.get() as i32).into()
            });
        }
    }
}

#[expect(clippy::type_complexity)]
pub(crate) fn load_chunks(
    mut q_conns : Query<(Entity, &mut Connection, &mut World, &ChunkCentre, &ViewDistance), (With<ConnStatePlay>, With<PlayerInWorld>,)>,
    mut ew_load : EventWriter<WorldChunkLoading>
) {
    for (_, mut conn, mut world, _, _,) in &mut q_conns {
        let newly_loaded = mem::take(&mut world.newly_loaded);
        for pos in newly_loaded {
            if let Some(chunk) = world.chunks.get_mut(&pos) {
                let data = chunk.ptc_chunk_section_data();
                for section in &mut chunk.sections {
                    section.clear_dirty();
                }
                let _ = conn.send_packet_play(LevelChunkWithLightS2CPlayPacket {
                    chunk_x                : pos.x,
                    chunk_z                : pos.y,
                    data,
                    heightmaps             : Nbt::new(),
                    block_entities         : Vec::new().into(),
                    sky_light_mask         : Vec::new().into(),
                    block_light_mask       : Vec::new().into(),
                    empty_sky_light_mask   : Vec::new().into(),
                    empty_block_light_mask : Vec::new().into(),
                    sky_light_array        : Vec::new().into(),
                    block_light_array      : Vec::new().into()
                });
            }
        }
    }

    for (entity, mut conn, mut world, chunk_centre, view_dist) in &mut q_conns {

        // TODO: Unload out-of-range chunks.

        // Update loaded chunks.
        for (cpos, chunk,) in &mut world.chunks {
            for (y, section) in chunk.sections.iter_mut().enumerate() {
                if let Some(packet) = section.ptc_update_section([cpos.x, y as i32, cpos.y,]) {
                    warn!("{} {} {}", cpos.x, y, cpos.y);
                    section.clear_dirty();
                    let _ = conn.send_packet_play(packet);
                }
            }
        }

        // Queue new chunks for load.
        try_load_chunk(entity, &conn, &mut ew_load, &mut world, *chunk_centre.0);
        for radius in 1..=(view_dist.0.get() as i32) {
            let edge_len = 2 * radius;
            for corner_cx in [-1i32, 1] {
                for corner_cz in [-1i32, 1] {
                    let shift_cx = (corner_cz != corner_cx) as i32 * -corner_cx.signum();
                    let shift_cz = (corner_cz == corner_cx) as i32 * -corner_cz.signum();
                    for i in 0..edge_len {
                        let offset_cx = (radius * corner_cx) + (i * shift_cx);
                        let offset_cz = (radius * corner_cz) + (i * shift_cz);
                        try_load_chunk(entity, &conn, &mut ew_load, &mut world, Vec2::new(
                            chunk_centre.0.x + offset_cx,
                            chunk_centre.0.y + offset_cz
                        ));
                    }
                }
            }
        }

    }
}
fn try_load_chunk(
    entity  : Entity,
    conn    : &Connection,
    ew_load : &mut EventWriter<WorldChunkLoading>,
    world   : &mut World,
    pos     : Vec2<i32>
) {
    if (! world.chunks.contains_key(&pos)) {
        world.chunks.insert(pos, Chunk {
            sections : {
                let     section  = ChunkSection::empty();
                let     count    = (world.dim_type.height / 16).max(1);
                let mut sections = Vec::with_capacity(count as usize);
                for _ in 0..(count.saturating_sub(1)) {
                    sections.push(section.clone());
                }
                sections.push(section);
                sections
            }
        });
        world.newly_loaded.push(pos);

        trace!("Loading chunk <{}, {}> for peer {}", pos.x, pos.y, conn.peer_addr());
        ew_load.write(WorldChunkLoading { entity, pos });
    }
}

// TODO: Unload chunks


fn in_section_block_linearise(dx : u8, dy : u8, dz : u8) -> u16 {
    (((dy & 0b00001111) as u16) << 8)
    | (((dz & 0b00001111) as u16) << 4)
    | ((dx & 0b00001111) as u16)
}

fn in_section_block_delinearise(l : u16) -> [u8; 3] {
    [
        (l as u8) & 0b00001111,
        ((l >> 8) as u8) & 0b00001111,
        ((l >> 4) as u8) & 0b00001111
    ]
}
