use crate::{
    CompressionThreshold,
    RejectNewConns,
    MaxConnCount,
    MojauthEnabled,
    ServerId,
    ServerBrand,
    DefaultDim,
    MaxViewDistance,
    Registries,
    RegistryPackets
};
use crate::player::{ Player, PlayerJoined };
use crate::conn::{ Connection, ConnKeepalive, RealStage, KEEPALIVE_INTERVAL, ACTIVE_CONNS };
use crate::conn::packet::{ PacketReadEvent, NextStage };
use crate::conn::play::ConnStatePlay;
use crate::world;
use flywheelmc_common::prelude::*;
use protocol::value::{
    Identifier,
    Angle
};
use protocol::packet::PacketWriter;
use protocol::packet::c2s::login::{
    C2SLoginPackets,
    HelloC2SLoginPacket
};
use protocol::packet::c2s::config::C2SConfigPackets;
use protocol::packet::s2c::login::{
    LoginCompressionS2CLoginPacket,
    HelloS2CLoginPacket,
    LoginFinishedS2CLoginPacket
};
use protocol::packet::s2c::config::{
    CustomPayloadS2CConfigPacket,
    SelectKnownPacksS2CConfigPacket,
    FinishConfigurationS2CConfigPacket
};
use protocol::packet::s2c::play::{
    LoginS2CPlayPacket,
    AddEntityS2CPlayPacket,
    PlayerInfoUpdateS2CPlayPacket,
    GameEventS2CPlayPacket,
    Gamemode,
    PlayerActionEntry,
    GameEvent
};
use protocol::packet::processing::{
    CompressionMode,
    generate_key_pair,
    PrivateKey,
    PublicKey,
    SecretCipher,
};
use protocol::registry::RegEntry;
use protocol::mojang::auth_verify::{
    MojAuth,
    MojAuthProperty,
    MojAuthError
};


#[derive(Component)]
pub(crate) enum ConnStateLogin {
    WaitingForHello,
    ExchangingKeys {
        username     : String,
        private_key  : PrivateKey,
        public_key   : PublicKey,
        verify_token : [u8; 4]
    },
    CheckingMojauth {
        fut : ManuallyPoll<'static, Result<MojAuth, MojAuthError>>
    },
    HandleMojauth {
        mojauth : MojAuth
    },
    FinishingLogin {
        uuid     : Uuid,
        username : String,
        props    : Vec<MojAuthProperty>
    },
    FinishingConfig {
        uuid     : Uuid,
        username : String,
        props    : Vec<MojAuthProperty>
    }
}


