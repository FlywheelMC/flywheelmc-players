use flywheelmc_common::prelude::*;
use protocol::packet::{
    PacketWriter, EncodeError,
    PrefixedPacketEncode,
    PacketMeta, BoundS2C
};
use protocol::packet::c2s::config::C2SConfigPackets;
use protocol::packet::c2s::play::C2SPlayPackets;


#[derive(Event)]
pub struct PacketReadEvent {
    pub entity : Entity,
    pub packet : Packet,
    pub index  : u128
}


#[derive(Debug)]
pub enum Packet {
    Config(C2SConfigPackets),
    Play(C2SPlayPackets)
}

impl From<C2SConfigPackets> for Packet {
    fn from(value : C2SConfigPackets) -> Self {
        Self::Config(value)
    }
}

impl From<C2SPlayPackets> for Packet {
    fn from(value : C2SPlayPackets) -> Self {
        Self::Play(value)
    }
}

#[derive(Debug)]
pub(crate) enum SetStage {
    NoSet,
    Config,
    Play
}
#[derive(Debug)]
enum CurrentStage {
    Startup,
    Config,
    Play
}


pub(crate) async fn run_packet_writer(
    mut write_receiver : mpsc::UnboundedReceiver<(SetStage, Vec<u8>,)>,
    mut stream         : OwnedWriteHalf,
        shutdown       : Arc<AtomicBool>,
        send_timeout   : Duration
) -> () {
    let mut current_stage = CurrentStage::Startup;
    loop {
        match (write_receiver.try_recv()) {
            Ok((set_stage, packet,)) => {
                todo!("set stage");
                /*match (task::timeout(send_timeout, stream.write_all(&packet)).await) {
                    Ok(Ok(_)) => { },
                    Ok(Err(err)) => {
                        // TODO: Log warning
                        shutdown.store(true, AtomicOrdering::Relaxed);
                        break;
                    }
                    Err(_) => {
                        // TODO: Log warning (timed out)
                        shutdown.store(true, AtomicOrdering::Relaxed);
                        break;
                    }
                }*/
            },
            Err(mpsc::TryRecvError::Empty) => { },
            Err(mpsc::TryRecvError::Disconnected) => { break; }
        }
        task::yield_now().await;
    }
}
