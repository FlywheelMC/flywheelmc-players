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
    SetChunkCacheRadiusS2CPlayPacket
};
use protocol::value::BlockState;
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
    chunks : BTreeMap<Vec2<i32>, Chunk>
}

pub struct Chunk {
    sections : Vec<ChunkSection>
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
            trace!("Updating chunk centre of peer {} to <{}, {}>", conn.peer_addr, chunk_centre.0.x, chunk_centre.0.y);
            let _ = conn.send_packet_play(SetChunkCacheCenterS2CPlayPacket {
                chunk_x : chunk_centre.0.x.into(),
                chunk_z : chunk_centre.0.y.into(),
            });
        }
        if (Ordered::take_dirty(&mut view_dist.0)) {
            trace!("Updating chunk radius of peer {} to {}", conn.peer_addr, *view_dist.0);
            let _ = conn.send_packet_play(SetChunkCacheRadiusS2CPlayPacket {
                view_dist : (view_dist.0.get() as i32).into()
            });
        }
    }
}
