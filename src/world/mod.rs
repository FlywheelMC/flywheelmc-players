use crate::MaxViewDistance;
use crate::conn::ConnStream;
use crate::conn::packet::{ PacketReadEvent, Packet };
use flywheelmc_common::prelude::*;
use protocol::packet::c2s::config::{
    C2SConfigPackets,
    ClientInformationC2SConfigPacket
};
use protocol::packet::c2s::play::{
    C2SPlayPackets,
    ClientInformationC2SPlayPacket
};
use protocol::packet::s2c::play::SetChunkCacheRadiusS2CPlayPacket;
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


pub(crate) fn update_view_distance(
    mut q_conns     : Query<(&mut ConnStream, &mut ViewDistance,)>,
    mut er_packet   : EventReader<PacketReadEvent>,
        r_view_dist : Res<MaxViewDistance>
) {
    for PacketReadEvent { entity, packet, index } in er_packet.read() {
        if let Ok((_, mut view_dist,)) = q_conns.get_mut(*entity) {
            if let Packet::Config(C2SConfigPackets::ClientInformation(ClientInformationC2SConfigPacket { info }))
                | Packet::Play(C2SPlayPackets::ClientInformation(ClientInformationC2SPlayPacket { info })) = packet
            {
                if let Some(value) = NonZeroU8::new(info.view_distance) {
                    Ordered::set(&mut view_dist.0, value.min(r_view_dist.0), *index);
                }
            }
        }
    }
    for (mut conn_stream, mut view_dist,) in &mut q_conns {
        if (Ordered::take_dirty(&mut view_dist.0)) {
            let _ = conn_stream.send_packet_play(SetChunkCacheRadiusS2CPlayPacket {
                view_dist : (view_dist.0.get() as i32).into()
            });
        }
    }
}
