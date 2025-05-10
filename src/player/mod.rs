use flywheelmc_common::prelude::*;
use protocol::mojang::auth_verify::MojAuthProperty;


mod world;


#[derive(Component)]
pub struct Player {
    pub(crate) uuid     : Uuid,
    pub(crate) username : String,
    pub(crate) props    : Vec<MojAuthProperty>
}
impl Player {

    pub fn uuid(&self) -> Uuid { self.uuid }

    pub fn username(&self) -> &str { &self.username }

}


#[derive(Event)]
pub struct PlayerJoined(pub Entity);

#[derive(Event)]
pub struct PlayerLeft {
    pub uuid     : Uuid,
    pub username : String
}
