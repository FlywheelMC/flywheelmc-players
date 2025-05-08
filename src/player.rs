use flywheelmc_common::prelude::*;
use voxidian_protocol::mojang::auth_verify::MojAuthProperty;


#[derive(Component)]
pub struct Player {
    pub(crate) uuid     : Uuid,
    pub(crate) username : String,
    pub(crate) props    : Vec<MojAuthProperty>
}


#[derive(Event)]
pub struct PlayerJoined(pub Entity);

#[derive(Event)]
pub struct PlayerLeft {
    pub uuid     : Uuid,
    pub username : String
}
