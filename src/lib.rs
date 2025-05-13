#![feature(
    let_chains,
    auto_traits,
    negative_impls
)]


use flywheelmc_common::prelude::*;
use protocol::packet::s2c::config::RegistryDataS2CConfigPacket;
use protocol::value::{ DimType, Identifier, Text, TextComponent, EntityType };
use protocol::registry::Registry;
pub use protocol::{ MINECRAFT_VERSION, PROTOCOL_VERSION };


mod conn;
pub use conn::packet::{ PacketReadEvent, Packet };

mod player;
pub use player::{ Player, PlayerJoined, PlayerLeft };

mod world;


pub struct FlywheelMcPlayersPlugin {
    pub listen_addrs       : SocketAddrs,
    pub motd               : Text,
    pub version            : Cow<'static, str>,
    pub favicon            : Cow<'static, str>,
    pub compress_threshold : usize,
    pub mojauth_enabled    : bool,
    pub server_id          : Cow<'static, str>,
    pub server_brand       : Cow<'static, str>,
    pub default_dim_id     : Identifier,
    pub default_dim_type   : DimType,
    pub max_view_distance  : NonZeroU8
}

impl Plugin for FlywheelMcPlayersPlugin {
    fn build(&self, app : &mut App) {
        app
            .add_event::<conn::packet::PacketReadEvent>()
            .add_event::<player::PlayerJoined>()
            .add_event::<player::PlayerLeft>()
            .insert_resource(ListenAddrs(self.listen_addrs.clone()))
            .insert_resource(ServerMotd(self.motd.clone()))
            .insert_resource(ServerVersion(self.version.clone()))
            .insert_resource(ServerFavicon(self.favicon.clone()))
            .insert_resource(CompressionThreshold(self.compress_threshold))
            .insert_resource(MojauthEnabled(self.mojauth_enabled))
            .insert_resource(ServerId(self.server_id.clone()))
            .insert_resource(ServerBrand(self.server_brand.clone()))
            .insert_resource(DefaultDim(self.default_dim_id.clone(), self.default_dim_type.clone()))
            .insert_resource(MaxViewDistance(self.max_view_distance))
            .insert_resource(LobbyYSections(self.default_dim_type.height / 16))
            .insert_resource(Registries::default())
            .insert_resource(RegistryPackets::new(&self.default_dim_id, &self.default_dim_type))
            .add_systems(Startup, start_listener)
            .add_systems(Update, conn::read_conn_streams)
            .add_systems(Update, conn::timeout_conns)
            .add_systems(Update, conn::shutdown_conns)
            .add_systems(Update, conn::handshake::handle_state)
            .add_systems(Update, conn::status::handle_state)
            .add_systems(Update, conn::login::handle_state)
            .add_systems(Update, conn::play::handle_state)
            .add_systems(Update, world::read_settings_updates)
            .add_systems(Update, world::update_chunk_view)
        ;
    }
}


#[derive(Resource)]
struct ListenAddrs(SocketAddrs);

#[derive(Resource)]
struct ServerMotd(Text);

#[derive(Resource)]
struct ServerVersion(Cow<'static, str>);

#[derive(Resource)]
struct ServerFavicon(Cow<'static, str>);

#[derive(Resource)]
struct CompressionThreshold(usize);

#[derive(Resource)]
struct MojauthEnabled(bool);

#[derive(Resource)]
struct ServerId(Cow<'static, str>);

#[derive(Resource)]
struct ServerBrand(Cow<'static, str>);

#[derive(Resource)]
struct DefaultDim(Identifier, DimType);

#[derive(Resource)]
struct MaxViewDistance(NonZeroU8);

#[derive(Resource)]
struct LobbyYSections(u32);

#[derive(Resource)]
struct Registries {
    entity_type : Registry<EntityType>
}
impl Default for Registries {
    fn default() -> Self { Self {
        entity_type : EntityType::vanilla_registry()
    } }
}

#[derive(Resource)]
struct RegistryPackets(Vec<RegistryDataS2CConfigPacket>);
impl RegistryPackets {
    fn new(default_dim_id : &Identifier, default_dim_type : &DimType) -> Self {
        use protocol::registry::Registry;
        use protocol::value::{
            DamageType,
            Biome,
            WolfVariant,
            PaintingVariant
        };
        Self(vec![
            DamageType::vanilla_registry().to_registry_data_packet(),
            Biome::vanilla_registry().to_registry_data_packet(),
            {
                let mut reg = Registry::<DimType>::new();
                reg.insert(default_dim_id.clone(), default_dim_type.clone());
                reg.to_registry_data_packet()
            },
            {
                let mut reg = Registry::<WolfVariant>::new();
                reg.insert(Identifier::vanilla_const("pale"), WolfVariant {
                    wild_texture  : Identifier::vanilla_const("wild_tex"),
                    tame_texture  : Identifier::vanilla_const("tame_tex"),
                    angry_texture : Identifier::vanilla_const("angry_tex"),
                    biomes        : Cow::Borrowed(&[])
                });
                reg.to_registry_data_packet()
            },
            {
                let mut reg = Registry::<PaintingVariant>::new();
                reg.insert(Identifier::vanilla_const("empty"), PaintingVariant {
                    asset_id : Identifier::vanilla_const("empty"),
                    width    : 1,
                    height   : 1,
                    title    : TextComponent::of_literal("Empty"),
                    author   : TextComponent::of_literal("Empty")
                });
                reg.to_registry_data_packet()
            }
        ])
    }
}


fn start_listener(
    mut cmds           : Commands,
        r_listen_addrs : Res<ListenAddrs>
) {
    let listen_addrs = r_listen_addrs.0.clone();
    cmds.spawn_task(async move || {
        let _ = handle_err(conn::run_listener(listen_addrs).await);
        Ok(())
    });
}
