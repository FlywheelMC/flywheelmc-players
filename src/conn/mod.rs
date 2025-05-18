use crate::KICK_FOOTER;
use crate::player::{
    Player,
    PlayerLeft
};
use flywheelmc_common::prelude::*;
use protocol::value::{ Text, TextComponent, TextColour };
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
use protocol::packet::s2c::login::LoginDisconnectS2CLoginPacket;
use protocol::packet::s2c::config::DisconnectS2CConfigPacket;
use protocol::packet::s2c::play::{
    KeepAliveS2CPlayPacket,
    DisconnectS2CPlayPacket
};


pub(crate) mod handshake;
pub(crate) mod status;
pub(crate) mod login;
pub(crate) mod play;

pub(crate) mod packet;


const KEEPALIVE_INTERVAL : Duration = Duration::from_millis(2500);
const KEEPALIVE_TIMEOUT  : Duration = Duration::from_millis(5000);
const MAX_KEEPALIVE_ID   : u64      = i64::MAX as u64;


static ACTIVE_CONNS : AtomicUsize = AtomicUsize::new(0);


enum RealStage {
    Handshake,
    Status,
    Login,
    Config,
    Play
}


#[derive(Component)]
pub(crate) struct Connection {
    peer_addr      : SocketAddr,
    read_stream    : OwnedReadHalf,
    write_sender   : mpsc::UnboundedSender<(ShortName<'static>, packet::SetStage, Vec<u8>,)>,
    stage_sender   : mpsc::UnboundedSender<packet::NextStage>,
    close_receiver : mpsc::Receiver<Cow<'static, str>>,
    writer_task    : Task<()>,
    data_queue     : VecDeque<u8>,
    packet_proc    : PacketProcessing,
    packet_index   : u128,
    real_stage     : RealStage,
    closing        : bool
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

    #[inline]
    pub fn peer_addr(&self) -> SocketAddr { self.peer_addr }

}

impl Connection {

