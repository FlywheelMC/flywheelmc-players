use crate::player::{
    Player,
    PlayerLeft
};
use flywheelmc_common::prelude::*;
use voxidian_protocol::packet::{
    DecodeError,
    PacketReader, PacketWriter,
    PrefixedPacketDecode, PrefixedPacketEncode,
    processing::PacketProcessing
};


pub(crate) mod handshake;
pub(crate) mod status;
pub(crate) mod login;
pub(crate) mod config;
pub(crate) mod play;


#[derive(Component)]
pub(crate) struct Connection {
    pub(crate) peer_addr : SocketAddr,
    pub(crate) shutdown  : Arc<AtomicBool>
}

#[derive(Component)]
pub(crate) struct ConnStream {
    pub(crate) read_stream  : OwnedReadHalf,
    pub(crate) write_stream : Arc<Mutex<OwnedWriteHalf>>,
    pub(crate) data_queue   : VecDeque<u8>,
    pub(crate) packet_proc  : PacketProcessing,
    pub(crate) shutdown     : Arc<AtomicBool>
}


impl ConnStream {

    pub fn read_packet<T : PrefixedPacketDecode>(&mut self) -> Option<T> {
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

    pub fn send_packet<T : PrefixedPacketEncode>(&mut self, cmds : &mut Commands, packet : T) -> () {
        let mut plaindata = PacketWriter::new();
        if let Err(_) = packet.encode_prefixed(&mut plaindata) {
            // TODO: Log warning
            self.shutdown.store(true, AtomicOrdering::Relaxed); return;
        }
        let cipherdata = match (self.packet_proc.encode_encrypt(plaindata)) {
            Ok(cipherdata) => cipherdata,
            Err(_) => {
                // TODO: Log warning
                self.shutdown.store(true, AtomicOrdering::Relaxed); return;
            }
        };
        let write_stream = Arc::clone(&self.write_stream);
        let shutdown     = Arc::clone(&self.shutdown);
        cmds.spawn_task(async move || {
            let mut write_stream = write_stream.lock().await;
            if let Err(_) = write_stream.write(cipherdata.as_slice()).await {
                // TODO: Log warning
                shutdown.store(true, AtomicOrdering::Relaxed);
            };
            Ok(())
        });
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
    mut q_conn_streams : Query<(&mut ConnStream,)>
) {
    let mut buf = [0u8; 128];
    for (mut conn_stream,) in &mut q_conn_streams {
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
