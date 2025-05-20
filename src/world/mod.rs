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
use protocol::value::ChunkSectionData as PtcChunkSectionData;
use protocol::registry::RegEntry;


mod chunk_section;
pub use chunk_section::*;


const BLOCK_AIR : RegEntry<BlockState> = unsafe { RegEntry::new_unchecked(0) };


#[derive(Component)]
pub struct ChunkCentre(pub(crate) Dirty<Vec2<i32>>);
pub type ChunkCenter = ChunkCentre;

#[derive(Component)]
pub struct ViewDistance(pub(crate) Ordered<NonZeroU8>);


#[derive(Component)]
pub struct World {
    pub(crate) dim_id       : Identifier,
    pub(crate) dim_type     : DimType,
    pub(crate) chunks       : BTreeMap<Vec2<i32>, Chunk>,
    pub(crate) newly_loaded : Vec<Vec2<i32>>
}

#[derive(Component)]
pub struct PlayerInWorld;

pub struct Chunk {
    sections : Vec<ChunkSection>
}


impl Chunk {

    pub(crate) fn ptc_chunk_section_data(&self) -> PtcChunkSectionData {
        let mut sections = Vec::with_capacity(self.sections.len());
        for section in &self.sections {
            sections.push(section.ptc_chunk_section());
        }
        PtcChunkSectionData { sections }
    }

}


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

    // Queue new chunks for load.
    for (entity, conn, mut world, chunk_centre, view_dist) in &mut q_conns {
        let v = view_dist.0.get() as i32;
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
                let mut section  = ChunkSection::empty();
                let mut count    = (world.dim_type.height / 16).max(1);
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


#[derive(Event)]
#[non_exhaustive]
pub struct WorldChunkLoading {
    pub entity : Entity,
    pub pos    : Vec2<i32>
}


#[derive(Event)]
pub struct WorldChunkActionEvent {
    pub entity : Entity,
    pub action : WorldChunkAction
}

pub enum WorldChunkAction {

    Set {
        blocks : Vec<(Vec3<i64>, String, Vec<(String, String,)>)>
    }

}


pub(crate) fn handle_actions(
    mut q_worlds  : Query<(&mut World,)>,
    mut er_action : EventReader<WorldChunkActionEvent>
) {
    for WorldChunkActionEvent { entity, action } in er_action.read() {
        if let Ok((mut world,)) = q_worlds.get_mut(*entity) {
            match (action) {

                WorldChunkAction::Set { blocks } => {
                    let mut sections = BTreeMap::new();
                    for (block_pos, block_id, states,) in blocks {
                        let chunk_pos   = Vec2::new((block_pos.x / 16) as i32, (block_pos.z / 16) as i32);
                        if (world.chunks.contains_key(&chunk_pos)) {
                            let block_id    = Identifier::from(block_id);
                            if let Some(mut block_state) = BlockState::default_for(&block_id) {
                                for (state, value,) in states {
                                    if (block_state.properties.contains_key(state)) {
                                        block_state.properties.insert(state.clone(), value.clone());
                                    }
                                }
                                if let Some(block_id) = block_state.to_id() {
                                    let section_pos    = Vec3::new(chunk_pos.x, (block_pos.y / 16) as i32, chunk_pos.y);
                                    let in_section_pos = Vec3::new(block_pos.x.rem_euclid(16) as u8, block_pos.y.rem_euclid(16) as u8, block_pos.z.rem_euclid(16) as u8);
                                    let section        = sections.entry(section_pos).or_insert(BTreeMap::new());
                                    section.insert(in_section_pos, unsafe { RegEntry::new_unchecked(block_id as u32) });
                                }
                            }
                        }
                    }
                    for (section_pos, blocks) in sections {
                        let chunk_pos = Vec2::new(section_pos.x, section_pos.z);
                        if let Some(chunk) = world.chunks.get_mut(&chunk_pos) {
                            if let Some(section) = chunk.sections.get_mut(section_pos.y as usize) {
                                let mut section_writer = section.writer();
                                for (in_section_pos, block,) in blocks {
                                    section_writer.set_xyz(in_section_pos.x, in_section_pos.y, in_section_pos.z, block);
                                }
                            }
                        }
                    }
                }

            }
        }
    }
}
