use crate::{
    CompressionThreshold,
    MojauthEnabled,
    ServerId,
    ServerBrand,
    DefaultDim,
    MaxViewDistance,
    Registries,
    RegistryPackets
};
use crate::player::{ Player, PlayerJoined };
use crate::conn::ConnStream;
use crate::conn::play::ConnStatePlay;
use flywheelmc_common::prelude::*;
use voxidian_protocol::value::{
    Identifier,
    Angle
};
use voxidian_protocol::packet::PacketWriter;
use voxidian_protocol::packet::c2s::login::{
    C2SLoginPackets,
    HelloC2SLoginPacket
};
use voxidian_protocol::packet::c2s::config::C2SConfigPackets;
use voxidian_protocol::packet::s2c::login::{
    LoginCompressionS2CLoginPacket,
    HelloS2CLoginPacket,
    LoginFinishedS2CLoginPacket
};
use voxidian_protocol::packet::s2c::config::{
    CustomPayloadS2CConfigPacket,
    SelectKnownPacksS2CConfigPacket,
    FinishConfigurationS2CConfigPacket
};
use voxidian_protocol::packet::s2c::play::{
    LoginS2CPlayPacket,
    AddEntityS2CPlayPacket,
    Gamemode
};
use voxidian_protocol::packet::processing::{
    CompressionMode,
    generate_key_pair,
    PrivateKey,
    PublicKey,
    SecretCipher,
};
use voxidian_protocol::registry::RegEntry;
use voxidian_protocol::mojang::auth_verify::{
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


pub(crate) fn handle_state(
    mut cmds           : Commands,
    mut q_conn_streams : Query<(Entity, &mut ConnStream, &mut ConnStateLogin,)>,
        r_threshold    : Res<CompressionThreshold>,
        r_mojauth      : Res<MojauthEnabled>,
        r_server_id    : Res<ServerId>,
        r_server_brand : Res<ServerBrand>,
        r_default_dim  : Res<DefaultDim>,
        r_view_dist    : Res<MaxViewDistance>,
        r_regs         : Res<Registries>,
        r_reg_packets  : Res<RegistryPackets>,
    mut ew_joined      : EventWriter<PlayerJoined>
) {
    for (entity, mut conn_stream, mut state) in &mut q_conn_streams {
        match (&mut*state) {


            ConnStateLogin::WaitingForHello => {
                if let Some(C2SLoginPackets::Hello(HelloC2SLoginPacket { username, .. })) = conn_stream.read_packet() {
                    // Set compression.
                    let threshold = r_threshold.0;
                    conn_stream.send_packet(&mut cmds, LoginCompressionS2CLoginPacket {
                        threshold : threshold.into()
                    });
                    conn_stream.packet_proc.compression = CompressionMode::ZLib { threshold };

                    // Share keys.
                    let (private_key, public_key) = generate_key_pair::<1024>();
                    let verify_token              = array::from_fn::<_, 4, _>(|_| rand::random::<u8>());
                    conn_stream.send_packet(&mut cmds, HelloS2CLoginPacket {
                        server_id    : r_server_id.0.clone(),
                        public_key   : public_key.der_bytes().into(),
                        verify_token : verify_token.to_vec().into(),
                        should_auth  : r_mojauth.0
                    });

                    *state = ConnStateLogin::ExchangingKeys {
                        username,
                        private_key, public_key,
                        verify_token
                    };
                }
            },


            ConnStateLogin::ExchangingKeys { username, private_key, public_key, verify_token } => {
                if let Some(C2SLoginPackets::Key(packet)) = conn_stream.read_packet() {
                    
                    // Check the verify token.
                    if let Ok(decrypted_verify_token) = private_key.decrypt(packet.verify_token.as_slice())
                        && (decrypted_verify_token == verify_token.as_slice()) {
                    } else {
                        // TODO: Log warning
                        conn_stream.shutdown.store(true, AtomicOrdering::Relaxed);
                        continue;
                    }
                    
                    // Decrypt the secret key and construct a cipher.
                    let Ok(secret_key) = private_key.decrypt(packet.secret_key.as_slice()) else {
                        // TODO: Log warning
                        conn_stream.shutdown.store(true, AtomicOrdering::Relaxed);
                        continue;
                    };
                    conn_stream.packet_proc.secret_cipher = SecretCipher::from_key_bytes(&secret_key);

                    // Check mojang authentication
                    *state = if (r_mojauth.0) {
                        let username          = mem::replace(username, String::new());
                        let server_id         = r_server_id.0.clone();
                        let secret_cipher_key = conn_stream.packet_proc.secret_cipher.key().unwrap().to_vec();
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
                        let mojauth = MojAuth::offline(mem::replace(username, String::new()));
                        ConnStateLogin::HandleMojauth { mojauth }
                    };
                }
            },


            ConnStateLogin::CheckingMojauth { fut } => {
                if let Poll::Ready(result) = fut.poll() {
                    match (result) {
                        Ok(mojauth) => {
                            *state = ConnStateLogin::HandleMojauth { mojauth }
                        },
                        Err(_) => {
                            // TODO: Log warning
                            conn_stream.shutdown.store(true, AtomicOrdering::Relaxed);
                        }
                    }
                }
            },


            ConnStateLogin::HandleMojauth { mojauth } => {
                conn_stream.send_packet(&mut cmds, LoginFinishedS2CLoginPacket {
                    uuid     : mojauth.uuid,
                    username : mojauth.name.clone(),
                    props    : default()
                });
                // TODO: Check infractions
                // TODO: Check already logged in network (max 5?)
                *state = ConnStateLogin::FinishingLogin {
                    uuid     : mojauth.uuid,
                    username : mem::replace(&mut mojauth.name, String::new()),
                    props    : mem::replace(&mut mojauth.props, Vec::new())
                };
            },


            ConnStateLogin::FinishingLogin { uuid, username, props } => {
                if let Some(C2SLoginPackets::LoginAcknowledged(_)) = conn_stream.read_packet() {

                    // Send server brand
                    conn_stream.send_packet(&mut cmds, CustomPayloadS2CConfigPacket {
                        channel : Identifier::vanilla_const("brand"),
                        data    : {
                            let mut buf = PacketWriter::new();
                            buf.encode_write(&r_server_brand.0).unwrap();
                            buf.into_inner().into()
                        }
                    });

                    // Send registries
                    conn_stream.send_packet(&mut cmds, SelectKnownPacksS2CConfigPacket::default());
                    for packet in &r_reg_packets.0 {
                        conn_stream.send_packet(&mut cmds, packet);
                    }

                    // Complete config
                    conn_stream.send_packet(&mut cmds, FinishConfigurationS2CConfigPacket);
                    *state = ConnStateLogin::FinishingConfig {
                        uuid     : *uuid,
                        username : mem::replace(username, String::new()),
                        props    : mem::replace(props, Vec::new())
                    };
                }
            },


            ConnStateLogin::FinishingConfig { uuid, username, props } => {
                if let Some(C2SConfigPackets::FinishConfiguration(_)) = conn_stream.read_packet() {
                    // TODO: Log info

                    cmds.entity(entity)
                        .remove::<ConnStateLogin>()
                        .insert((
                            ConnStatePlay,
                            Player {
                                uuid     : *uuid,
                                username : mem::replace(username, String::new()),
                                props    : mem::replace(props, Vec::new())
                            }
                        ));
                    ew_joined.write(PlayerJoined(entity));
                    drop((username, props,));

                    conn_stream.send_packet(&mut cmds, LoginS2CPlayPacket {
                        entity               : 1.into(),
                        hardcore             : false,
                        dims                 : vec![ r_default_dim.0.clone() ].into(),
                        max_players          : 0.into(),
                        view_dist            : r_view_dist.0.into(),
                        sim_dist             : r_view_dist.0.into(),
                        reduced_debug        : false,
                        respawn_screen       : false,
                        limited_crafting     : true,
                        dim                  : unsafe { RegEntry::new_unchecked(0) },
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
                    });
                    conn_stream.send_packet(&mut cmds, AddEntityS2CPlayPacket {
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
                    });
                }
            },


        }
    }
}