#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_state(
    mut cmds           : Commands,
    mut q_conns        : Query<(Entity, &mut Connection, &mut ConnStateLogin,)>,
        r_reject       : Option<Res<RejectNewConns>>,
        r_max_conns    : Option<Res<MaxConnCount>>,
        r_threshold    : Res<CompressionThreshold>,
        r_mojauth      : Res<MojauthEnabled>,
        r_server_id    : Res<ServerId>,
        r_server_brand : Res<ServerBrand>,
        r_default_dim  : Res<DefaultDim>,
        r_view_dist    : Res<MaxViewDistance>,
        r_regs         : Res<Registries>,
        r_reg_packets  : Res<RegistryPackets>,
    mut ew_joined      : EventWriter<PlayerJoined>,
    mut ew_packet      : EventWriter<PacketReadEvent>
) {
    for (entity, mut conn, mut state) in &mut q_conns {
        if (conn.closing) { continue; }
        match (&mut*state) {


            ConnStateLogin::WaitingForHello => {
                if let Some(C2SLoginPackets::Hello(HelloC2SLoginPacket { username, .. })) = conn.read_packet() {
                    debug!("Peer {} is logging in...", conn.peer_addr);

                    if let Some(reject) = &r_reject {
                        conn.kick(&reject.0);
                    }
                    if let Some(max_conns) = &r_max_conns {
                        if (ACTIVE_CONNS.load(AtomicOrdering::Relaxed) > max_conns.0) {
                            conn.kick("Server is full");
                            continue;
                        }
                    }

                    // Set compression.
                    let threshold = r_threshold.0;
                    if (unsafe { conn.send_packet_noset(LoginCompressionS2CLoginPacket {
                        threshold : threshold.into()
                    }) }.is_err()) { continue; }
                    conn.packet_proc.compression = CompressionMode::ZLib { threshold };

                    trace!("Exchanging public-key with peer {}...", conn.peer_addr);
                    // Share keys.
                    let (private_key, public_key) = generate_key_pair::<1024>();
                    let verify_token              = array::from_fn::<_, 4, _>(|_| random::<u8>());
                    if (unsafe { conn.send_packet_noset(HelloS2CLoginPacket {
                        server_id    : r_server_id.0.to_string(),
                        public_key   : public_key.der_bytes().into(),
                        verify_token : verify_token.to_vec().into(),
                        should_auth  : r_mojauth.0
                    }) }.is_err()) { continue; }

                    *state = ConnStateLogin::ExchangingKeys {
                        username,
                        private_key, public_key,
                        verify_token
                    };
                }
            },


            ConnStateLogin::ExchangingKeys { username, private_key, public_key, verify_token } => {
                if let Some(C2SLoginPackets::Key(packet)) = conn.read_packet() {

                    // Check the verify token.
                    if let Ok(decrypted_verify_token) = private_key.decrypt(packet.verify_token.as_slice())
                        && (decrypted_verify_token == verify_token.as_slice()) {
                    } else {
                        error!("Failed to verify keys from peer {}", conn.peer_addr);
                        conn.kick("Key exchange verification failed");
                        continue;
                    }

                    // Decrypt the secret key and construct a cipher.
                    let Ok(secret_key) = private_key.decrypt(packet.secret_key.as_slice()) else {
                        error!("Failed to decrypt keys from peer {}", conn.peer_addr);
                        conn.kick("Failed to decrypt secret key");
                        continue;
                    };
                    conn.packet_proc.secret_cipher = SecretCipher::from_key_bytes(&secret_key);
                    trace!("Got private-key from peer {}", conn.peer_addr);

                    trace!("Validating mojauth of peer {}...", conn.peer_addr);
                    // Check mojang authentication
                    *state = if (r_mojauth.0) {
                        let username          = mem::take(username);
                        let server_id         = r_server_id.0.clone();
                        let secret_cipher_key = conn.packet_proc.secret_cipher.key().unwrap().to_vec();
                        let public_key        = public_key.clone();
                        ConnStateLogin::CheckingMojauth { fut : ManuallyPoll::new(async move {
                            MojAuth::start(
                                None,
                                username,
                                server_id,
                                &secret_cipher_key,
                                &public_key
                            ).await
                        }) }
                    } else {
                        let mojauth = MojAuth::offline(mem::take(username));
                        ConnStateLogin::HandleMojauth { mojauth }
                    };
                }
            },


            ConnStateLogin::CheckingMojauth { fut } => {
                if let Poll::Ready(result) = fut.poll() {
                    match (result) {
                        Ok(mojauth) => {
                            trace!("Peer {} authenticated as {} ({})", conn.peer_addr, mojauth.name, mojauth.uuid);
                            *state = ConnStateLogin::HandleMojauth { mojauth }
                        },
                        Err(err) => {
                            error!("Failed to authenticate peer {}: {err}", conn.peer_addr);
                            conn.kick(&format!("Authentication failed: {err}"));
                        }
                    }
                }
            },


            ConnStateLogin::HandleMojauth { mojauth } => {
                // TODO: Check infractions
                // TODO: Check already logged in network (max 5?)
                if (unsafe { conn.send_packet_noset(LoginFinishedS2CLoginPacket {
                    uuid     : mojauth.uuid,
                    username : mojauth.name.clone(),
                    props    : default()
                }) }.is_err()) { continue; }
                *state = ConnStateLogin::FinishingLogin {
                    uuid     : mojauth.uuid,
                    username : mem::take(&mut mojauth.name),
                    props    : mem::take(&mut mojauth.props)
                };
            },


            ConnStateLogin::FinishingLogin { uuid, username, props } => {
                if let Some(C2SLoginPackets::LoginAcknowledged(_)) = conn.read_packet() {
                    conn.real_stage = RealStage::Config;

                    if (conn.stage_sender.send(NextStage::Config).is_err()) {
                        error!("Failed to switch peer {} to config stage", conn.peer_addr);
                        conn.kick("Could not switch to config stage");
                        continue;
                    }

                    // Send server brand
                    if (unsafe { conn.send_packet_noset(CustomPayloadS2CConfigPacket {
                        channel : Identifier::vanilla_const("brand"),
                        data    : {
                            let mut buf = PacketWriter::new();
                            buf.encode_write(&r_server_brand.0).unwrap();
                            buf.into_inner().into()
                        }
                    }) }.is_err()) { continue; }

                    // Send registries
                    if (unsafe { conn.send_packet_noset(
                        SelectKnownPacksS2CConfigPacket::default()
                    ) }.is_err()) { continue; }
                    for packet in &r_reg_packets.0 {
                        if (unsafe { conn.send_packet_noset(packet) }.is_err()) { continue; }
                    }

                    // Complete config
                    info!("Player {} ({}) joined.", username, uuid);
                    cmds.entity(entity).insert((
                        Player {
                            uuid     : *uuid,
                            username : username.clone(),
                            props    : props.clone()
                        },
                        world::ChunkCentre(Dirty::new_dirty(Vec2::<i32>::ZERO)),
                        world::ViewDistance(Ordered::new(NonZeroU8::MIN))
                    ));

                    if (unsafe { conn.send_packet_noset(FinishConfigurationS2CConfigPacket) }.is_err()) {
                        continue;
                    }
                    *state = ConnStateLogin::FinishingConfig {
                        uuid     : *uuid,
                        username : mem::take(username),
                        props    : mem::take(props)
                    }

                }
            },


            ConnStateLogin::FinishingConfig { uuid, username, props } => {
                if let Some(packet) = conn.read_packet() {
                    if let C2SConfigPackets::FinishConfiguration(_) = packet {
                        conn.real_stage = RealStage::Play;

                        cmds.entity(entity)
                            .remove::<ConnStateLogin>()
                            .insert((
                                ConnStatePlay {
                                    stage : NextStage::Play
                                },
                                ConnKeepalive::Sending { sending_at : Instant::now() + KEEPALIVE_INTERVAL }
                            ));
                        ew_joined.write(PlayerJoined {
                            entity,
                            _private : ()
                        });

                        if (conn.stage_sender.send(NextStage::Play).is_err()) {
                            error!("Failed to switch peer {} to play stage", conn.peer_addr);
                            conn.kick("Could not switch to play stage");
                            continue;
                        }

                        let view_dist = (r_view_dist.0.get() as usize).into();
                        if (unsafe { conn.send_packet_noset(LoginS2CPlayPacket {
                            entity               : 1,
                            hardcore             : false,
                            dims                 : vec![ r_default_dim.0.clone() ].into(),
                            max_players          : 0.into(),
                            view_dist,
                            sim_dist             : view_dist,
                            reduced_debug        : false,
                            respawn_screen       : false,
                            limited_crafting     : true,
                            dim                  : RegEntry::new_unchecked(0),
                            dim_name             : r_default_dim.0.clone(),
                            seed                 : 0,
                            gamemode             : Gamemode::Adventure,
                            old_gamemode         : Gamemode::None,
                            is_debug             : false,
                            is_flat              : false,
                            death_loc            : None,
                            portal_cooldown      : 0.into(),
                            sea_level            : 0.into(),
                            enforce_chat_reports : false
                        }) }.is_err()) { continue; }

                        if (unsafe { conn.send_packet_noset(PlayerInfoUpdateS2CPlayPacket {
                            actions : vec![(*uuid, vec![
                                PlayerActionEntry::AddPlayer {
                                    name  : username.clone(),
                                    props : mem::take(props)
                                        .into_iter()
                                        .map(|prop| prop.into())
                                        .collect::<Vec<_>>()
                                        .into()
                                }
                            ],)],
                        }) }.is_err()) { continue; }

                        if (unsafe { conn.send_packet_noset(AddEntityS2CPlayPacket {
                            id       : 1.into(),
                            uuid     : *uuid,
                            kind     : r_regs.entity_type.get_entry(&Identifier::vanilla_const("player")).unwrap(),
                            x        : 0.0,
                            y        : 0.0,
                            z        : 0.0,
                            pitch    : Angle::of_frac(0.0),
                            yaw      : Angle::of_frac(0.0),
                            head_yaw : Angle::of_frac(0.0),
                            data     : 0.into(),
                            vel_x    : 0,
                            vel_y    : 0,
                            vel_z    : 0
                        }) }.is_err()) { continue; }

                        if (unsafe { conn.send_packet_noset(GameEventS2CPlayPacket {
                            event : GameEvent::WaitForChunks,
                            value : 0.0
                        }) }.is_err()) { continue; }

                    } else {
                        ew_packet.write(PacketReadEvent {
                            entity,
                            packet : packet.into(),
                            index  : conn.packet_index.increment()
                        });
                    }
                }
            },


        }
    }
}
