use flywheelmc_common::prelude::*;
use protocol::value::Text;
use protocol::mojang::auth_verify::MojAuthProperty;


pub mod comms;


#[derive(Component)]
pub struct Player {
    pub(crate) uuid     : Uuid,
    pub(crate) username : String,
    #[expect(dead_code)]
    pub(crate) props    : Vec<MojAuthProperty>
}
impl Player {

    pub fn uuid(&self) -> Uuid { self.uuid }

    pub fn username(&self) -> &str { &self.username }

}


#[derive(Event)]
#[non_exhaustive]
pub struct PlayerJoined {
    pub entity : Entity
}

#[derive(Event)]
#[non_exhaustive]
pub struct PlayerLeft {
    pub uuid     : Uuid,
    pub username : String
}


#[derive(Event)]
pub struct KickPlayer {
    pub entity  : Entity,
    pub message : Text
}
