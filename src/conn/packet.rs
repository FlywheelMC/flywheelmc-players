use flywheelmc_common::prelude::*;
use protocol::packet::c2s::config::C2SConfigPackets;
use protocol::packet::c2s::play::C2SPlayPackets;


#[derive(Event)]
pub struct PacketReadEvent {
    pub entity : Entity,
    pub packet : IncomingPacket,
    pub index  : u128
}


#[derive(Debug)]
pub enum IncomingPacket {
    Config(C2SConfigPackets),
    Play(C2SPlayPackets)
}

impl From<C2SConfigPackets> for IncomingPacket {
    fn from(value : C2SConfigPackets) -> Self {
        Self::Config(value)
    }
}

impl From<C2SPlayPackets> for IncomingPacket {
    fn from(value : C2SPlayPackets) -> Self {
        Self::Play(value)
    }
}