    pub fn read_packet<T>(&mut self) -> Option<T>
    where
        T : PrefixedPacketDecode + PacketMeta<BoundT = BoundC2S> + Debug
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
            Ok(packet) => {
                trace!("Received packet {:?} from peer {}", packet, self.peer_addr);
                Some(packet)
            },
            Err(err) => {
                match (err) {
                    DecodeError::EndOfBuffer => { },
                    DecodeError::InvalidData(err) => {
                        error!("Failed to decode packet from peer {}: {}", self.peer_addr, err);
                        self.kick(&format!("Bad packet: {err}"));
                    }
                    DecodeError::UnconsumedBuffer => {
                        error!("Failed to decode packet from peer {}: {}", self.peer_addr, DecodeError::UnconsumedBuffer);
                        self.kick(&format!("Bad packet: {}", DecodeError::UnconsumedBuffer));
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
    {
        unsafe { self.send_packet(packet::SetStage::Config, packet)?; }
        self.real_stage = RealStage::Config;
        Ok(())
    }

    pub fn send_packet_play<T>(&mut self, packet : T) -> Result<(), EncodeError>
    where
        T : PrefixedPacketEncode + PacketMeta<BoundT = BoundS2C, StageT = StagePlay>
    {
        unsafe { self.send_packet(packet::SetStage::Play, packet)?; }
        self.real_stage = RealStage::Play;
        Ok(())
    }

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
            self.kick("Failed to encode packet");
            return Err(err);
        }
        let cipherdata = match (self.packet_proc.encode_encrypt(plaindata)) {
            Ok(cipherdata) => cipherdata,
            Err(err) => {
                error!("Failed to encrypt and compress packet for peer {}: {}", self.peer_addr, err);
                self.kick("Failed to encrypt and compress packet");
                return Err(err);
            }
        };
        if let Err(err) = self.write_sender.send((ShortName::of::<T>(), set_stage, cipherdata.into_inner(),)) {
            error!("Failed to send packet to peer {}: {}", self.peer_addr, err);
            self.kick("Failed to send packet");
            return Err(EncodeError::SendFailed);
        }
        Ok(())
    }

    pub fn kick(&mut self, reason : &str) {
        self.close();
        info!("Kicking peer {}: {reason}", self.peer_addr);
        let reason = || Text::from(vec![
            {
                let     c = TextComponent::of_literal("");
                let mut c = c.colour(TextColour::RGB(178, 255, 228));
                c.extra.push(TextComponent::of_literal(reason));
                c
            },
            TextComponent::of_literal("\n\n\n"),
            {
                let     c = TextComponent::of_literal("");
                let mut c = c.underline(true);
                c.extra.extend(KICK_FOOTER.read().unwrap().components().iter().cloned());
                c
            },
            TextComponent::of_literal("\n"),
            TextComponent::of_literal(Utc::now().format("%Y-%m-%d %H:%M:%S%.9f %Z%z").to_string())
                .colour(TextColour::DarkGrey)
        ]);
        match (self.real_stage) {
            RealStage::Handshake => { },
            RealStage::Status => { },
            RealStage::Login  => { unsafe { let _ = self.send_packet_noset(LoginDisconnectS2CLoginPacket { reason : reason().to_json() }); } },
            RealStage::Config => { let _ = self.send_packet_config(DisconnectS2CConfigPacket { reason : reason().to_nbt() }); },
            RealStage::Play   => { let _ = self.send_packet_play(DisconnectS2CPlayPacket { reason : reason().to_nbt() }); }
        }
    }

    #[inline]
    pub fn close(&mut self) {
        self.closing = true;
    }

}


pub(crate) async fn run_listener(
    listen_addrs : SocketAddrs
) -> io::Result<()> {
    info!("Starting game server...");
    let listener = TcpListener::bind(&**listen_addrs).await?;
    pass!("Started game server on {}", listen_addrs);
    loop {
        let (stream, peer_addr,) = listener.accept().await?;
        ACTIVE_CONNS.fetch_add(1, AtomicOrdering::Relaxed);
        debug!("Incoming connection from {}", peer_addr);
        let (read_stream, write_stream,) = stream.into_split();
        let (write_sender, write_receiver,) = mpsc::unbounded_channel();
        let (stage_sender, stage_receiver,) = mpsc::unbounded_channel();
        let (close_sender, close_receiver,) = mpsc::channel(1);
        AsyncWorld.spawn_bundle((
            Connection {
                peer_addr,
                read_stream,
                write_sender,
                stage_sender,
                close_receiver,
                writer_task  : AsyncWorld.spawn_task(packet::PacketWriterTask {
                    peer_addr,
                    current_stage  : packet::CurrentStage::Startup,
                    write_receiver,
                    stage_receiver,
                    close_sender,
                    stream         : write_stream,
                    send_timeout   : Duration::from_millis(250)
                }.run()),
                data_queue   : VecDeque::new(),
                packet_proc  : PacketProcessing::NONE,
                packet_index : 0,
                real_stage   : RealStage::Handshake,
                closing      : false
            },
            handshake::ConnStateHandshake,
        ));
    }
}

pub(crate) fn read_conn_streams(
    mut q_conns : Query<(&mut Connection,)>
) {
    for (mut conn,) in &mut q_conns {
        let mut buf = [0u8; 128];
        match (conn.read_stream.try_read(&mut buf)) {
            Ok(0) => {
                // Disconnected.
                conn.close();
            },
            Ok(count) => {
                conn.data_queue.reserve(count);
                for b1 in &buf[..count] {
                    if let Ok(b) = conn.packet_proc.secret_cipher.decrypt_u8(*b1) {
                        conn.data_queue.push_back(b);
                    } else {
                        error!("Failed to decrypt packet from peer {}", conn.peer_addr);
                        conn.kick("Bad packet: could not decrypt");
                    }
                }
            },
            Err(err) if (err.kind() == io::ErrorKind::WouldBlock) => { },
            Err(err) => {
                error!("Failed to read packet from peer {}", err);
                conn.kick(&format!("Bad packet: {err}"));
            }
        }
    }
}

// TODO: timeout logins

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
                conn.kick("Timed out");
            } }
        }
    }
    for packet::PacketReadEvent { entity, packet, .. } in er_packets.read() {
        if let packet::Packet::Play(C2SPlayPackets::ClientTickEnd(_)) = packet {} else {
            trace!("Received {} {:?}", entity, packet);
        }
        if let packet::Packet::Play(C2SPlayPackets::KeepAlive(KeepAliveC2SPlayPacket(id)))
            | packet::Packet::Config(C2SConfigPackets::KeepAlive(KeepAliveC2SConfigPacket(id))) = packet
            && let Ok((mut conn, mut keepalive,)) = q_conns.get_mut(*entity)
        {
            match (*keepalive) {
                ConnKeepalive::Sending { .. } => {
                    error!("Received unordered keepalive from peer {}", conn.peer_addr);
                    conn.kick("Unordered keepalive");
                },
                ConnKeepalive::Waiting { expected_id, .. } => {
                    if (*id == expected_id) {
                        trace!("Received keepalive {} from peer {}", id, conn.peer_addr);
                        *keepalive = ConnKeepalive::Sending { sending_at : Instant::now() + KEEPALIVE_INTERVAL };
                    } else {
                        error!("Received unordered keepalive from peer {}", conn.peer_addr);
                        conn.kick("Unordered keepalive");
                    }
                }
            }
        }
    }
}

pub(crate) fn close_conns(
    mut cmds    : Commands,
    mut q_conns : Query<(Entity, &mut Connection, Option<&mut Player>,)>,
    mut ew_left : EventWriter<PlayerLeft>
) {
    for (entity, mut conn, player,) in &mut q_conns {
        if (conn.closing) {
            if let Some(mut player) = player {
                info!("Player {} ({}) disconnected.", player.username(), player.uuid());
                ew_left.write(PlayerLeft {
                    uuid     : player.uuid,
                    username : mem::take(&mut player.username)
                });
            }
            debug!("Peer {} disconnected.", conn.peer_addr);
            cmds.entity(entity).despawn();
            ACTIVE_CONNS.fetch_sub(1, AtomicOrdering::Relaxed);
        }
        match (conn.close_receiver.try_recv()) {
            Ok(reason) => { conn.kick(&reason); },
            Err(mpsc::TryRecvError::Empty) => { },
            Err(mpsc::TryRecvError::Disconnected) => { conn.close(); }
        }
    }
}
