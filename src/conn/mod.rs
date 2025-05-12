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
    StageConfig, StagePlay
};


pub(crate) mod handshake;
pub(crate) mod status;
pub(crate) mod login;
pub(crate) mod config;
pub(crate) mod play;

pub(crate) mod packet;


#[derive(Component)]
pub(crate) struct Connection {
    pub(crate) peer_addr    : SocketAddr,
    pub(crate) read_stream  : OwnedReadHalf,
    pub(crate) write_sender : mpsc::UnboundedSender<(packet::SetStage, Vec<u8>,)>,
    pub(crate) stage_sender : mpsc::UnboundedSender<packet::NextStage>,
    pub(crate) writer_task  : ManuallyDrop<Task<()>>,
    pub(crate) data_queue   : VecDeque<u8>,
    pub(crate) packet_proc  : PacketProcessing,
    pub(crate) packet_index : u128,
    pub(crate) shutdown     : Arc<AtomicBool>
}


impl Connection {

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
                    DecodeError::InvalidData(err) => {
                        error!("Failed to decode packet from peer {}: {}", self.peer_addr, err);
                        self.shutdown.store(true, AtomicOrdering::Relaxed);
                    }
                    DecodeError::UnconsumedBuffer => {
                        error!("Failed to decode packet from peer {}: {}", self.peer_addr, DecodeError::UnconsumedBuffer);
                        self.shutdown.store(true, AtomicOrdering::Relaxed);
                    },
                    DecodeError::UnknownPacketPrefix(prefix) => {
                        warn!("Received packet with unknown prefix {:#04X} from peer {}", prefix, self.peer_addr);
                    }
                }
                None
            }
        }
    }

    pub fn send_packet_config<T : PrefixedPacketEncode + PacketMeta<BoundT = BoundS2C, StageT = StageConfig> + Debug>(&mut self, packet : T) -> Result<(), EncodeError> {
        unsafe { self.send_packet(packet::SetStage::Config, packet) }
    }

    pub fn send_packet_play<T : PrefixedPacketEncode + PacketMeta<BoundT = BoundS2C, StageT = StagePlay> + Debug>(&mut self, packet : T) -> Result<(), EncodeError> {
        unsafe { self.send_packet(packet::SetStage::Play, packet) }
    }

    pub unsafe fn send_packet_noset<T : PrefixedPacketEncode + PacketMeta<BoundT = BoundS2C> + Debug>(&mut self, packet : T) -> Result<(), EncodeError> {
        unsafe { self.send_packet(packet::SetStage::NoSet, packet) }
    }

    pub unsafe fn send_packet<T : PrefixedPacketEncode + PacketMeta<BoundT = BoundS2C> + Debug>(&mut self, set_stage : packet::SetStage, packet : T) -> Result<(), EncodeError> {
        let mut plaindata = PacketWriter::new();
        if let Err(err) = packet.encode_prefixed(&mut plaindata) {
            error!("Failed to encode packet for peer {}: {}", self.peer_addr, err);
            self.shutdown.store(true, AtomicOrdering::Relaxed);
            return Err(err);
        }
        let cipherdata = match (self.packet_proc.encode_encrypt(plaindata)) {
            Ok(cipherdata) => cipherdata,
            Err(err) => {
                error!("Failed to encrypt and compress packet for peer {}: {}", self.peer_addr, err);
                self.shutdown.store(true, AtomicOrdering::Relaxed);
                return Err(err);
            }
        };
        if let Err(err) = self.write_sender.send((set_stage, cipherdata.into_inner(),)) {
            error!("Failed to send packet to peer {}: {}", self.peer_addr, err);
            self.shutdown.store(true, AtomicOrdering::Relaxed);
            return Err(EncodeError::SendFailed);
        }
        Ok(())
    }

}

impl Drop for Connection {
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


pub(crate) async fn run_listener(
    listen_addrs : SocketAddrs
) -> io::Result<()> {
    info!("Starting game server...");
    let listener = TcpListener::bind(&**listen_addrs).await?;
    pass!("Started game server on {}.", listen_addrs);
    loop {
        let (stream, peer_addr,) = listener.accept().await?;
        debug!("Incoming connection from {}.", peer_addr);
        let (read_stream, write_stream,) = stream.into_split();
        let (write_sender, write_receiver,) = mpsc::unbounded_channel();
        let (stage_sender, stage_receiver,) = mpsc::unbounded_channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        AsyncWorld.spawn_bundle((
            Connection {
                peer_addr,
                read_stream,
                write_sender,
                stage_sender,
                writer_task  : ManuallyDrop::new(AsyncWorld.spawn_task(packet::PacketWriterTask {
                    peer_addr,
                    current_stage  : packet::CurrentStage::Startup,
                    write_receiver,
                    stage_receiver,
                    stream         : write_stream,
                    shutdown       : Arc::clone(&shutdown),
                    send_timeout   : Duration::from_millis(250)
                }.run())),
                data_queue   : VecDeque::new(),
                packet_proc  : PacketProcessing::NONE,
                packet_index : 0,
                shutdown
            },
            ConnKeepalive::Sending { send_at : Instant::now() },
            handshake::ConnStateHandshake,
        ));
    }
}

pub(crate) fn read_conn_streams(
    mut q_conns : Query<(&mut Connection,)>
) {
    let mut buf = [0u8; 128];
    for (mut conn,) in &mut q_conns {
        match (conn.read_stream.try_read(&mut buf)) {
            Ok(count) => {
                conn.data_queue.reserve(count);
                for i in 0..count {
                    if let Ok(b) = conn.packet_proc.secret_cipher.decrypt_u8(buf[i]) {
                        conn.data_queue.push_back(b);
                    } else {
                        error!("Failed to decrypt packet from peer {}", conn.peer_addr);
                        conn.shutdown.store(true, AtomicOrdering::Relaxed);
                    }
                }
            },
            Err(err) if (err.kind() == io::ErrorKind::WouldBlock) => { },
            Err(err) => {
                error!("Failed to read packet from peer {}", err);
                conn.shutdown.store(true, AtomicOrdering::Relaxed);
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
            if let Some(mut player) = player {
                ew_left.write(PlayerLeft {
                    uuid     : player.uuid,
                    username : mem::replace(&mut player.username, String::new())
                });
                info!("Player {} ({}) disconnected.", player.username(), player.uuid());
            }
            debug!("Peer {} disconnected.", conn.peer_addr);
            cmds.entity(entity).despawn();
        }
    }
}
