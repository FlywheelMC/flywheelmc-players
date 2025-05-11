use crate::player::{
    Player,
    PlayerLeft
};
use flywheelmc_common::prelude::*;
use protocol::packet::{
    DecodeError,
    PacketReader, PacketWriter,
    PrefixedPacketDecode, PrefixedPacketEncode, PacketMeta,
    EncodeError,
    processing::PacketProcessing
};
use protocol::packet::{
    BoundC2S, BoundS2C,
    StageStatus, StageLogin, StageConfig, StagePlay
};


pub(crate) mod handshake;
pub(crate) mod status;
pub(crate) mod login;
pub(crate) mod config;
pub(crate) mod play;

pub(crate) mod packet;
pub use packet::{ PacketReadEvent, Packet };
pub(crate) use packet::SetStage;


#[derive(Component)]
pub(crate) struct Connection {
    pub(crate) peer_addr : SocketAddr,
    pub(crate) shutdown  : Arc<AtomicBool>
}

#[derive(Component)]
pub(crate) struct ConnStream {
    pub(crate) read_stream  : OwnedReadHalf,
    pub(crate) write_sender : mpsc::UnboundedSender<(SetStage, Vec<u8>,)>,
    pub(crate) writer_task  : ManuallyDrop<Task<()>>,
    pub(crate) data_queue   : VecDeque<u8>,
    pub(crate) packet_proc  : PacketProcessing,
    pub(crate) packet_index : u128,
    pub(crate) shutdown     : Arc<AtomicBool>
}


impl ConnStream {

    pub fn read_packet<T : PrefixedPacketDecode + PacketMeta<BoundT = BoundC2S>>(&mut self) -> Option<T> {
        let result = PacketReader::from_raw_queue(self.data_queue.iter().cloned()).and_then(
            |(smalldata, consumed,)| {
                for _ in 0..consumed {
                    self.data_queue.pop_front();
                }
                self.packet_proc.compression.decompress(smalldata)
            }
        ).and_then(|mut plaindata| {
            T::decode_prefixed(&mut plaindata)
        });
        match (result) {
            Ok(packet) => Some(packet),
            Err(err) => {
                match (err) {
                    DecodeError::EndOfBuffer => { },
                    DecodeError::InvalidData(_) => { self.shutdown.store(true, AtomicOrdering::Relaxed); } // TODO: Log warning
                    DecodeError::UnconsumedBuffer => { self.shutdown.store(true, AtomicOrdering::Relaxed); }, // TODO: Log warning
                    DecodeError::UnknownPacketPrefix(_) => { } // TODO: Log warning
                }
                None
            }
        }
    }

    pub fn send_packet_config<T : PrefixedPacketEncode + PacketMeta<BoundT = BoundS2C, StageT = StageConfig>>(&mut self, packet : T) -> Result<(), EncodeError> {
        unsafe { self.send_packet(SetStage::Config, packet) }
    }

    pub fn send_packet_play<T : PrefixedPacketEncode + PacketMeta<BoundT = BoundS2C, StageT = StagePlay>>(&mut self, packet : T) -> Result<(), EncodeError> {
        unsafe { self.send_packet(SetStage::Play, packet) }
    }

    pub unsafe fn send_packet_noset<T : PrefixedPacketEncode + PacketMeta<BoundT = BoundS2C>>(&mut self, packet : T) -> Result<(), EncodeError> {
        unsafe { self.send_packet(SetStage::NoSet, packet) }
    }

    pub unsafe fn send_packet<T : PrefixedPacketEncode + PacketMeta<BoundT = BoundS2C>>(&mut self, set_stage : SetStage, packet : T) -> Result<(), EncodeError> {
        let mut plaindata = PacketWriter::new();
        if let Err(err) = packet.encode_prefixed(&mut plaindata) {
            // TODO: Log warning
            self.shutdown.store(true, AtomicOrdering::Relaxed);
            return Err(err);
        }
        let cipherdata = match (self.packet_proc.encode_encrypt(plaindata)) {
            Ok(cipherdata) => cipherdata,
            Err(err) => {
                // TODO: Log warning
                self.shutdown.store(true, AtomicOrdering::Relaxed);
                return Err(err);
            }
        };
        if let Err(_) = self.write_sender.send((set_stage, cipherdata.into_inner(),)) {
            // TODO: Log warning
            self.shutdown.store(true, AtomicOrdering::Relaxed);
            return Err(EncodeError::SendFailed);
        }
        Ok(())
    }

}

impl Drop for ConnStream {
    fn drop(&mut self) {
        let _ = unsafe { ManuallyDrop::take(&mut self.writer_task) }.cancel();
    }
}

#[derive(Component)]
pub(crate) enum ConnKeepalive {
    Sending {
        send_at : Instant
    },
    Waiting {
        expected_id : u64,
        expected_by : Instant
    }
}


pub(crate) fn read_conn_streams(
    mut q_conns : Query<(&mut ConnStream,)>
) {
    let mut buf = [0u8; 128];
    for (mut conn_stream,) in &mut q_conns {
        match (conn_stream.read_stream.try_read(&mut buf)) {
            Ok(count) => {
                conn_stream.data_queue.reserve(count);
                for i in 0..count {
                    if let Ok(b) = conn_stream.packet_proc.secret_cipher.decrypt_u8(buf[i]) {
                        conn_stream.data_queue.push_back(b);
                    } else {
                        // TODO: Log warning
                        conn_stream.shutdown.store(true, AtomicOrdering::Relaxed);
                    }
                }
            },
            Err(err) if (err.kind() == io::ErrorKind::WouldBlock) => { },
            Err(_) => {
                // TODO: Log warning
                conn_stream.shutdown.store(true, AtomicOrdering::Relaxed);
            }
        }
    }
}

pub(crate) fn timeout_conns() {
    // TODO
}

pub(crate) fn shutdown_conns(
    mut cmds    : Commands,
    mut q_conns : Query<(Entity, &Connection, Option<&mut Player>,)>,
    mut ew_left : EventWriter<PlayerLeft>
) {
    for (entity, conn, player,) in &mut q_conns {
        if (conn.shutdown.load(AtomicOrdering::Relaxed)) {
            println!("Shutdown {}", conn.peer_addr);
            if let Some(mut player) = player {
                ew_left.write(PlayerLeft {
                    uuid     : player.uuid,
                    username : mem::replace(&mut player.username, String::new())
                });
            }
            cmds.entity(entity).despawn();
        }
    }
}
