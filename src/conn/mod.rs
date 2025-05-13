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
use protocol::packet::c2s::config::{
    C2SConfigPackets,
    KeepAliveC2SConfigPacket
};
use protocol::packet::c2s::play::{
    C2SPlayPackets,
    KeepAliveC2SPlayPacket
};
use protocol::packet::s2c::play::KeepAliveS2CPlayPacket;


pub(crate) mod handshake;
pub(crate) mod status;
pub(crate) mod login;
pub(crate) mod play;

pub(crate) mod packet;


const KEEPALIVE_INTERVAL : Duration = Duration::from_millis(2500);
const KEEPALIVE_TIMEOUT  : Duration = Duration::from_millis(5000);
const MAX_KEEPALIVE_ID   : u64      = i64::MAX as u64;


#[derive(Component)]
pub(crate) struct Connection {
    pub(crate) peer_addr    : SocketAddr,
    pub(crate) read_stream  : OwnedReadHalf,
    pub(crate) write_sender : mpsc::UnboundedSender<(ShortName<'static>, packet::SetStage, Vec<u8>,)>,
    pub(crate) stage_sender : mpsc::UnboundedSender<packet::NextStage>,
    pub(crate) writer_task  : ManuallyDrop<Task<()>>,
    pub(crate) data_queue   : VecDeque<u8>,
    pub(crate) packet_proc  : PacketProcessing,
    pub(crate) packet_index : u128,
    pub(crate) shutdown     : Arc<AtomicBool>
}

#[derive(Component)]
pub(crate) enum ConnKeepalive {
    Sending {
        sending_at : Instant
    },
    Waiting {
        expected_id : u64,
        expected_by : Instant
    }
}


impl Connection {

    pub fn read_packet<T>(&mut self) -> Option<T>
    where
        T : PrefixedPacketDecode + PacketMeta<BoundT = BoundC2S>
    {
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

    pub fn send_packet_config<T>(&mut self, packet : T) -> Result<(), EncodeError>
    where
        T : PrefixedPacketEncode + PacketMeta<BoundT = BoundS2C, StageT = StageConfig>
    { unsafe { self.send_packet(packet::SetStage::Config, packet) } }

    pub fn send_packet_play<T>(&mut self, packet : T) -> Result<(), EncodeError>
    where
        T : PrefixedPacketEncode + PacketMeta<BoundT = BoundS2C, StageT = StagePlay>
    { unsafe { self.send_packet(packet::SetStage::Play, packet) } }

    pub unsafe fn send_packet_noset<T>(&mut self, packet : T) -> Result<(), EncodeError>
    where
        T : PrefixedPacketEncode + PacketMeta<BoundT = BoundS2C>
    { unsafe { self.send_packet(packet::SetStage::NoSet, packet) } }

    pub unsafe fn send_packet<T>(&mut self, set_stage : packet::SetStage, packet : T) -> Result<(), EncodeError>
    where
        T : PrefixedPacketEncode + PacketMeta<BoundT = BoundS2C>
    {
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
        if let Err(err) = self.write_sender.send((ShortName::of::<T>(), set_stage, cipherdata.into_inner(),)) {
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
            Ok(0) => {
                // Disconnected.
                conn.shutdown.store(true, AtomicOrdering::Relaxed);
            },
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

pub(crate) fn timeout_conns(
    mut q_conns    : Query<(&mut Connection, &mut ConnKeepalive,), (With<play::ConnStatePlay>,)>,
    mut er_packets : EventReader<packet::PacketReadEvent>
) {
    for (mut conn, mut keepalive,) in &mut q_conns {
        match (*keepalive) {
            ConnKeepalive::Sending { sending_at } => { if (Instant::now() >= sending_at) {
                let sending_id = random_range(0..=MAX_KEEPALIVE_ID);
                trace!("Sending keepalive {} to peer {}", sending_id, conn.peer_addr);
                *keepalive = ConnKeepalive::Waiting { expected_id : sending_id, expected_by : Instant::now() + KEEPALIVE_TIMEOUT };
                let _ = conn.send_packet_play(KeepAliveS2CPlayPacket(sending_id));
            } },
            ConnKeepalive::Waiting { expected_by, .. } => { if (Instant::now() >= expected_by) {
                warn!("Peer {} timed out", conn.peer_addr);
                conn.shutdown.store(true, AtomicOrdering::Relaxed);
            } }
        }
    }
    for packet::PacketReadEvent { entity, packet, .. } in er_packets.read() {
        trace!("Received {} {:?}", entity, packet);
        if let packet::Packet::Play(C2SPlayPackets::KeepAlive(KeepAliveC2SPlayPacket(id)))
            | packet::Packet::Config(C2SConfigPackets::KeepAlive(KeepAliveC2SConfigPacket(id))) = packet
        {
            if let Ok((conn, mut keepalive,)) = q_conns.get_mut(*entity) {
                match (*keepalive) {
                    ConnKeepalive::Sending { .. } => {
                        error!("Received unordered keepalive from peer {}", conn.peer_addr);
                        conn.shutdown.store(true, AtomicOrdering::Relaxed);
                    },
                    ConnKeepalive::Waiting { expected_id, .. } => {
                        if (*id == expected_id) {
                            trace!("Received keepalive {} from peer {}", id, conn.peer_addr);
                            *keepalive = ConnKeepalive::Sending { sending_at : Instant::now() + KEEPALIVE_INTERVAL };
                        } else {
                            error!("Received unordered keepalive from peer {}", conn.peer_addr);
                            conn.shutdown.store(true, AtomicOrdering::Relaxed);
                        }
                    }
                }
            }
        }
    }
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
